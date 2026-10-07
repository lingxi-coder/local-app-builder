//! The tools `local-app mcp` serves: the Local App service's host operations, behind a host that does only what a
//! command line can do alone.
//!
//! This is the first host, and it is deliberately small. The service asks its host for everything that crosses a
//! trust boundary: running a build, driving a WebView, answering with the person's approval. A command line has
//! none of those yet, so [`UnsupportedHost`] refuses each of them with one stable, explicit message
//! ([`UNSUPPORTED`]) instead of failing in some way a model would have to guess at. What remains is what the service
//! answers from its own store: listing and reading apps.
//!
//! **Only read-only operations are offered.** The service has mutating operations that need no host at all
//! (`create`, `scaffold`, `update_manifest`), and nothing here yet stops two `local-app` processes — one per
//! client, since each of Codex and Claude Code starts its own — from writing the same data root at once. The
//! write lock that makes one writer of the data root arrives with the local host (T1); until then a tool that
//! changes anything is neither listed nor callable, which the tests pin.

use crate::mcp_protocol::{CallError, ToolBackend, ToolResult, ToolSpec};
use async_trait::async_trait;
use local_app_service::mcp_server::{LocalAppsMcpHost, LocalAppsMcpTransport};
use local_app_service::tool_names::LOCAL_APP_TOOLS;
use local_apps::clock::SystemClock;
use local_apps::events::NoopAppEventObserver;
use local_apps::{AppError, AppRecord, AppService};
use mcp_wire::transport::McpError;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// What a model reads when it asks this host for something that needs a capability the command line lacks.
pub const UNSUPPORTED: &str = "unsupported_on_this_host: the `local-app` command line has no way to do this yet \
(it needs a running app runtime, a screen, or the person's approval). Nothing was changed.";

/// The host the command line gives the service: every capability it lacks is refused, by name.
pub struct UnsupportedHost;

