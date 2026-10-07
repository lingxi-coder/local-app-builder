//! The tools `local-app mcp` serves: the Local App service's host operations, behind a host that can build.
//!
//! **What is offered.** The operations that work on this host and that a client can drive: reading apps, and the steps of
//! making one (create the empty app, prepare it from an approved plan, install its dependencies, build it, declare its
//! manifest). [`SERVED_READ`] and [`SERVED_WRITE`] name them, and each addition comes with the capability that makes it
//! work: a tool that would answer "unsupported" to every call is not listed, because it costs the model context and
//! invites a call that cannot succeed. Running an app serves its build on a loopback address; driving its screen is not here
//! yet, and neither is the page's connection to the host (see `manage_runtime`).
//!
//! **Who approves.** Creating an app from a plan, and changing its dependencies, need the person's yes. The service
//! was written for a host with a native sheet; here the question is put through MCP elicitation
//! ([`crate::Approver`]), and a client that cannot be asked gets a refusal, not an approval. `LocalAppPrepare` is
//! intercepted for exactly this: the plan the model names is read once, shown to the person in full, and only on their
//! yes recorded as approved, under the digest of the text they saw.
//!
//! **Whose turn.** Every call takes the data root through the [`Lease`], so only one `local-app` process at a time has it
//! loaded and changes it.

use crate::lease::Lease;
use crate::local_host::{within_call, HostConfig, LocalHost};
use crate::mcp_protocol::{Approval, ApprovalRequest, CallContext, CallError, ToolBackend, ToolResult, ToolSpec};
use async_trait::async_trait;
use local_app_service::mcp_server::LocalAppsMcpTransport;
use mcp_wire::transport::McpToolResultDto;
use local_app_service::plan_approval::parse_authoring_block;
use local_app_service::tool_names::LOCAL_APP_TOOLS;
use mcp_wire::transport::McpError;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Operations that only read, offered.
pub const SERVED_READ: &[&str] = &[
    "list",
    "get",
    "read_logs",
    "list_checkpoints",
    "background_list",
    "background_status",
    "template_catalog",
];

/// Operations that change an app, offered.
pub const SERVED_WRITE: &[&str] = &[
    "create",
    "prepare",
    "install_dependencies",
    "build",
    "update_manifest",
    "confirm_dependency_change",
    "update_dependencies",
    "manage_runtime",
];

/// The largest plan the person is asked to read. A plan they cannot reasonably read is not one they can approve.
pub const MAX_PLAN_BYTES: u64 = 48 * 1024;

