//! The MCP wire protocol the `local-app mcp` command speaks, independent of what the tools do.
//!
//! Two protocol generations are served from one process, chosen per request by how the client opens:
//!
//! - **Modern (`2026-07-28`)** is stateless. There is no `initialize`; every request carries
//!   `_meta["io.modelcontextprotocol/protocolVersion"]`, and a result carries `resultType`. `server/discover`
//!   reports the versions and capabilities, and a request for a version this server does not serve is answered
//!   with `UnsupportedProtocolVersionError` (`-32022`) listing the ones it does.
//! - **Legacy (`2025-11-25`)** is a session: `initialize`, then `notifications/initialized`, then requests that
//!   carry no version of their own. The session is the stdio process, as the specification scopes it.
//!
//! A request that carries the modern metadata is served statelessly whether or not a legacy session was opened;
//! `initialize` selects legacy semantics. Anything else before a session exists is refused with a message that
//! names both ways in, because a legacy client has no other way to learn why it was refused.
//!
//! **Asking the person.** A tool that needs the person's approval asks through its [`CallContext`]'s [`Approver`]. In a
//! legacy session that is an `elicitation/create` request sent to the client while the call waits for the answer. A
//! modern client can only be asked through an `input_required` result (multi round-trip requests), which this server
//! does not produce yet. Every way the question cannot be put or answered (the client declared no elicitation
//! support, the modern generation, a timeout, a malformed answer, a closed stream) is [`Approval::Unavailable`] or
//! [`Approval::Declined`], never an approval: asking fails closed.
//!
//! What the tools are, and what they do, is the [`ToolBackend`]'s business. Everything here is testable with a
//! backend that is a table.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// The stateless generation.
pub const MODERN_VERSION: &str = "2026-07-28";
/// The session generation.
pub const LEGACY_VERSION: &str = "2025-11-25";
/// Every version this server serves, newest first (the order the specification's own examples use).
pub const SUPPORTED_VERSIONS: [&str; 2] = [MODERN_VERSION, LEGACY_VERSION];

const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// JSON-RPC error codes this server produces.
pub mod code {
    /// The line was not JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// The message was JSON but not a JSON-RPC request or notification, or not allowed yet.
    pub const INVALID_REQUEST: i64 = -32600;
    /// No such method in the era the request was made in.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The parameters, or the tool named in them, are wrong.
    pub const INVALID_PARAMS: i64 = -32602;
    /// The server failed.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// A modern request named a protocol version this server does not serve.
    pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
}

/// How long a client may keep a tool list, in milliseconds. The set is fixed for the life of the process.
const TOOLS_TTL_MS: u64 = 300_000;

/// What the server says about itself.
#[derive(Debug, Clone)]
pub struct ServerIdentity {
    /// The product name.
    pub name: String,
    /// The product version.
    pub version: String,
    /// Guidance for the model on using the server, if any.
    pub instructions: Option<String>,
}

/// One tool, as a client sees it.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// The name a client calls it by.
    pub name: String,
    /// A short human-readable name, if the tool has one.
    pub title: Option<String>,
    /// What the tool does, for the model.
    pub description: String,
    /// JSON Schema for the arguments.
    pub input_schema: Value,
    /// JSON Schema for `structuredContent`, if the tool promises one.
    pub output_schema: Option<Value>,
    /// The tool's claim that it observes and changes nothing (`readOnlyHint`).
    pub read_only: bool,
}

/// What a tool returned.
#[derive(Debug, Clone, Default)]
pub struct ToolResult {
    /// Content blocks (`text`, `image`, …).
    pub content: Vec<Value>,
    /// The structured form of the result, if any.
    pub structured: Option<Value>,
    /// The tool ran and reported failure, in terms the model can act on.
    pub is_error: bool,
}

impl ToolResult {
    /// A failure the model can read.
    #[must_use]
    pub fn failure(message: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": message.into()})], structured: None, is_error: true }
    }
}

/// Why a call did not produce a result at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// No tool has that name.
    UnknownTool(String),
    /// The server failed.
    Internal(String),
}

