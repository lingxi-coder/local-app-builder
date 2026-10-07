//! The `local-app mcp` process, driven over real pipes by the conversations in `tests/conversations/`.
//!
//! Each conversation file names the client it stands for (name, version, protocol) and lists steps: a message to
//! send and the response expected (`null` for a notification, which gets none). An expected object matches when
//! every key it names matches; `"<any>"` matches anything and `"<array>"` any array. Everything the server wrote
//! must be a response to a step, and it must write nothing to its error stream.
//!
//! The files are written from the protocol revisions' published examples. They are not captures of real clients;
//! those come with the headless client scripts, and will be added here in the same format.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn spawn(data_root: &Path, extra: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_local-app"))
        .arg("mcp")
        .arg("--data-root")
        .arg(data_root)
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn local-app mcp")
}

/// Lines from the child's stdout, delivered through a channel so a silent server fails a test instead of hanging it.
fn lines(child: &mut Child) -> mpsc::Receiver<String> {
    let stdout = child.stdout.take().expect("stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn matches(expected: &Value, actual: &Value, path: &str) -> Result<(), String> {
    match expected {
        Value::String(s) if s == "<any>" => Ok(()),
        Value::String(s) if s == "<array>" => {
            actual.is_array().then_some(()).ok_or_else(|| format!("{path}: expected an array, got {actual}"))
        }
        Value::Object(map) => {
            let Some(object) = actual.as_object() else {
                return Err(format!("{path}: expected an object, got {actual}"));
            };
            for (key, value) in map {
                let found = object.get(key).ok_or_else(|| format!("{path}.{key}: missing in {actual}"))?;
                matches(value, found, &format!("{path}.{key}"))?;
            }
            Ok(())
        }
        other => (other == actual).then_some(()).ok_or_else(|| format!("{path}: expected {other}, got {actual}")),
    }
}

fn conversations() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/conversations");
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(&dir).expect("conversations dir").flatten().map(|e| e.path()).collect();
    files.sort();
    files
}

#[test]
fn the_conversations_exist_and_name_their_client() {
    let files = conversations();
    assert!(files.len() >= 3, "{files:?}");
    for file in files {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        for field in ["name", "version", "protocol"] {
            assert!(doc["client"][field].is_string(), "{}: client.{field}", file.display());
        }
        assert!(!doc["steps"].as_array().unwrap().is_empty(), "{}", file.display());
    }
}

#[test]
fn every_conversation_is_answered_exactly_as_recorded() {
    for file in conversations() {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let root = tempfile::tempdir().unwrap();
        let mut child = spawn(root.path(), &[]);
        let mut stdin = child.stdin.take().unwrap();
        let rx = lines(&mut child);

        for (index, step) in doc["steps"].as_array().unwrap().iter().enumerate() {
            writeln!(stdin, "{}", step["send"]).unwrap();
            stdin.flush().unwrap();
            if step["expect"].is_null() {
                continue;
            }
            let line = rx
                .recv_timeout(Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("{name} step {index}: no response to {}", step["send"]));
            let actual: Value =
                serde_json::from_str(&line).unwrap_or_else(|_| panic!("{name} step {index}: stdout line is not JSON: {line}"));
            if let Err(message) = matches(&step["expect"], &actual, "$") {
                panic!("{name} step {index}: {message}\n  sent: {}\n  got:  {line}", step["send"]);
            }
        }

        drop(stdin); // the client leaves; the server must exit on its own
        let status = wait(&mut child, Duration::from_secs(20));
        assert!(status.success(), "{name}: exit status {status}");
        let extra: Vec<String> = rx.try_iter().collect();
        assert!(extra.is_empty(), "{name}: unexpected output after the last step: {extra:?}");
        let mut stderr = String::new();
        std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
        assert!(stderr.is_empty(), "{name}: wrote to stderr: {stderr}");
    }
}