/// Descriptions that replace the service's where it was written for the LingXi app. The service's own text stays as it
/// is, because LingXi still uses it; this host speaks to any MCP client.
const DESCRIPTIONS: &[(&str, &str)] = &[
    (
        "list",
        "List the Local Apps in this data root (id, name, brief). Read-only. The page is bounded by `limit` (default 50, \
         max 100); when `has_more` is true, narrow the list with `query`. Use it to find an app's id; the other tools \
         take that id.",
    ),
    (
        "create",
        "Create a Local App: a record and an empty workspace with no source. Give it a `brief` (what the app is for) and \
         optionally a `name`. The new app has no shape yet, and every other tool refuses it until a plan is approved and \
         prepared with `LocalAppPrepare`. The result carries the app's id.",
    ),
    (
        "prepare",
        "Turn a plan the person approves into a prepared workspace. Write the plan as a Markdown file that contains \
         exactly one fenced code block whose info string is `authoring-spec`; the block is a JSON object with `name`, \
         `brief`, `template_id` and `spec` and nothing else. `template_id` is required to create an app (choose it from \
         `LocalAppTemplateCatalog`) and must be left out to modify an app that is already built. Call this tool with the \
         app id and the absolute path of the file. The person is shown the whole plan and asked to approve it; nothing is \
         created without their yes, and the file must not change afterwards. On approval the template is landed in the \
         app's workspace and the result gives the execution id and contract handle that `LocalAppBuild` needs. If the \
         client cannot ask the person, the call fails and nothing is done. A plan that cannot be honoured is refused by \
         name: read the message and fix the plan.",
    ),
    (
        "install_dependencies",
        "Install the app's dependencies into its workspace (`pnpm install --frozen-lockfile`, install scripts disabled), \
         with the pinned toolchain. Use `wait=true` to get the final dependency state before continuing.",
    ),
    (
        "build",
        "Build the app's workspace with the pinned toolchain, offline (30-minute budget). Dependencies must be installed \
         first (`LocalAppInstallDeps`). On success the app is marked ready. A `contract_handle` from `LocalAppPrepare`, \
         together with its `workflow_run_id`, is checked against the build's provenance. On failure the message names \
         what to fix; the build log is readable with `LocalAppLogs`.",
    ),
    (
        "confirm_dependency_change",
        "Propose a change to the app's npm dependencies and, if the person approves it, get a short-lived receipt for it. \
         `changes` is a list of `{kind, package, version}`: `kind` is `add`, `update` or `remove`, `version` is an exact \
         version for `add` and `update` and is left out for `remove`. The person is shown the packages and asked to \
         approve; nothing is downloaded before that. The packages the app's template is built on cannot be changed. If the \
         client cannot ask the person, or they say no, the call fails and nothing is changed. Apply the receipt with \
         `LocalAppUpdateDependencies`.",
    ),
    (
        "update_dependencies",
        "Apply a dependency change the person approved, using the receipt from `LocalAppConfirmDependencyChange`. The \
         packages are resolved in staging with install scripts disabled, a dependency snapshot is checked, the offline \
         production build is run, and only then are the package files, `node_modules` and the build replaced together. Any \
         failure restores the previous dependencies and build.",
    ),
    (
        "manage_runtime",
        "Serve a built app's pages on this computer so the person can look at them: `start` returns a `url` on 127.0.0.1 \
         for them to open in a browser, `stop` ends it, `restart` serves the latest build. The app must have been built \
         first (`LocalAppBuild`). The address is reachable only from this computer. What the app shows is what it was \
         built to show; its own data storage, network access and device features are supplied by a native host and are \
         not available in a plain browser yet, so a page that needs them shows an error where they are used. The page is \
         served for as long as this server process runs, and while it is served no other process can change the data \
         root.",
    ),
    (
        "update_manifest",
        "Declare the app's data collections, allowed network domains and capabilities in its manifest. Every collection is \
         `{id,name,fields}` and every field is `{id,label,kind,required?,enumOptions?}`; ids are lower snake_case. \
         `recordId`, `revision`, `createdAtMs` and `updatedAtMs` are host-owned record metadata: never declare them as \
         fields. The device context is recorded by the host: never declare it.",
    ),
];

/// One offered tool: the operation behind it.
#[derive(Clone, Copy)]
struct Offered {
    operation: &'static str,
}

/// The Local App service, served as MCP tools.
pub struct LocalAppBackend {
    lease: Arc<Lease>,
    tools: Vec<ToolSpec>,
    offered: HashMap<String, Offered>,
}

impl LocalAppBackend {
    /// A backend for the data root at `root`. The root is opened once, so a root that cannot be used fails here and not
    /// in the middle of the first call.
    ///
    /// # Errors
    /// The data root cannot be opened, or its stored state is corrupt.
    pub async fn open(root: &Path, config: HostConfig) -> Result<Self, String> {
        Self::with_lease(Lease::new(root.to_path_buf(), config)).await
    }

    /// A backend that takes its turns through `lease`.
    ///
    /// # Errors
    /// The data root cannot be opened.
    pub async fn with_lease(lease: Arc<Lease>) -> Result<Self, String> {
        // Open (and so validate) the root now, then give the turn back.
        drop(lease.acquire().await.map_err(|error| error.message())?);

        let catalog: HashMap<String, _> = LocalAppsMcpTransport::host_tool_catalog()
            .into_iter()
            .map(|tool| (tool.tool_name.clone(), tool))
            .collect();
        let mut tools = Vec::new();
        let mut offered = HashMap::new();
        for (name, operation, read_only) in LOCAL_APP_TOOLS {
            let writes = SERVED_WRITE.contains(operation);
            if !(SERVED_READ.contains(operation) && *read_only) && !writes {
                continue;
            }
            let definition = catalog
                .get(*operation)
                .ok_or_else(|| format!("the service has no catalog entry for operation `{operation}`"))?;
            let mut input_schema = definition.input_schema.clone();
            if *operation == "prepare" {
                input_schema["properties"]["plan_path"]["description"] =
                    json!("Absolute path of the Markdown plan file you wrote, with its `authoring-spec` block.");
            }
            if *operation == "manage_runtime" {
                // `open`, `suspend` and `resume` are the LingXi apps' own screen and background-state controls.
                input_schema["properties"]["action"] = json!({"enum": ["start", "stop", "restart"]});
            }
            if *operation == "confirm_dependency_change" {
                input_schema["properties"]["changes"] = json!({
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": {"enum": ["add", "update", "remove"]},
                            "package": {"type": "string"},
                            "version": {"type": "string"},
                        },
                        "required": ["kind", "package"],
                        "additionalProperties": false,
                    },
                });
            }
            tools.push(ToolSpec {
                name: (*name).to_string(),
                title: None,
                description: DESCRIPTIONS
                    .iter()
                    .find(|(op, _)| op == operation)
                    .map_or_else(|| definition.description.clone(), |(_, text)| (*text).to_string()),
                input_schema,
                output_schema: definition.output_schema.clone(),
                read_only: !writes,
            });
            offered.insert((*name).to_string(), Offered { operation });
        }
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self { lease, tools, offered })
    }
}