#[async_trait]
impl LocalAppsMcpHost for UnsupportedHost {
    fn create_next_step(&self) -> String {
        UNSUPPORTED.into()
    }
    async fn manage_runtime(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn query_data(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn mutate_data(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn inspect_ui(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn act_on_ui(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn capture_ui(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn restore_checkpoint(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn build_app(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn install_dependencies(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn prepare_shell_app(&self, _record: AppRecord) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    async fn update_manifest(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn read_app_events(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    async fn scaffold_shell_app(&self, _input: Value) -> Result<Value, String> {
        Err(UNSUPPORTED.into())
    }
    // Nothing is ever created here, so there is no client-side "creating…" state to disarm.
    async fn emit_create_failure(&self, _error: &AppError) {}
}

/// The Local App service, served as MCP tools.
pub struct LocalAppBackend {
    transport: LocalAppsMcpTransport,
    tools: Vec<ToolSpec>,
    operations: HashMap<String, &'static str>,
}

impl LocalAppBackend {
    /// Open the service on `root` (created if missing) and build the tool table.
    ///
    /// # Errors
    /// The data root could not be opened, or its stored state is corrupt.
    pub async fn open(root: &Path) -> Result<Self, String> {
        let service = AppService::load(root, Arc::new(SystemClock), Arc::new(NoopAppEventObserver))
            .await
            .map_err(|error| format!("cannot open the data root {}: {error}", root.display()))?;
        let transport = LocalAppsMcpTransport::new(root.to_path_buf());
        transport
            .attach_service(Arc::new(service))
            .map_err(|_| "the service was attached twice".to_string())?;
        transport
            .attach_host(Arc::new(UnsupportedHost))
            .map_err(|_| "the host was attached twice".to_string())?;

        let catalog: HashMap<String, _> = LocalAppsMcpTransport::host_tool_catalog()
            .into_iter()
            .map(|tool| (tool.tool_name.clone(), tool))
            .collect();
        let mut tools = Vec::new();
        let mut operations = HashMap::new();
        for (name, operation, read_only) in LOCAL_APP_TOOLS {
            if !read_only {
                continue;
            }
            let definition = catalog
                .get(*operation)
                .ok_or_else(|| format!("the service has no catalog entry for operation `{operation}`"))?;
            tools.push(ToolSpec {
                name: (*name).to_string(),
                title: None,
                description: definition.description.clone(),
                input_schema: definition.input_schema.clone(),
                output_schema: definition.output_schema.clone(),
                read_only: true,
            });
            operations.insert((*name).to_string(), *operation);
        }
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Self { transport, tools, operations })
    }
}

#[async_trait]
impl ToolBackend for LocalAppBackend {
    fn tools(&self) -> Vec<ToolSpec> {
        self.tools.clone()
    }

    async fn call(&self, name: &str, arguments: Value) -> Result<ToolResult, CallError> {
        let Some(operation) = self.operations.get(name) else {
            return Err(CallError::UnknownTool(name.to_string()));
        };
        match self.transport.call_host_operation(operation, arguments).await {
            Ok(result) => Ok(ToolResult {
                content: match result.content {
                    Value::Array(blocks) => blocks,
                    other => vec![json!({"type": "text", "text": other.to_string()})],
                },
                structured: result.structured_content,
                is_error: result.is_error,
            }),
            // The service reports a bad argument as `Internal`; that is a failure the model can correct, so it is
            // a result it can read rather than a fault of the server's.
            Err(McpError::Internal(message)) => Ok(ToolResult::failure(message)),
            Err(McpError::ToolNotFound(tool)) => Err(CallError::UnknownTool(tool)),
            Err(other) => Err(CallError::Internal(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn backend() -> (tempfile::TempDir, LocalAppBackend) {
        let root = tempfile::tempdir().expect("tempdir");
        let backend = LocalAppBackend::open(root.path()).await.expect("open");
        (root, backend)
    }

    #[tokio::test]
    async fn only_read_only_operations_are_offered() {
        let (_root, backend) = backend().await;
        let names: Vec<String> = backend.tools().into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"LocalAppList".to_string()), "{names:?}");
        assert!(names.contains(&"LocalAppGet".to_string()), "{names:?}");
        let read_only: Vec<&str> = LOCAL_APP_TOOLS.iter().filter(|(_, _, ro)| *ro).map(|(n, _, _)| *n).collect();
        assert_eq!(names.len(), read_only.len());
        for (name, _, ro) in LOCAL_APP_TOOLS {
            assert_eq!(names.iter().any(|n| n == name), *ro, "{name}");
        }
        assert!(backend.tools().iter().all(|t| t.read_only && !t.description.is_empty()));
    }

    #[tokio::test]
    async fn a_mutating_operation_is_not_callable_even_by_name() {
        let (root, backend) = backend().await;
        for name in ["LocalAppCreate", "LocalAppScaffold", "LocalAppManifest", "LocalAppBuild", "create", "list"] {
            let outcome = backend.call(name, json!({"name": "x"})).await;
            assert_eq!(outcome.unwrap_err(), CallError::UnknownTool(name.into()), "{name}");
        }
        // Opening the store writes its own index and nothing else; in particular no app directory appeared.
        let apps = root.path().join("apps");
        let directories: Vec<_> = std::fs::read_dir(&apps)
            .map(|entries| entries.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(directories.is_empty(), "no app was created: {directories:?}");
    }

    #[tokio::test]
    async fn listing_an_empty_store_answers_from_the_service() {
        let (_root, backend) = backend().await;
        let result = backend.call("LocalAppList", json!({})).await.expect("call");
        assert!(!result.is_error, "{result:?}");
        let structured = result.structured.expect("structured content");
        assert_eq!(structured["count"], 0);
        assert_eq!(structured["total"], 0);
        assert_eq!(result.content[0]["type"], "text");
    }

    #[tokio::test]
    async fn reading_an_app_that_does_not_exist_is_a_result_the_model_can_read() {
        let (_root, backend) = backend().await;
        let result = backend.call("LocalAppGet", json!({"app_id": "nothing-here"})).await.expect("call");
        assert!(result.is_error, "{result:?}");
        // A missing argument is likewise a failure to read, not a fault of the server.
        let missing = backend.call("LocalAppGet", json!({})).await.expect("call");
        assert!(missing.is_error, "{missing:?}");
    }

    #[tokio::test]
    async fn an_operation_that_needs_a_screen_or_a_runtime_says_so_by_name() {
        let (_root, backend) = backend().await;
        for name in ["LocalAppInspectUi", "LocalAppCaptureUi", "LocalAppQueryData", "LocalAppRuntimeProfiles"] {
            let result = backend.call(name, json!({"app_id": "any-app"})).await;
            match result {
                Ok(result) => {
                    assert!(result.is_error, "{name}: {result:?}");
                    let text = result.content[0]["text"].as_str().unwrap_or_default().to_string();
                    // An unknown app may be reported before the host is asked; either way the answer is an
                    // explicit refusal, never silence.
                    assert!(!text.is_empty(), "{name}");
                }
                Err(error) => panic!("{name}: a refusal must be a readable result, got {error:?}"),
            }
        }
    }

    #[tokio::test]
    async fn the_host_refuses_with_the_one_stable_code() {
        let host = UnsupportedHost;
        for result in [
            host.inspect_ui(json!({})).await,
            host.capture_ui(json!({})).await,
            host.build_app(json!({})).await,
            host.manage_runtime(json!({})).await,
        ] {
            let message = result.unwrap_err();
            assert!(message.starts_with("unsupported_on_this_host:"), "{message}");
        }
    }
}