/// A question for the person, put by the server and answered yes or no.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// What the person is asked to approve, in full: the client shows this text and nothing the model wrote
    /// beyond what the server chose to quote in it.
    pub message: String,
    /// The label of the yes/no field.
    pub question: String,
}

/// The answer to an [`ApprovalRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approval {
    /// The person said yes.
    Approved,
    /// The person said no, dismissed the question, or answered in a way that is not a clear yes.
    Declined,
    /// The question could not be put or answered. The reason is for the model and the person to read.
    Unavailable(String),
}

/// Something that can ask the person.
#[async_trait]
pub trait Approver: Send + Sync {
    /// Ask. Only [`Approval::Approved`] is permission.
    async fn ask(&self, request: ApprovalRequest) -> Approval;
}

/// What a call may use besides its arguments.
#[derive(Clone)]
pub struct CallContext {
    /// How to ask the person.
    pub approver: Arc<dyn Approver>,
}

/// An approver that never asks: every question is unavailable. For calls with no client to ask, and for tests.
pub struct NoApprover(pub &'static str);

#[async_trait]
impl Approver for NoApprover {
    async fn ask(&self, _request: ApprovalRequest) -> Approval {
        Approval::Unavailable(self.0.to_string())
    }
}

/// What the protocol layer serves.
#[async_trait]
pub trait ToolBackend: Send + Sync {
    /// Every tool, in a deterministic order.
    fn tools(&self) -> Vec<ToolSpec>;
    /// Run one tool. `arguments` is always a JSON object.
    async fn call(&self, name: &str, arguments: Value, context: &CallContext) -> Result<ToolResult, CallError>;
}

/// Why a request to the client got no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// The client answered with a JSON-RPC error.
    Rejected(String),
    /// No answer came in time.
    TimedOut,
    /// The stream closed first.
    Closed,
}

/// The way back to the client: how a server sends it a request and waits for the answer.
#[async_trait]
pub trait ClientLink: Send + Sync {
    /// Send a request and wait up to `timeout` for its result. If the wait is abandoned (the future is dropped) the
    /// client is told the request is cancelled.
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, LinkError>;
}

/// How long the person has to answer a question.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

/// Which generation a call arrived in, and what that client can be asked.
struct Asking {
    link: Option<Arc<dyn ClientLink>>,
    modern: bool,
    elicitation: bool,
}

#[async_trait]
impl Approver for Asking {
    async fn ask(&self, request: ApprovalRequest) -> Approval {
        if !self.elicitation {
            return Approval::Unavailable(
                "the client did not declare support for elicitation, so the person cannot be asked, and this action \
                 needs their approval. Nothing was done."
                    .into(),
            );
        }
        if self.modern {
            return Approval::Unavailable(
                "this client speaks protocol 2026-07-28, where the person can only be asked through a multi round-trip \
                 request, which this server does not support yet. Nothing was done."
                    .into(),
            );
        }
        let Some(link) = &self.link else {
            return Approval::Unavailable("there is no way to reach the client from here. Nothing was done.".into());
        };
        let params = json!({
            "message": request.message,
            "requestedSchema": {
                "type": "object",
                "properties": {"approve": {"type": "boolean", "title": request.question, "default": false}},
                "required": ["approve"],
            },
        });
        match link.request("elicitation/create", params, APPROVAL_TIMEOUT).await {
            Ok(result) => decision(&result),
            Err(LinkError::Rejected(why)) => Approval::Unavailable(format!("the client refused the question: {why}")),
            Err(LinkError::TimedOut) => Approval::Unavailable("the person did not answer in time. Nothing was done.".into()),
            Err(LinkError::Closed) => Approval::Unavailable("the client went away before answering. Nothing was done.".into()),
        }
    }
}

/// Read an elicitation result. Only an explicit `accept` with `approve: true` is a yes; every other shape is a no.
fn decision(result: &Value) -> Approval {
    let accepted = result.get("action").and_then(Value::as_str) == Some("accept");
    let approved = result.pointer("/content/approve").and_then(Value::as_bool) == Some(true);
    if accepted && approved {
        Approval::Approved
    } else {
        Approval::Declined
    }
}