fn convert(outcome: Result<McpToolResultDto, McpError>) -> Result<ToolResult, CallError> {
    match outcome {
        Ok(result) => Ok(ToolResult {
            content: match result.content {
                Value::Array(blocks) => blocks,
                other => vec![json!({"type": "text", "text": other.to_string()})],
            },
            structured: result.structured_content,
            is_error: result.is_error,
        }),
        // The service reports a bad argument as `Internal`; that is a failure the model can correct, so it is a result it
        // can read rather than a fault of the server's.
        Err(McpError::Internal(message)) => Ok(ToolResult::failure(message)),
        Err(McpError::ToolNotFound(tool)) => Err(CallError::UnknownTool(tool)),
        Err(other) => Err(CallError::Internal(other.to_string())),
    }
}

/// `LocalAppPrepare`, with the person's approval of the plan in front of it.
///
/// The plan file is read once. What the person is shown is that text, in full; what is recorded as approved is that
/// same text, under its digest, so the service's own check (the file must still hold the approved bytes) refuses
/// anything written to the file after the person looked.
async fn prepare_with_approval(host: &LocalHost, arguments: Value, context: &CallContext) -> Result<ToolResult, CallError> {
    let fail = |message: String| Ok(ToolResult::failure(message));
    let (Some(app_id), Some(plan_path)) =
        (arguments.get("app_id").and_then(Value::as_str), arguments.get("plan_path").and_then(Value::as_str))
    else {
        return fail("app_id and plan_path are required".into());
    };
    let path = PathBuf::from(plan_path);
    if !path.is_absolute() {
        return fail(format!("plan_path must be an absolute path, got `{plan_path}`"));
    }
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => return fail(format!("cannot read the plan file {plan_path}: {error}")),
    };
    if !metadata.is_file() {
        return fail(format!("plan_path must name a regular file, and {plan_path} is not one"));
    }
    if metadata.len() > MAX_PLAN_BYTES {
        return fail(format!(
            "the plan is {} bytes, more than the {MAX_PLAN_BYTES} the person can reasonably be asked to read and approve; \
             shorten it",
            metadata.len()
        ));
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return fail(format!("the plan file {plan_path} is not UTF-8 text"));
    };
    // A plan that could not be honoured is refused before the person is asked to approve it.
    match parse_authoring_block(&text) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return fail("plan_approval_invalid: the file has no `authoring-spec` block, so it is not a Local App plan".into())
        }
        Err(reason) => return fail(format!("plan_approval_invalid: {reason}")),
    }
    let digest: String = Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let message = format!(
        "Approve this plan for the Local App `{app_id}`?\n\nThe assistant wrote the plan below to {plan_path}. If you approve, \
         the app is built from exactly this text (sha256 {}); if the file changes afterwards, nothing is created.\n\n\
         ----- plan -----\n{text}\n----- end of plan -----",
        &digest[..16]
    );
    match context.approver.ask(ApprovalRequest { message, question: "Approve this plan".into() }).await {
        Approval::Approved => {}
        Approval::Declined => {
            return fail(
                "plan_not_approved: the person did not approve the plan. Nothing was created; revise the plan with them.".into(),
            )
        }
        Approval::Unavailable(why) => return fail(format!("approval_unavailable: {why}")),
    }
    host.plans.observe(&json!({"plan": text, "filePath": plan_path}).to_string(), &host.session);
    convert(host.transport.call_host_operation("prepare", arguments).await)
}

