//! Making an app through the real `local-app-builder mcp` process, over real pipes, with a client that answers the server's
//! questions the way a person would.
//!
//! The client here is a legacy (2025-11-25) session that declares elicitation. The flow is the one a model drives:
//! create the empty app, write a plan, call prepare; the server asks the client whether to approve the plan, and only
//! the client's explicit yes lets it go on.
//!
//! `the_whole_flow_with_the_real_toolchain` needs the network and the pinned toolchain and runs only on request:
//! `LOCAL_APP_REAL_TOOLCHAIN=1 LOCAL_APP_REAL_TOOLCHAIN_DATA=<dir> cargo test -p local-app-builder-cli --test create_flow_test`.

use local_app_builder_service::plan_approval::parse_authoring_block;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The plan the `local-app-builder-create` skill shows a model. The tests use it as it stands, so the example cannot rot.
const CREATE_SKILL: &str =
    include_str!("../../../plugins/local-app-builder/skills/local-app-builder-create/SKILL.md");

fn skill_plan() -> String {
    let start = CREATE_SKILL
        .find("<!-- plan-example:start -->")
        .expect("start marker");
    let end = CREATE_SKILL
        .find("<!-- plan-example:end -->")
        .expect("end marker");
    let block = &CREATE_SKILL[start..end];
    let lines: Vec<&str> = block.lines().collect();
    // Drop the marker line and the four-backtick fence around the example.
    let open = lines
        .iter()
        .position(|l| l.starts_with("````"))
        .expect("outer fence");
    let close = lines
        .iter()
        .rposition(|l| l.starts_with("````"))
        .expect("outer fence end");
    lines[open + 1..close].join("\n") + "\n"
}

/// What the person does when asked.
#[derive(Clone, Copy, PartialEq)]
enum Person {
    Approves,
    Declines,
}

struct Client {
    child: Child,
    stdin: std::process::ChildStdin,
    lines: mpsc::Receiver<String>,
    next_id: i64,
    /// The questions the server put to the person, in order.
    questions: Vec<String>,
}