/// Whether a client's declared capabilities let a form question be put: `elicitation` present, and either empty
/// (form mode, by the specification's backwards-compatibility rule) or naming `form`.
fn can_elicit(capabilities: Option<&Value>) -> bool {
    match capabilities.and_then(|c| c.get("elicitation")) {
        Some(Value::Object(map)) => map.is_empty() || map.contains_key("form"),
        _ => false,
    }
}

/// One client's conversation with the server: the protocol state and the backend it reaches.
pub struct Session<B> {
    backend: Arc<B>,
    identity: ServerIdentity,
    legacy_initialize_seen: AtomicBool,
    legacy_initialized: AtomicBool,
    /// What the legacy client declared at `initialize`.
    legacy_capabilities: Mutex<Option<Value>>,
    /// The way back to the client, set by the transport that serves the session.
    link: OnceLock<Arc<dyn ClientLink>>,
}

/// The protocol layer's answer to one message.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// Nothing goes back (a notification).
    None,
    /// One JSON-RPC response.
    Response(Value),
}

fn error_response(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn error_with_data(id: &Value, code: i64, message: &str, data: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message, "data": data}})
}

fn ok_response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// The response to a line that could not be parsed as JSON.
#[must_use]
pub fn parse_error() -> Value {
    error_response(&Value::Null, code::PARSE_ERROR, "the line is not valid JSON")
}

/// The response to a line that exceeded the size limit.
#[must_use]
pub fn too_large(limit: usize) -> Value {
    error_response(&Value::Null, code::INVALID_REQUEST, format!("the message is larger than {limit} bytes"))
}

impl<B: ToolBackend> Session<B> {
    /// A fresh session: nothing initialized, no legacy handshake seen.
    #[must_use]
    pub fn new(backend: Arc<B>, identity: ServerIdentity) -> Self {
        Self {
            backend,
            identity,
            legacy_initialize_seen: AtomicBool::new(false),
            legacy_initialized: AtomicBool::new(false),
            legacy_capabilities: Mutex::new(None),
            link: OnceLock::new(),
        }
    }

    /// Give the session the way back to its client. The transport does this once, before serving.
    pub fn attach_link(&self, link: Arc<dyn ClientLink>) {
        let _ = self.link.set(link);
    }

    /// Answer one parsed message.
    pub async fn handle(&self, message: &Value) -> Reply {
        let Some(object) = message.as_object() else {
            return Reply::Response(error_response(
                &Value::Null,
                code::INVALID_REQUEST,
                "a message must be a JSON object; batches are not part of the protocol",
            ));
        };
        let method = object.get("method").and_then(Value::as_str);
        let id = object.get("id");
        let (Some(method), true) = (method, object.get("jsonrpc").and_then(Value::as_str) == Some("2.0")) else {
            // A response from the client (it has no method) or something that is not JSON-RPC: this server never
            // sends requests, so there is nothing a response could answer.
            return match (id, method) {
                (Some(id), Some(_)) if is_valid_id(id) => {
                    Reply::Response(error_response(id, code::INVALID_REQUEST, "not a JSON-RPC 2.0 request"))
                }
                _ => Reply::None,
            };
        };
        let Some(id) = id else {
            self.notification(method, object);
            return Reply::None;
        };
        if !is_valid_id(id) {
            return Reply::Response(error_response(
                &Value::Null,
                code::INVALID_REQUEST,
                "a request id must be a string or an integer",
            ));
        }
        let params = object.get("params");
        Reply::Response(self.request(id, method, params).await)
    }

    fn notification(&self, method: &str, _object: &Map<String, Value>) {
        // `notifications/cancelled` is acted on by the transport loop, which owns the in-flight calls. The rest
        // (`initialized`, `roots/list_changed`, anything unknown) change nothing a stateless server holds, except
        // that `initialized` completes a legacy session.
        if method == "notifications/initialized" && self.legacy_initialize_seen.load(Ordering::SeqCst) {
            self.legacy_initialized.store(true, Ordering::SeqCst);
        }
    }