#[async_trait]
impl ToolBackend for LocalAppBackend {
    fn tools(&self) -> Vec<ToolSpec> {
        self.tools.clone()
    }

    async fn call(&self, name: &str, arguments: Value, context: &CallContext) -> Result<ToolResult, CallError> {
        let Some(&Offered { operation }) = self.offered.get(name) else {
            return Err(CallError::UnknownTool(name.to_string()));
        };
        let turn = match self.lease.acquire().await {
            Ok(turn) => turn,
            Err(error) => return Ok(ToolResult::failure(error.message())),
        };
        let host = Arc::clone(turn.host());
        let outcome = within_call(context.clone(), async {
            if operation == "prepare" {
                prepare_with_approval(&host, arguments, context).await
            } else {
                convert(host.transport.call_host_operation(operation, arguments).await)
            }
        })
        .await;
        drop(turn);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_protocol::NoApprover;

    fn ctx() -> CallContext {
        CallContext { approver: Arc::new(NoApprover("no client in this test")) }
    }

    async fn backend() -> (tempfile::TempDir, LocalAppBackend) {
        let root = tempfile::tempdir().expect("tempdir");
        let backend = LocalAppBackend::open(root.path(), HostConfig::default()).await.expect("open");
        (root, backend)
    }

    fn text(result: &ToolResult) -> String {
        result.content.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
    }

    #[tokio::test]
    async fn the_offered_tools_are_exactly_the_served_operations_and_say_whether_they_write() {
        let (_root, backend) = backend().await;
        let tools = backend.tools();
        let offered: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        let expected: Vec<&str> = LOCAL_APP_TOOLS
            .iter()
            .filter(|(_, op, ro)| SERVED_WRITE.contains(op) || (SERVED_READ.contains(op) && *ro))
            .map(|(name, _, _)| *name)
            .collect();
        assert_eq!(expected.len(), SERVED_READ.len() + SERVED_WRITE.len(), "every served operation has a tool row");
        assert_eq!(offered.len(), expected.len(), "{offered:?}");
        for name in &expected {
            assert!(offered.contains(name), "{name} missing from {offered:?}");
        }
        for tool in &tools {
            let writes = [
                "LocalAppCreate",
                "LocalAppPrepare",
                "LocalAppInstallDeps",
                "LocalAppBuild",
                "LocalAppManifest",
                "LocalAppConfirmDependencyChange",
                "LocalAppUpdateDependencies",
                "LocalAppRuntime",
            ]
            .contains(&tool.name.as_str());
            assert_eq!(tool.read_only, !writes, "{}", tool.name);
            assert!(!tool.description.is_empty());
        }
    }

    #[tokio::test]
    async fn nothing_offered_is_worded_for_another_product() {
        let (_root, backend) = backend().await;
        for tool in backend.tools() {
            let text = format!("{} {} {:?} {:?}", tool.name, tool.description, tool.input_schema, tool.output_schema).to_lowercase();
            for word in ["lingxi", "global conversation", "workspace contract", "exitplanmode", "plan-mode", "the engine"] {
                assert!(!text.contains(word), "{} mentions `{word}`: {text}", tool.name);
            }
        }
    }

    #[tokio::test]
    async fn every_offered_read_tool_answers_for_real_and_none_says_unsupported() {
        let (_root, backend) = backend().await;
        for tool in backend.tools().into_iter().filter(|t| t.read_only) {
            let required: Vec<String> = tool.input_schema["required"]
                .as_array()
                .map(|r| r.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let arguments: serde_json::Map<String, Value> = required.iter().map(|key| (key.clone(), json!("no-such-app"))).collect();
            let result = backend.call(&tool.name, Value::Object(arguments), &ctx()).await.unwrap_or_else(|e| panic!("{}: {e:?}", tool.name));
            assert!(!text(&result).contains("unsupported_on_this_host"), "{} is listed but unsupported: {}", tool.name, text(&result));
        }
    }

    #[tokio::test]
    async fn an_operation_that_needs_a_screen_or_a_runtime_is_not_offered_at_all() {
        let (_root, backend) = backend().await;
        for name in ["LocalAppInspectUi", "LocalAppCaptureUi", "LocalAppQueryData", "LocalAppScaffold", "LocalAppActOnUi", "create", "list"] {
            assert_eq!(backend.call(name, json!({"app_id": "any-app"}), &ctx()).await.unwrap_err(), CallError::UnknownTool(name.into()), "{name}");
        }
    }

    #[tokio::test]
    async fn listing_an_empty_store_answers_from_the_service() {
        let (_root, backend) = backend().await;
        let result = backend.call("LocalAppList", json!({}), &ctx()).await.expect("call");
        assert!(!result.is_error, "{result:?}");
        let structured = result.structured.expect("structured content");
        assert_eq!((structured["count"].clone(), structured["total"].clone()), (json!(0), json!(0)));
    }

    #[tokio::test]
    async fn reading_an_app_that_does_not_exist_is_a_result_the_model_can_read() {
        let (_root, backend) = backend().await;
        assert!(backend.call("LocalAppGet", json!({"app_id": "nothing-here"}), &ctx()).await.unwrap().is_error);
        assert!(backend.call("LocalAppGet", json!({}), &ctx()).await.unwrap().is_error);
    }

    #[tokio::test]
    async fn creating_an_app_makes_an_empty_shell_that_the_other_tools_refuse_until_it_is_prepared() {
        let (_root, backend) = backend().await;
        let created = backend.call("LocalAppCreate", json!({"brief": "Track errands", "name": "Errands"}), &ctx()).await.unwrap();
        assert!(!created.is_error, "{}", text(&created));
        let id = created
            .structured
            .as_ref()
            .and_then(|s| s["app"]["id"].as_str().or_else(|| s["id"].as_str()).or_else(|| s["app_id"].as_str()))
            .map(String::from)
            .unwrap_or_else(|| panic!("the result carries no app id: {created:?}"));
        let list = backend.call("LocalAppList", json!({}), &ctx()).await.unwrap();
        assert_eq!(list.structured.unwrap()["total"], 1);
        let got = backend.call("LocalAppGet", json!({"app_id": id}), &ctx()).await.unwrap();
        assert!(!got.is_error, "{}", text(&got));
        // A build of a shell is refused by the service, not attempted.
        let build = backend.call("LocalAppBuild", json!({"app_id": id}), &ctx()).await.unwrap();
        assert!(build.is_error, "{}", text(&build));
    }

    // ----- approving a plan -----------------------------------------------------------------------------------

    use crate::mcp_protocol::Approver;
    use local_app_service::plan_approval::test_support::{plan_with_block, spec_json};
    use std::sync::Mutex;

    /// An approver that records what it was asked and answers as told.
    struct Scripted {
        answer: Mutex<Approval>,
        asked: Mutex<Vec<ApprovalRequest>>,
        /// Run when asked, before answering: a plan rewritten while the person is deciding, for instance.
        during: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl Scripted {
        fn answering(answer: Approval) -> Arc<Self> {
            Arc::new(Self { answer: Mutex::new(answer), asked: Mutex::default(), during: Mutex::default() })
        }
        fn context(self: &Arc<Self>) -> CallContext {
            CallContext { approver: Arc::clone(self) as Arc<dyn Approver> }
        }
        fn times_asked(&self) -> usize {
            self.asked.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl Approver for Scripted {
        async fn ask(&self, request: ApprovalRequest) -> Approval {
            self.asked.lock().unwrap().push(request);
            if let Some(action) = self.during.lock().unwrap().take() {
                action();
            }
            self.answer.lock().unwrap().clone()
        }
    }

    fn plan(template: &str) -> String {
        plan_with_block(json!({
            "name": "Water Tracker",
            "brief": "Log glasses of water and see today's total.",
            "template_id": template,
            "spec": spec_json(),
        }))
    }

    /// A backend with one empty app and a plan file for it.
    async fn with_app_and_plan(text: &str) -> (tempfile::TempDir, LocalAppBackend, String, PathBuf) {
        let (root, backend) = backend().await;
        let created = backend.call("LocalAppCreate", json!({"brief": "Track water"}), &ctx()).await.unwrap();
        let s = created.structured.expect("structured");
        let id = s["app"]["id"].as_str().or_else(|| s["id"].as_str()).or_else(|| s["app_id"].as_str()).expect("app id").to_string();
        let plans = tempfile::tempdir().unwrap();
        let path = plans.path().join("plan.md");
        std::fs::write(&path, text).unwrap();
        std::mem::forget(plans); // the directory lives as long as the test process; it is a temp dir
        (root, backend, id, path)
    }

    async fn prepare(backend: &LocalAppBackend, id: &str, path: &Path, who: &CallContext) -> ToolResult {
        backend.call("LocalAppPrepare", json!({"app_id": id, "plan_path": path}), who).await.unwrap()
    }

    #[tokio::test]
    async fn the_person_is_shown_the_whole_plan_and_nothing_is_prepared_without_their_yes() {
        for (label, answer) in [
            ("declined", Approval::Declined),
            ("unavailable", Approval::Unavailable("the client did not declare support for elicitation".into())),
        ] {
            let text_of_plan = plan("react-dom-r4");
            let (_root, backend, id, path) = with_app_and_plan(&text_of_plan).await;
            let person = Scripted::answering(answer);
            let result = prepare(&backend, &id, &path, &person.context()).await;
            assert!(result.is_error, "{label}: {}", text(&result));
            let said = text(&result);
            assert!(said.starts_with("plan_not_approved:") || said.starts_with("approval_unavailable:"), "{label}: {said}");
            // The question carried the plan itself and the app, and nothing the model could dress up differently.
            let asked = person.asked.lock().unwrap();
            assert_eq!(asked.len(), 1, "{label}");
            assert!(asked[0].message.contains(&text_of_plan), "{label}: the person must see the plan in full");
            assert!(asked[0].message.contains(&format!("`{id}`")), "{label}");
            drop(asked);
            // The refusal left no approval behind: the service itself says no plan was approved.
            let again = prepare(&backend, &id, &path, &Scripted::answering(Approval::Unavailable("none".into())).context()).await;
            assert!(again.is_error);
            let got = backend.call("LocalAppGet", json!({"app_id": id}), &ctx()).await.unwrap();
            assert!(!got.is_error);
        }
    }

    #[tokio::test]
    async fn a_plan_that_cannot_be_honoured_is_refused_without_asking_the_person() {
        let long = format!("{}\n", "x".repeat(MAX_PLAN_BYTES as usize + 1));
        let no_block = "# A plan\n\nNo machine-readable block here.\n".to_string();
        let bad_json = "```authoring-spec\n{not json}\n```\n".to_string();
        let two_blocks = format!("{0}\n{0}", plan("react-dom-r4"));
        for (label, text_of_plan, expect) in [
            ("too long", long, "more than the"),
            ("no block", no_block, "has no `authoring-spec` block"),
            ("bad json", bad_json, "plan_approval_invalid"),
            ("two blocks", two_blocks, "more than one"),
        ] {
            let (_root, backend, id, path) = with_app_and_plan(&text_of_plan).await;
            let person = Scripted::answering(Approval::Approved);
            let result = prepare(&backend, &id, &path, &person.context()).await;
            assert!(result.is_error && text(&result).contains(expect), "{label}: {}", text(&result));
            assert_eq!(person.times_asked(), 0, "{label}: the person must not be asked to approve what cannot be prepared");
        }
    }

    #[tokio::test]
    async fn the_plan_path_must_be_an_absolute_path_to_a_regular_file() {
        let (_root, backend, id, path) = with_app_and_plan(&plan("react-dom-r4")).await;
        let person = Scripted::answering(Approval::Approved);
        let dir = path.parent().unwrap().to_path_buf();
        let link = dir.join("link.md");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        for (label, given) in [
            ("relative", PathBuf::from("plan.md")),
            ("a directory", dir.clone()),
            ("missing", dir.join("missing.md")),
            ("a symlink", link),
        ] {
            let result = prepare(&backend, &id, &given, &person.context()).await;
            assert!(result.is_error, "{label}: {}", text(&result));
        }
        assert_eq!(person.times_asked(), 0);
    }

    #[tokio::test]
    async fn an_approval_is_for_the_text_the_person_saw_and_not_for_a_later_edit() {
        let original = plan("react-dom-r4");
        let (_root, backend, id, path) = with_app_and_plan(&original).await;
        let person = Scripted::answering(Approval::Approved);
        // While the person decides, the model rewrites the plan to name another template.
        let rewrite = path.clone();
        *person.during.lock().unwrap() = Some(Box::new(move || std::fs::write(rewrite, plan("canvas-2d-r4")).unwrap()));
        let result = prepare(&backend, &id, &path, &person.context()).await;
        assert!(result.is_error, "{}", text(&result));
        assert!(text(&result).contains("changed after it was approved"), "{}", text(&result));
    }
}