impl Client {
    fn start(data_root: &Path, capabilities: Value) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_local-app-builder"))
            .args(["mcp", "--data-root"])
            .arg(data_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn local-app-builder mcp");
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let stdin = child.stdin.take().unwrap();
        let mut client = Self {
            child,
            stdin,
            lines,
            next_id: 0,
            questions: Vec::new(),
        };
        let init = client.request("initialize", json!({"protocolVersion": "2025-11-25", "capabilities": capabilities, "clientInfo": {"name": "test", "version": "0"}}), Person::Declines);
        assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
        client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        client
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Send a request and return its response, answering any question the server asks meanwhile.
    fn request(&mut self, method: &str, params: Value, person: Person) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(900))
                .expect("a message from the server");
            let message: Value =
                serde_json::from_str(&line).unwrap_or_else(|_| panic!("not a message: {line}"));
            if message["method"] == "elicitation/create" {
                self.questions.push(
                    message["params"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                );
                let approve = person == Person::Approves;
                let answer = json!({"jsonrpc": "2.0", "id": message["id"], "result": {"action": if approve { "accept" } else { "decline" }, "content": {"approve": approve}}});
                self.send(&answer);
                continue;
            }
            if message["id"] == id {
                return message;
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value, person: Person) -> (bool, String, Value) {
        let reply = self.request(
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
            person,
        );
        let result = &reply["result"];
        assert!(result.is_object(), "{tool}: {reply}");
        let text = result["content"]
            .as_array()
            .map(|c| {
                c.iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        (
            result["isError"].as_bool().unwrap_or(false),
            text,
            result["structuredContent"].clone(),
        )
    }

    fn finish(mut self) {
        drop(self.stdin);
        let _ = self.child.wait();
    }
}

/// One plain HTTP/1.1 GET to a loopback `url`: the status line, the header block and the body.
fn http_get(url: &str) -> std::io::Result<(String, String, Vec<u8>)> {
    use std::io::Read;
    let address = url.strip_prefix("http://").expect("an http url");
    let mut stream = std::net::TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header block");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let (status, headers) = head.split_once("\r\n").unwrap_or((&head, ""));
    Ok((
        status.to_string(),
        headers.to_string(),
        raw[split + 4..].to_vec(),
    ))
}

fn app_id(structured: &Value) -> String {
    structured["app"]["id"]
        .as_str()
        .or_else(|| structured["id"].as_str())
        .or_else(|| structured["app_id"].as_str())
        .unwrap_or_else(|| panic!("no app id in {structured}"))
        .to_string()
}

fn write_plan(dir: &Path, template: &str) -> std::path::PathBuf {
    assert_eq!(
        template, "react-dom-r4",
        "the skill's example is the react-dom-r4 plan"
    );
    let path = dir.join("plan.md");
    std::fs::write(&path, skill_plan()).unwrap();
    path
}

#[test]
fn the_plan_the_skill_shows_is_one_the_server_accepts() {
    let plan = skill_plan();
    assert!(plan.starts_with("# Water tracker"), "{plan}");
    let block = parse_authoring_block(&plan)
        .expect("the example parses")
        .expect("the example carries an authoring block");
    drop(block);
    // And it names a template the catalog has.
    let catalog = include_str!("../../plugins/lingxi-local-app/assets/templates/catalog.json");
    assert!(catalog.contains("\"react-dom-r4\""));
}

#[test]
fn a_person_who_declines_the_plan_gets_no_app_and_the_model_is_told_so() {
    let data = tempfile::tempdir().unwrap();
    let plans = tempfile::tempdir().unwrap();
    let mut client = Client::start(data.path(), json!({"elicitation": {}}));
    let (_, _, created) = client.call(
        "LocalAppCreate",
        json!({"brief": "Track water"}),
        Person::Declines,
    );
    let id = app_id(&created);
    let plan = write_plan(plans.path(), "react-dom-r4");
    let (is_error, said, _) = client.call(
        "LocalAppPrepare",
        json!({"app_id": id, "plan_path": plan}),
        Person::Declines,
    );
    assert!(is_error && said.starts_with("plan_not_approved:"), "{said}");
    assert_eq!(client.questions.len(), 1);
    assert!(
        client.questions[0].contains("Water Tracker")
            && client.questions[0].contains("authoring-spec"),
        "{}",
        client.questions[0]
    );
    // Nothing was prepared: the app is still an empty shell, which the other tools refuse.
    let (is_error, said, _) = client.call("LocalAppBuild", json!({"app_id": id}), Person::Declines);
    assert!(is_error, "{said}");
    client.finish();
}

#[test]
fn a_client_that_cannot_be_asked_cannot_approve_and_is_never_sent_a_question() {
    let data = tempfile::tempdir().unwrap();
    let plans = tempfile::tempdir().unwrap();
    let mut client = Client::start(data.path(), json!({}));
    let (_, _, created) = client.call(
        "LocalAppCreate",
        json!({"brief": "Track water"}),
        Person::Approves,
    );
    let id = app_id(&created);
    let plan = write_plan(plans.path(), "react-dom-r4");
    // The person "approves" in this client, but it declared no elicitation, so nothing was asked and nothing happens.
    let (is_error, said, _) = client.call(
        "LocalAppPrepare",
        json!({"app_id": id, "plan_path": plan}),
        Person::Approves,
    );
    assert!(
        is_error
            && said.starts_with("approval_unavailable:")
            && said.contains("did not declare support for elicitation"),
        "{said}"
    );
    assert!(client.questions.is_empty(), "{:?}", client.questions);
    client.finish();
}

#[cfg(target_os = "macos")]
#[test]
fn an_approved_plan_without_the_toolchain_fails_closed_and_says_how_to_get_it() {
    let data = tempfile::tempdir().unwrap();
    let plans = tempfile::tempdir().unwrap();
    let mut client = Client::start(data.path(), json!({"elicitation": {}}));
    let (_, _, created) = client.call(
        "LocalAppCreate",
        json!({"brief": "Track water"}),
        Person::Approves,
    );
    let id = app_id(&created);
    let plan = write_plan(plans.path(), "react-dom-r4");
    let (is_error, said, _) = client.call(
        "LocalAppPrepare",
        json!({"app_id": id, "plan_path": plan}),
        Person::Approves,
    );
    assert_eq!(client.questions.len(), 1, "the person was asked once");
    assert!(is_error, "{said}");
    assert!(
        said.contains("toolchain_not_installed")
            && said.contains("local-app-builder toolchain install"),
        "{said}"
    );
    client.finish();
}

/// Only a Mac has a pinned toolchain, so elsewhere there is no build executor at all: the approved plan still fails
/// closed, and says the runtime is missing rather than how to install a toolchain that does not exist for this machine.
#[cfg(not(target_os = "macos"))]
#[test]
fn an_approved_plan_on_a_machine_without_a_pinned_toolchain_fails_closed() {
    let data = tempfile::tempdir().unwrap();
    let plans = tempfile::tempdir().unwrap();
    let mut client = Client::start(data.path(), json!({"elicitation": {}}));
    let (_, _, created) = client.call(
        "LocalAppCreate",
        json!({"brief": "Track water"}),
        Person::Approves,
    );
    let id = app_id(&created);
    let plan = write_plan(plans.path(), "react-dom-r4");
    let (is_error, said, _) = client.call(
        "LocalAppPrepare",
        json!({"app_id": id, "plan_path": plan}),
        Person::Approves,
    );
    assert_eq!(client.questions.len(), 1, "the person was asked once");
    assert!(
        is_error && said.contains("Node runtime is unavailable"),
        "{said}"
    );
    client.finish();
}

#[test]
fn the_whole_flow_with_the_real_toolchain() {
    if std::env::var_os("LOCAL_APP_REAL_TOOLCHAIN").is_none() {
        eprintln!("skipped: set LOCAL_APP_REAL_TOOLCHAIN=1 (and LOCAL_APP_REAL_TOOLCHAIN_DATA=<dir>) to run it");
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let data = std::env::var_os("LOCAL_APP_REAL_TOOLCHAIN_DATA")
        .map_or_else(|| scratch.path().join("data"), std::path::PathBuf::from);
    std::fs::create_dir_all(&data).unwrap();
    let plans = tempfile::tempdir().unwrap();

    // The toolchain, installed by the command a person would run.
    let installed = Command::new(env!("CARGO_BIN_EXE_local-app-builder"))
        .args(["toolchain", "install", "--data-root"])
        .arg(&data)
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );

    let mut client = Client::start(&data, json!({"elicitation": {}}));
    let (_, _, catalog) = client.call("LocalAppTemplateCatalog", json!({}), Person::Approves);
    assert!(catalog.to_string().contains("react-dom-r4"), "{catalog}");
    let (is_error, said, created) = client.call(
        "LocalAppCreate",
        json!({"brief": "Log glasses of water"}),
        Person::Approves,
    );
    assert!(!is_error, "{said}");
    let id = app_id(&created);
    let plan = write_plan(plans.path(), "react-dom-r4");
    let (is_error, said, prepared) = client.call(
        "LocalAppPrepare",
        json!({"app_id": id, "plan_path": plan}),
        Person::Approves,
    );
    assert!(!is_error, "prepare: {said}");
    assert_eq!(client.questions.len(), 1);
    eprintln!("prepared: {prepared}");
    let (is_error, said, _) = client.call(
        "LocalAppInstallDeps",
        json!({"app_id": id, "wait": true}),
        Person::Approves,
    );
    assert!(!is_error, "install: {said}");
    let mut build_args = json!({"app_id": id});
    for (from, to) in [
        ("execution_id", "workflow_run_id"),
        ("workflow_run_id", "workflow_run_id"),
        ("contract_handle", "contract_handle"),
    ] {
        if let Some(value) = prepared.get(from).filter(|v| v.is_string()) {
            build_args[to] = value.clone();
        }
    }
    let (is_error, said, _) = client.call("LocalAppBuild", build_args, Person::Approves);
    assert!(!is_error, "build: {said}");
    let (is_error, said, got) = client.call("LocalAppGet", json!({"app_id": id}), Person::Approves);
    assert!(!is_error, "{said}");
    eprintln!("app after build: {got}");

    // Run it: the build is served on a loopback address, and stopping it closes the address.
    let (is_error, said, started) = client.call(
        "LocalAppRuntime",
        json!({"app_id": id, "action": "start"}),
        Person::Approves,
    );
    assert!(!is_error, "start: {said}");
    let url = started["url"]
        .as_str()
        .unwrap_or_else(|| panic!("no url in {started}"))
        .to_string();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    let (status, headers, body) = http_get(&url).expect("the served page");
    assert!(status.contains("200"), "{status}");
    assert!(
        headers.to_lowercase().contains("content-security-policy"),
        "{headers}"
    );
    assert!(
        String::from_utf8_lossy(&body).contains("<div id=\"root\""),
        "not the app's page"
    );
    let (is_error, said, _) = client.call(
        "LocalAppRuntime",
        json!({"app_id": id, "action": "stop"}),
        Person::Approves,
    );
    assert!(!is_error, "stop: {said}");
    assert!(
        http_get(&url).is_err(),
        "the address is still served after stop"
    );

    // A dependency change: the person says no first, and nothing is issued.
    let change = json!([{"kind": "add", "package": "dayjs", "version": "1.11.13"}]);
    let asked = client.questions.len();
    let (is_error, said, _) = client.call(
        "LocalAppConfirmDependencyChange",
        json!({"app_id": id, "changes": change}),
        Person::Declines,
    );
    assert!(is_error && said.contains("denied"), "{said}");
    assert_eq!(client.questions.len(), asked + 1);
    assert!(
        client.questions[asked].contains("add dayjs@1.11.13"),
        "{}",
        client.questions[asked]
    );
    // Then yes: a receipt, and applying it puts the package in the app.
    let (is_error, said, confirmed) = client.call(
        "LocalAppConfirmDependencyChange",
        json!({"app_id": id, "changes": change}),
        Person::Approves,
    );
    assert!(!is_error, "confirm: {said}");
    let receipt = confirmed["receipt"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no receipt in {confirmed}"))
        .to_string();
    let (is_error, said, _) = client.call(
        "LocalAppUpdateDependencies",
        json!({"app_id": id, "receipt_id": receipt}),
        Person::Approves,
    );
    assert!(!is_error, "update: {said}");
    let package_json =
        std::fs::read_to_string(data.join("apps").join(&id).join("workspace/package.json"))
            .unwrap();
    assert!(package_json.contains("\"dayjs\""), "{package_json}");

    // The build changed, so the page is served from the new one; then the process goes away and takes the address with it.
    let (is_error, said, restarted) = client.call(
        "LocalAppRuntime",
        json!({"app_id": id, "action": "restart"}),
        Person::Approves,
    );
    assert!(!is_error, "restart: {said}");
    let url = restarted["url"].as_str().unwrap().to_string();
    assert!(http_get(&url)
        .expect("the restarted page")
        .0
        .contains("200"));
    client.finish();
    assert!(
        http_get(&url).is_err(),
        "the page outlived the server process"
    );

    // A later process finds the app as it was left and runs it again.
    let mut again = Client::start(&data, json!({"elicitation": {}}));
    let (is_error, said, started) = again.call(
        "LocalAppRuntime",
        json!({"app_id": id, "action": "start"}),
        Person::Approves,
    );
    assert!(!is_error, "start after restart: {said}");
    assert!(http_get(started["url"].as_str().unwrap())
        .expect("the page after the restart")
        .0
        .contains("200"));
    again.finish();
}