    async fn request(&self, id: &Value, method: &str, params: Option<&Value>) -> Value {
        let params = match params {
            None | Some(Value::Null) => None,
            Some(Value::Object(map)) => Some(map),
            Some(_) => return error_response(id, code::INVALID_PARAMS, "params must be an object"),
        };
        if method == "initialize" {
            return self.initialize(id, params);
        }
        // Which era is this request in?
        match requested_version(params) {
            Requested::Version(version) if version == MODERN_VERSION => self.modern(id, method, params).await,
            Requested::Version(version) => unsupported_version(id, Some(version)),
            Requested::NotAString => error_response(
                id,
                code::INVALID_PARAMS,
                format!("_meta[\"{META_PROTOCOL_VERSION}\"] must be a string"),
            ),
            Requested::Absent => {
                if method == "server/discover" {
                    // The probe a client sends to learn what it is talking to. Withholding the answer because the
                    // probe lacks the very thing it asks about would defeat it.
                    return self.discover(id);
                }
                self.legacy(id, method, params).await
            }
        }
    }

    // ----- modern -------------------------------------------------------------------------------------------------

    async fn modern(&self, id: &Value, method: &str, params: Option<&Map<String, Value>>) -> Value {
        match method {
            "server/discover" => self.discover(id),
            "tools/list" => {
                let mut result = json!({
                    "resultType": "complete",
                    "tools": self.tool_definitions(),
                    "ttlMs": TOOLS_TTL_MS,
                    "cacheScope": "private",
                });
                self.stamp_server_info(&mut result);
                ok_response(id, result)
            }
            "tools/call" => match self.call(params, true).await {
                Ok(tool) => {
                    let mut result = tool_result_json(&tool, true);
                    self.stamp_server_info(&mut result);
                    ok_response(id, result)
                }
                Err(error) => call_failure(id, error),
            },
            // `ping`, `logging/setLevel` and the rest of what the modern revision removed, and everything this
            // server does not offer, are the same answer.
            other => error_response(id, code::METHOD_NOT_FOUND, format!("method not found: {other}")),
        }
    }

    fn discover(&self, id: &Value) -> Value {
        let mut result = json!({
            "resultType": "complete",
            "supportedVersions": SUPPORTED_VERSIONS,
            "capabilities": capabilities(),
        });
        if let Some(instructions) = &self.identity.instructions {
            result["instructions"] = json!(instructions);
        }
        result["ttlMs"] = json!(TOOLS_TTL_MS);
        result["cacheScope"] = json!("private");
        self.stamp_server_info(&mut result);
        ok_response(id, result)
    }

    fn stamp_server_info(&self, result: &mut Value) {
        let info = json!({"name": self.identity.name, "version": self.identity.version});
        match result.get_mut("_meta").and_then(Value::as_object_mut) {
            Some(meta) => {
                meta.insert(META_SERVER_INFO.into(), info);
            }
            None => result["_meta"] = json!({ META_SERVER_INFO: info }),
        }
    }

    // ----- legacy -------------------------------------------------------------------------------------------------