fn wait(child: &mut Child, limit: Duration) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("the server did not exit after its input closed");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn two_clients_of_different_versions_can_be_served_at_the_same_time() {
    // Each of Codex and Claude Code starts its own `local-app mcp`, possibly of different wrapper versions, against
    // the same data root. Both must be answered correctly while the other is connected.
    let root = tempfile::tempdir().unwrap();
    let mut modern = spawn(root.path(), &[]);
    let mut legacy = spawn(root.path(), &[]);
    let mut modern_in = modern.stdin.take().unwrap();
    let mut legacy_in = legacy.stdin.take().unwrap();
    let modern_out = lines(&mut modern);
    let legacy_out = lines(&mut legacy);

    writeln!(legacy_in, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-11-25"}}}}"#).unwrap();
    writeln!(legacy_in, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(
        modern_in,
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{"_meta":{{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}}}}"#
    )
    .unwrap();
    writeln!(legacy_in, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();

    let reply = |rx: &mpsc::Receiver<String>| -> Value {
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(20)).expect("a response")).unwrap()
    };
    let modern_list = reply(&modern_out);
    assert_eq!(modern_list["result"]["resultType"], "complete");
    let legacy_init = reply(&legacy_out);
    assert_eq!(legacy_init["result"]["protocolVersion"], "2025-11-25");
    let legacy_list = reply(&legacy_out);
    assert!(legacy_list["result"].get("resultType").is_none());
    assert_eq!(
        modern_list["result"]["tools"].as_array().unwrap().len(),
        legacy_list["result"]["tools"].as_array().unwrap().len(),
        "both generations see the same tools"
    );

    drop(modern_in);
    drop(legacy_in);
    assert!(wait(&mut modern, Duration::from_secs(20)).success());
    assert!(wait(&mut legacy, Duration::from_secs(20)).success());
}

#[test]
fn a_bad_command_line_is_refused_without_touching_stdout() {
    let root = tempfile::tempdir().unwrap();
    for (args, code) in [
        (vec!["mcp", "--bogus"], 2),
        (vec!["mcp", "--data-root"], 2),
        (vec!["mcp", "--data-root", "relative/path"], 1),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_local-app")).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(code), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}: stdout must stay clean for the protocol");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
    let _ = root;
}

#[test]
fn an_unusable_data_root_fails_before_any_message_is_exchanged() {
    // A regular file where the root should be: the store cannot open, and the server says so on stderr and exits 1.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-directory");
    std::fs::write(&file, b"x").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_local-app"))
        .args(["mcp", "--data-root"])
        .arg(&file)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("local-app mcp:") && stderr.contains("not-a-directory"), "{stderr}");
}

#[test]
fn many_servers_can_open_one_data_root_at_the_same_time() {
    // Every client starts its own server against the same root. Before the open lock, two opening together failed
    // each other (`File exists`, `scaffold-recovery.lock` not found).
    let root = tempfile::tempdir().unwrap();
    let mut servers: Vec<(Child, std::process::ChildStdin, mpsc::Receiver<String>)> = (0..8)
        .map(|_| {
            let mut child = spawn(root.path(), &[]);
            let stdin = child.stdin.take().unwrap();
            let rx = lines(&mut child);
            (child, stdin, rx)
        })
        .collect();
    for (_, stdin, _) in &mut servers {
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"LocalAppList","arguments":{{}},"_meta":{{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}}}}"#
        )
        .unwrap();
    }
    for (index, (_, _, rx)) in servers.iter().enumerate() {
        let line = rx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| panic!("server {index} never answered"));
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["result"]["structuredContent"]["total"], 0, "server {index}: {line}");
    }
    for (mut child, stdin, _) in servers {
        drop(stdin);
        let status = wait(&mut child, Duration::from_secs(30));
        let mut stderr = String::new();
        std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
        assert!(status.success() && stderr.is_empty(), "{status} {stderr}");
    }
}