    fn initialize(&self, id: &Value, params: Option<&Map<String, Value>>) -> Value {
        let requested = params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str);
        if requested.is_none() {
            return error_response(id, code::INVALID_PARAMS, "initialize needs params.protocolVersion");
        }
        // A client that asks for a version this server does not serve is answered with the one it does, and
        // decides for itself whether to continue: that is the legacy negotiation.
        self.legacy_initialize_seen.store(true, Ordering::SeqCst);
        *self.legacy_capabilities.lock().expect("capabilities") = params.and_then(|p| p.get("capabilities")).cloned();
        let mut result = json!({
            "protocolVersion": LEGACY_VERSION,
            "capabilities": capabilities(),
            "serverInfo": {"name": self.identity.name, "version": self.identity.version},
        });
        if let Some(instructions) = &self.identity.instructions {
            result["instructions"] = json!(instructions);
        }
        ok_response(id, result)
    }

    async fn legacy(&self, id: &Value, method: &str, params: Option<&Map<String, Value>>) -> Value {
        if method == "ping" && self.legacy_initialize_seen.load(Ordering::SeqCst) {
            return ok_response(id, json!({}));
        }
        if !self.legacy_initialized.load(Ordering::SeqCst) {
            return error_response(
                id,
                code::INVALID_REQUEST,
                format!(
                    "no protocol version was declared and no session is open: send `initialize` (protocol \
                     {LEGACY_VERSION}) and then `notifications/initialized`, or include \
                     _meta[\"{META_PROTOCOL_VERSION}\"] = \"{MODERN_VERSION}\" in the request; call `server/discover` \
                     to list the supported versions"
                ),
            );
        }
        match method {
            "tools/list" => ok_response(id, json!({"tools": self.tool_definitions()})),
            "tools/call" => match self.call(params, false).await {
                Ok(tool) => ok_response(id, tool_result_json(&tool, false)),
                Err(error) => call_failure(id, error),
            },
            other => error_response(id, code::METHOD_NOT_FOUND, format!("method not found: {other}")),
        }
    }

    // ----- shared -------------------------------------------------------------------------------------------------

    fn tool_definitions(&self) -> Vec<Value> {
        let mut tools = self.backend.tools();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools.iter().map(tool_json).collect()
    }

    async fn call(&self, params: Option<&Map<String, Value>>, modern: bool) -> Result<ToolResult, CallFailure> {
        let params = params.ok_or_else(|| CallFailure::Invalid("tools/call needs params".into()))?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| CallFailure::Invalid("tools/call needs params.name".into()))?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(value @ Value::Object(_)) => value.clone(),
            Some(_) => return Err(CallFailure::Invalid("params.arguments must be an object".into())),
        };
        let capabilities = if modern {
            params.get("_meta").and_then(|m| m.get(META_CLIENT_CAPABILITIES)).cloned()
        } else {
            self.legacy_capabilities.lock().expect("capabilities").clone()
        };
        let context = CallContext {
            approver: Arc::new(Asking {
                link: self.link.get().cloned(),
                modern,
                elicitation: can_elicit(capabilities.as_ref()),
            }),
        };
        self.backend.call(name, arguments, &context).await.map_err(|error| match error {
            CallError::UnknownTool(tool) => CallFailure::Invalid(format!("Unknown tool: {tool}")),
            CallError::Internal(message) => CallFailure::Internal(message),
        })
    }
}

enum CallFailure {
    Invalid(String),
    Internal(String),
}

fn call_failure(id: &Value, failure: CallFailure) -> Value {
    match failure {
        CallFailure::Invalid(message) => error_response(id, code::INVALID_PARAMS, message),
        CallFailure::Internal(message) => error_response(id, code::INTERNAL_ERROR, message),
    }
}

enum Requested<'a> {
    Absent,
    NotAString,
    Version(&'a str),
}

fn requested_version(params: Option<&Map<String, Value>>) -> Requested<'_> {
    match params.and_then(|p| p.get("_meta")).and_then(|m| m.get(META_PROTOCOL_VERSION)) {
        None => Requested::Absent,
        Some(Value::String(version)) => Requested::Version(version),
        Some(_) => Requested::NotAString,
    }
}

fn unsupported_version(id: &Value, requested: Option<&str>) -> Value {
    error_with_data(
        id,
        code::UNSUPPORTED_PROTOCOL_VERSION,
        "Unsupported protocol version",
        json!({"supported": SUPPORTED_VERSIONS, "requested": requested}),
    )
}

fn is_valid_id(id: &Value) -> bool {
    id.is_string() || id.is_i64() || id.is_u64()
}

fn capabilities() -> Value {
    json!({"tools": {}})
}

fn tool_json(tool: &ToolSpec) -> Value {
    let mut value = json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": tool.input_schema,
        "annotations": {"readOnlyHint": tool.read_only},
    });
    if let Some(title) = &tool.title {
        value["title"] = json!(title);
    }
    if let Some(schema) = &tool.output_schema {
        value["outputSchema"] = schema.clone();
    }
    value
}

fn tool_result_json(tool: &ToolResult, modern: bool) -> Value {
    let mut value = json!({"content": tool.content, "isError": tool.is_error});
    if let Some(structured) = &tool.structured {
        value["structuredContent"] = structured.clone();
    }
    if modern {
        value["resultType"] = json!("complete");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Table;

    #[async_trait]
    impl ToolBackend for Table {
        fn tools(&self) -> Vec<ToolSpec> {
            // Deliberately out of order: the protocol layer owns the ordering.
            ["zeta", "alpha"]
                .into_iter()
                .map(|name| ToolSpec {
                    name: name.into(),
                    title: None,
                    description: format!("the {name} tool"),
                    input_schema: json!({"type": "object", "additionalProperties": false}),
                    output_schema: None,
                    read_only: true,
                })
                .collect()
        }

        async fn call(&self, name: &str, arguments: Value, _context: &CallContext) -> Result<ToolResult, CallError> {
            match name {
                "alpha" => Ok(ToolResult {
                    content: vec![json!({"type": "text", "text": "ok"})],
                    structured: Some(json!({"echo": arguments})),
                    is_error: false,
                }),
                "refuses" => Ok(ToolResult::failure("cannot")),
                "breaks" => Err(CallError::Internal("boom".into())),
                other => Err(CallError::UnknownTool(other.into())),
            }
        }
    }

    fn session() -> Session<Table> {
        Session::new(
            Arc::new(Table),
            ServerIdentity { name: "local-app".into(), version: "9.9.9".into(), instructions: Some("hello".into()) },
        )
    }

    fn modern_meta() -> Value {
        json!({
            META_PROTOCOL_VERSION: MODERN_VERSION,
            "io.modelcontextprotocol/clientInfo": {"name": "TestClient", "version": "1.0.0"},
            "io.modelcontextprotocol/clientCapabilities": {},
        })
    }

    fn request(id: i64, method: &str, mut params: Value) -> Value {
        if params.is_null() {
            params = json!({});
        }
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    fn modern_request(id: i64, method: &str, mut params: Value) -> Value {
        if params.is_null() {
            params = json!({});
        }
        params["_meta"] = modern_meta();
        request(id, method, params)
    }

    async fn answer(session: &Session<Table>, message: Value) -> Value {
        match session.handle(&message).await {
            Reply::Response(value) => value,
            Reply::None => panic!("expected a response to {message}"),
        }
    }

    async fn open_legacy(session: &Session<Table>) {
        let init = answer(
            session,
            request(1, "initialize", json!({"protocolVersion": LEGACY_VERSION, "capabilities": {}, "clientInfo": {"name": "Old", "version": "1"}})),
        )
        .await;
        assert_eq!(init["result"]["protocolVersion"], LEGACY_VERSION);
        let reply = session.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
        assert_eq!(reply, Reply::None);
    }

    #[tokio::test]
    async fn discover_lists_both_versions_and_identifies_the_server() {
        let reply = answer(&session(), modern_request(1, "server/discover", Value::Null)).await;
        let result = &reply["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["supportedVersions"], json!(["2026-07-28", "2025-11-25"]));
        assert_eq!(result["capabilities"], json!({"tools": {}}));
        assert_eq!(result["_meta"][META_SERVER_INFO], json!({"name": "local-app", "version": "9.9.9"}));
        assert_eq!(result["instructions"], "hello");
    }

    #[tokio::test]
    async fn discover_without_metadata_still_answers_because_it_is_the_probe() {
        let reply = answer(&session(), request(1, "server/discover", Value::Null)).await;
        assert_eq!(reply["result"]["supportedVersions"], json!(["2026-07-28", "2025-11-25"]));
    }

    #[tokio::test]
    async fn a_modern_request_needs_no_handshake_and_gets_a_sorted_cacheable_list() {
        let reply = answer(&session(), modern_request(7, "tools/list", Value::Null)).await;
        assert_eq!(reply["id"], 7);
        let result = &reply["result"];
        assert_eq!(result["resultType"], "complete");
        let names: Vec<_> = result["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        assert_eq!(result["tools"][0]["annotations"]["readOnlyHint"], true);
        assert_eq!(result["ttlMs"], TOOLS_TTL_MS);
        assert_eq!(result["cacheScope"], "private");
    }

    #[tokio::test]
    async fn a_modern_call_returns_a_complete_result_with_structured_content() {
        let reply = answer(&session(), modern_request(2, "tools/call", json!({"name": "alpha", "arguments": {"x": 1}}))).await;
        let result = &reply["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["text"], "ok");
        assert_eq!(result["structuredContent"], json!({"echo": {"x": 1}}));
    }

    #[tokio::test]
    async fn a_tool_that_reports_failure_is_a_result_not_a_protocol_error() {
        let reply = answer(&session(), modern_request(2, "tools/call", json!({"name": "refuses"}))).await;
        assert!(reply.get("error").is_none());
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(reply["result"]["content"][0]["text"], "cannot");
    }

    #[tokio::test]
    async fn an_unknown_tool_and_a_server_failure_are_protocol_errors_with_the_right_codes() {
        let unknown = answer(&session(), modern_request(3, "tools/call", json!({"name": "nope"}))).await;
        assert_eq!(unknown["error"]["code"], code::INVALID_PARAMS);
        assert_eq!(unknown["error"]["message"], "Unknown tool: nope");
        let broken = answer(&session(), modern_request(4, "tools/call", json!({"name": "breaks"}))).await;
        assert_eq!(broken["error"]["code"], code::INTERNAL_ERROR);
    }

    #[tokio::test]
    async fn bad_call_parameters_are_invalid_params() {
        for params in [json!({}), json!({"name": 5}), json!({"name": "alpha", "arguments": [1]})] {
            let reply = answer(&session(), modern_request(1, "tools/call", params.clone())).await;
            assert_eq!(reply["error"]["code"], code::INVALID_PARAMS, "{params}");
        }
    }

    #[tokio::test]
    async fn an_unserved_version_is_refused_with_the_supported_list() {
        for version in ["1900-01-01", LEGACY_VERSION] {
            let mut message = modern_request(5, "tools/list", Value::Null);
            message["params"]["_meta"][META_PROTOCOL_VERSION] = json!(version);
            let reply = answer(&session(), message).await;
            assert_eq!(reply["error"]["code"], code::UNSUPPORTED_PROTOCOL_VERSION, "{version}");
            assert_eq!(
                reply["error"]["data"],
                json!({"supported": ["2026-07-28", "2025-11-25"], "requested": version}),
                "{version}"
            );
        }
    }

    #[tokio::test]
    async fn a_non_string_version_is_invalid_params_not_a_crash() {
        let mut message = modern_request(5, "tools/list", Value::Null);
        message["params"]["_meta"][META_PROTOCOL_VERSION] = json!(20260728);
        let reply = answer(&session(), message).await;
        assert_eq!(reply["error"]["code"], code::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn what_the_modern_revision_removed_is_method_not_found() {
        for method in ["ping", "logging/setLevel", "resources/list", "tasks/list"] {
            let reply = answer(&session(), modern_request(1, method, Value::Null)).await;
            assert_eq!(reply["error"]["code"], code::METHOD_NOT_FOUND, "{method}");
        }
    }

    #[tokio::test]
    async fn a_request_with_no_version_and_no_session_is_refused_with_both_ways_in() {
        let reply = answer(&session(), request(1, "tools/list", Value::Null)).await;
        assert_eq!(reply["error"]["code"], code::INVALID_REQUEST);
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(message.contains("initialize") && message.contains(MODERN_VERSION), "{message}");
    }

    #[tokio::test]
    async fn a_legacy_session_serves_tools_without_per_request_metadata() {
        let session = session();
        open_legacy(&session).await;
        let list = answer(&session, request(2, "tools/list", Value::Null)).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 2);
        assert!(list["result"].get("resultType").is_none(), "legacy results carry no resultType");
        assert!(list["result"].get("ttlMs").is_none());
        let call = answer(&session, request(3, "tools/call", json!({"name": "alpha", "arguments": {}}))).await;
        assert_eq!(call["result"]["isError"], false);
        assert!(call["result"].get("resultType").is_none());
        let ping = answer(&session, request(4, "ping", Value::Null)).await;
        assert_eq!(ping["result"], json!({}));
    }

    #[tokio::test]
    async fn the_legacy_session_is_not_open_until_initialized_arrives() {
        let session = session();
        let init = answer(&session, request(1, "initialize", json!({"protocolVersion": LEGACY_VERSION}))).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "local-app");
        assert_eq!(init["result"]["instructions"], "hello");
        let early = answer(&session, request(2, "tools/list", Value::Null)).await;
        assert_eq!(early["error"]["code"], code::INVALID_REQUEST);
        // ping is allowed once initialize has been answered, as the legacy lifecycle permits.
        assert_eq!(answer(&session, request(3, "ping", Value::Null)).await["result"], json!({}));
    }

    #[tokio::test]
    async fn initialize_answers_with_the_served_version_whatever_the_client_asked_for() {
        let reply = answer(&session(), request(1, "initialize", json!({"protocolVersion": "2024-11-05"}))).await;
        assert_eq!(reply["result"]["protocolVersion"], LEGACY_VERSION);
        let bad = answer(&session(), request(1, "initialize", json!({}))).await;
        assert_eq!(bad["error"]["code"], code::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn a_modern_request_is_served_statelessly_inside_a_legacy_session() {
        let session = session();
        open_legacy(&session).await;
        let reply = answer(&session, modern_request(9, "tools/list", Value::Null)).await;
        assert_eq!(reply["result"]["resultType"], "complete");
    }

    #[tokio::test]
    async fn a_modern_server_run_never_opens_a_session_by_itself() {
        let session = session();
        let _ = answer(&session, modern_request(1, "tools/list", Value::Null)).await;
        let bare = answer(&session, request(2, "tools/list", Value::Null)).await;
        assert_eq!(bare["error"]["code"], code::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn notifications_get_no_reply_and_unknown_ones_are_ignored() {
        let session = session();
        for method in ["notifications/initialized", "notifications/roots/list_changed", "notifications/whatever"] {
            let reply = session.handle(&json!({"jsonrpc": "2.0", "method": method})).await;
            assert_eq!(reply, Reply::None, "{method}");
        }
        // `initialized` without `initialize` does not open a session.
        let bare = answer(&session, request(1, "tools/list", Value::Null)).await;
        assert_eq!(bare["error"]["code"], code::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn malformed_messages_are_answered_without_panicking() {
        let session = session();
        let batch = match session.handle(&json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}])).await {
            Reply::Response(value) => value,
            Reply::None => panic!("a batch is an invalid request"),
        };
        assert_eq!(batch["error"]["code"], code::INVALID_REQUEST);
        assert_eq!(batch["id"], Value::Null);

        let bad_id = answer(&session, json!({"jsonrpc": "2.0", "id": null, "method": "tools/list"})).await;
        assert_eq!(bad_id["error"]["code"], code::INVALID_REQUEST);
        let bad_id = answer(&session, json!({"jsonrpc": "2.0", "id": 1.5, "method": "tools/list"})).await;
        assert_eq!(bad_id["error"]["code"], code::INVALID_REQUEST);

        let no_version = answer(&session, json!({"id": 3, "method": "tools/list"})).await;
        assert_eq!(no_version["error"]["code"], code::INVALID_REQUEST);
        assert_eq!(no_version["id"], 3);

        let bad_params = answer(&session, json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": [1]})).await;
        assert_eq!(bad_params["error"]["code"], code::INVALID_PARAMS);

        // A client's response to something the server never asked is dropped, not answered.
        let stray = session.handle(&json!({"jsonrpc": "2.0", "id": 5, "result": {}})).await;
        assert_eq!(stray, Reply::None);
    }

    #[tokio::test]
    async fn string_ids_are_echoed_unchanged() {
        let mut message = modern_request(0, "tools/list", Value::Null);
        message["id"] = json!("discover-1");
        let reply = answer(&session(), message).await;
        assert_eq!(reply["id"], "discover-1");
    }
}
