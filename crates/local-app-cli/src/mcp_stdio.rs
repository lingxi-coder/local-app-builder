//! The stdio transport: newline-delimited JSON-RPC over a byte stream.
//!
//! Each request runs as its own task, so a slow tool call does not hold up the next request and
//! `notifications/cancelled` has something to cancel. The messages that change the session (`initialize` and every
//! notification) are the exception: they are handled in the order they arrive, so nothing is served ahead of them. The writer is a single task, so two responses never
//! interleave on one line. Nothing but protocol messages is ever written to the output; diagnostics go to the error
//! stream, which the specification leaves to the server.
//!
//! The server can also put a request to the client (a legacy session's `elicitation/create`): [`Links`] writes it
//! through the same writer, matches the client's answer by id as it comes off the input, tells the client when the wait
//! is abandoned, and fails every wait still open when the input closes.

use crate::mcp_protocol::{parse_error, too_large, ClientLink, LinkError, Reply, Session, ToolBackend};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, JoinSet};

/// A message larger than this is refused. A tool input is at most 256 KiB; the rest is headroom for encoding.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// How long requests already received may keep running after the client closes its end. A client that pipes its
/// requests and closes the stream (a script, a test) still gets its answers; a call that has not finished by then
/// is abandoned, because the specification wants the server gone when its input is.
pub const DRAIN_GRACE: Duration = Duration::from_secs(10);

/// A request id in a form usable as a map key: `1` and `"1"` are different ids.
fn id_key(id: &Value) -> String {
    id.to_string()
}

/// A request being served: how to stop it, and the flag the writer checks before it lets a response out.
struct Running {
    abort: AbortHandle,
    cancelled: Arc<AtomicBool>,
}

type InFlight = Mutex<HashMap<String, Running>>;

/// What a task hands the writer: the response, and (for a request) the key and flag it was registered under.
type Outgoing = (Option<(String, Arc<AtomicBool>)>, Value);

/// Requests this server has put to the client and not yet seen answered.
struct Links {
    next: AtomicU64,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Value, LinkError>>>>,
    /// `None` once the input has closed: the session outlives the stream, and holding a sender past that point would
    /// keep the writer from ever finishing.
    out: Mutex<Option<mpsc::UnboundedSender<Outgoing>>>,
}

impl Links {
    fn new(out: mpsc::UnboundedSender<Outgoing>) -> Self {
        Self { next: AtomicU64::new(1), pending: Mutex::new(HashMap::new()), out: Mutex::new(Some(out)) }
    }

    /// Hand an answer from the client to whoever is waiting for it. Anything else is ignored: an answer to a request
    /// that was given up on, or one this server never sent.
    fn deliver(&self, message: &Value) {
        let Some(id) = message.get("id").and_then(Value::as_str) else { return };
        let Some(waiter) = self.pending.lock().expect("pending requests").remove(id) else { return };
        let outcome = match message.get("error") {
            Some(error) => Err(LinkError::Rejected(
                error.get("message").and_then(Value::as_str).unwrap_or("the client returned an error").to_string(),
            )),
            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = waiter.send(outcome);
    }

    fn send(&self, message: Value) -> bool {
        self.out.lock().expect("output").as_ref().is_some_and(|out| out.send((None, message)).is_ok())
    }

    /// The input closed: nobody will answer.
    fn close(&self) {
        self.out.lock().expect("output").take();
        for (_, waiter) in self.pending.lock().expect("pending requests").drain() {
            let _ = waiter.send(Err(LinkError::Closed));
        }
    }
}

/// Withdraws a request from the table when its wait ends any way but an answer, and tells the client so.
struct Abandon<'a> {
    links: &'a Links,
    id: String,
    answered: bool,
}

impl Drop for Abandon<'_> {
    fn drop(&mut self) {
        if self.links.pending.lock().expect("pending requests").remove(&self.id).is_some() && !self.answered {
            self.links.send(serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": {"requestId": self.id, "reason": "the server stopped waiting"},
            }));
        }
    }
}

#[async_trait]
impl ClientLink for Links {
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, LinkError> {
        let id = format!("srv-{}", self.next.fetch_add(1, Ordering::SeqCst));
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().expect("pending requests").insert(id.clone(), sender);
        let mut abandon = Abandon { links: self, id: id.clone(), answered: false };
        if !self.send(serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})) {
            return Err(LinkError::Closed);
        }
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(outcome)) => {
                abandon.answered = true;
                outcome
            }
            Ok(Err(_)) => Err(LinkError::Closed),
            Err(_) => Err(LinkError::TimedOut),
        }
    }
}

/// Serve one client until it closes its end of the stream.
///
/// # Errors
/// Reading or writing the stream failed.
pub async fn serve<B, R, W>(session: Arc<Session<B>>, reader: R, writer: W) -> io::Result<()>
where
    B: ToolBackend + 'static,
    R: tokio::io::AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (out, mut lines) = mpsc::unbounded_channel::<Outgoing>();
    let in_flight: Arc<InFlight> = Arc::default();

    let writer_state = Arc::clone(&in_flight);
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some((request, message)) = lines.recv().await {
            if let Some((key, cancelled)) = &request {
                // Once a request has been cancelled nothing more is sent for it. Its entry was removed by the
                // cancellation; removing it here would take out a later request that reused the id.
                if cancelled.load(Ordering::SeqCst) {
                    continue;
                }
                writer_state.lock().expect("in-flight table").remove(key);
            }
            let mut line = message.to_string();
            line.push('\n');
            if writer.write_all(line.as_bytes()).await.is_err() || writer.flush().await.is_err() {
                break;
            }
        }
    });

    let links = Arc::new(Links::new(out.clone()));
    session.attach_link(links.clone());

    let mut tasks: JoinSet<()> = JoinSet::new();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if text.len() > MAX_MESSAGE_BYTES {
            let _ = out.send((None, too_large(MAX_MESSAGE_BYTES)));
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            let _ = out.send((None, parse_error()));
            continue;
        };
        if message.get("method").and_then(Value::as_str) == Some("notifications/cancelled") {
            cancel(&in_flight, &message);
            continue;
        }
        // An answer to a request this server put to the client: it has an id and a result or an error, and no method.
        if message.get("method").is_none() && message.get("id").is_some() && (message.get("result").is_some() || message.get("error").is_some()) {
            links.deliver(&message);
            continue;
        }
        // What changes the session is handled here, in arrival order, before the next line is read: a legacy client
        // pipes `initialize`, `notifications/initialized` and its first request without waiting, and a request must
        // never be served ahead of the handshake that precedes it on the wire.
        let method = message.get("method").and_then(Value::as_str);
        if method == Some("initialize") || (method.is_some() && message.get("id").is_none()) {
            if let Reply::Response(response) = session.handle(&message).await {
                let _ = out.send((None, response));
            }
            continue;
        }
        let key = message.get("id").filter(|_| message.get("method").is_some()).map(id_key);
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = key.clone().map(|key| (key, Arc::clone(&cancelled)));
        let session = Arc::clone(&session);
        let sender = out.clone();
        let handle = tasks.spawn(async move {
            if let Reply::Response(response) = session.handle(&message).await {
                let _ = sender.send((request, response));
            }
        });
        if let Some(key) = key {
            in_flight.lock().expect("in-flight table").insert(key, Running { abort: handle, cancelled });
        }
    }

    // The client closed its end: nobody will answer what is still waiting on it, and what it already asked for may
    // finish within the grace period.
    links.close();
    let drained = tokio::time::timeout(DRAIN_GRACE, async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        tasks.abort_all();
    }
    drop(out);
    let _ = writer_task.await;
    Ok(())
}

fn cancel(in_flight: &InFlight, message: &Value) {
    let Some(request_id) = message.pointer("/params/requestId") else { return };
    let running = in_flight.lock().expect("in-flight table").remove(&id_key(request_id));
    if let Some(running) = running {
        // The task may already have queued its response; the flag makes the writer drop it.
        running.cancelled.store(true, Ordering::SeqCst);
        running.abort.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_protocol::{Approval, ApprovalRequest, CallContext, CallError, ServerIdentity, ToolResult, ToolSpec};
    use async_trait::async_trait;
    use serde_json::json;
    use tokio::io::{duplex, split, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

    struct Backend;

    #[async_trait]
    impl ToolBackend for Backend {
        fn tools(&self) -> Vec<ToolSpec> {
            Vec::new()
        }

        async fn call(&self, name: &str, _arguments: Value, context: &CallContext) -> Result<ToolResult, CallError> {
            match name {
                // Asks the person and reports the answer, so a test can see what the protocol layer made of it.
                "ask" => {
                    let answer = context
                        .approver
                        .ask(ApprovalRequest { message: "Create the app `Errands`?".into(), question: "Approve".into() })
                        .await;
                    let text = match answer {
                        Approval::Approved => "approved".to_string(),
                        Approval::Declined => "declined".to_string(),
                        Approval::Unavailable(why) => format!("unavailable: {why}"),
                    };
                    Ok(ToolResult { content: vec![json!({"type": "text", "text": text})], ..ToolResult::default() })
                }
                // Never finishes on its own: only a cancellation or the drain limit ends it.
                "hang" => std::future::pending().await,
                "brief" => {
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    Ok(ToolResult { content: vec![json!({"type": "text", "text": "done"})], ..ToolResult::default() })
                }
                other => Err(CallError::UnknownTool(other.into())),
            }
        }
    }

    const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;

    fn call(id: &str, tool: &str) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}",{META}}}}}"#)
    }

    struct Client {
        input: tokio::io::WriteHalf<DuplexStream>,
        output: BufReader<tokio::io::ReadHalf<DuplexStream>>,
        server: tokio::task::JoinHandle<io::Result<()>>,
    }

    fn connect() -> Client {
        let (client, server) = duplex(1 << 20);
        let (server_read, server_write) = split(server);
        let session = Arc::new(Session::new(
            Arc::new(Backend),
            ServerIdentity { name: "t".into(), version: "0".into(), instructions: None },
        ));
        let server = tokio::spawn(serve(session, server_read, server_write));
        let (client_read, client_write) = split(client);
        Client { input: client_write, output: BufReader::new(client_read), server }
    }

    impl Client {
        async fn send(&mut self, line: &str) {
            self.input.write_all(line.as_bytes()).await.unwrap();
            self.input.write_all(b"\n").await.unwrap();
        }

        async fn next(&mut self) -> Value {
            let mut line = String::new();
            let read = tokio::time::timeout(Duration::from_secs(5), self.output.read_line(&mut line))
                .await
                .expect("a response within 5 s")
                .unwrap();
            assert!(read > 0, "the server closed its output");
            serde_json::from_str(&line).unwrap_or_else(|_| panic!("stdout carried a non-message line: {line:?}"))
        }

        /// The next message, with no time limit of its own: for tests where time is paused and a limit measured in
        /// the clock the test controls would fire before the thing being waited for.
        async fn next_when_time_is_paused(&mut self) -> Value {
            let mut line = String::new();
            assert!(self.output.read_line(&mut line).await.unwrap() > 0, "the server closed its output");
            serde_json::from_str(&line).unwrap_or_else(|_| panic!("non-message line {line:?}"))
        }

        /// Close the client's end and return everything left on the server's output.
        async fn finish(mut self) -> Vec<Value> {
            self.input.shutdown().await.unwrap();
            let result = tokio::time::timeout(Duration::from_secs(15), self.server).await.expect("server exits");
            result.unwrap().unwrap();
            let mut rest = Vec::new();
            let mut line = String::new();
            while self.output.read_line(&mut line).await.unwrap() > 0 {
                rest.push(serde_json::from_str(&line).unwrap_or_else(|_| panic!("non-message line {line:?}")));
                line.clear();
            }
            rest
        }
    }

    #[tokio::test]
    async fn every_output_line_is_one_json_message_and_garbage_gets_a_parse_error() {
        let mut client = connect();
        client.send("   ").await; // blank lines are skipped
        client.send("this is not json").await;
        let error = client.next().await;
        assert_eq!(error["error"]["code"], -32700);
        assert_eq!(error["id"], Value::Null);
        client.send(&call("1", "brief")).await;
        let reply = client.next().await;
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["content"][0]["text"], "done");
        assert!(client.finish().await.is_empty());
    }

    #[tokio::test]
    async fn a_slow_call_does_not_hold_up_the_next_request() {
        let mut client = connect();
        client.send(&call("1", "hang")).await;
        client.send(&call("2", "brief")).await;
        let first = client.next().await;
        assert_eq!(first["id"], 2, "the quick call answers while the hanging one is still running");
        client.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#).await;
        client.finish().await;
    }

    #[tokio::test]
    async fn a_cancelled_request_gets_no_response_and_the_next_one_still_does() {
        let mut client = connect();
        client.send(&call("1", "hang")).await;
        client.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#).await;
        client.send(&call("2", "brief")).await;
        let reply = client.next().await;
        assert_eq!(reply["id"], 2);
        assert!(client.finish().await.is_empty(), "nothing is sent for the cancelled request");
    }

    #[tokio::test]
    async fn an_id_that_was_cancelled_can_be_used_again() {
        let mut client = connect();
        client.send(&call("7", "hang")).await;
        client.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}}"#).await;
        client.send(&call("7", "brief")).await;
        let reply = client.next().await;
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"]["content"][0]["text"], "done");
        assert!(client.finish().await.is_empty());
    }

    #[tokio::test]
    async fn a_string_id_and_an_integer_id_are_different_requests() {
        let mut client = connect();
        client.send(&call("1", "brief")).await;
        // Cancelling the string "1" must not cancel the integer 1.
        client.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"1"}}"#).await;
        let reply = client.next().await;
        assert_eq!(reply["id"], 1);
        assert!(client.finish().await.is_empty());
    }

    #[tokio::test]
    async fn a_client_that_pipes_its_requests_and_closes_still_gets_the_answers() {
        let mut client = connect();
        client.send(&call("1", "brief")).await;
        client.send(&call("2", "brief")).await;
        let mut ids: Vec<i64> = client.finish().await.iter().map(|m| m["id"].as_i64().unwrap()).collect();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_that_outlives_the_drain_grace_is_abandoned_when_the_client_leaves() {
        let mut client = connect();
        client.send(&call("1", "hang")).await;
        // Time is paused and auto-advances while everything is idle, so the grace period elapses at once.
        let rest = client.finish().await;
        assert!(rest.is_empty(), "{rest:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_legacy_handshake_piped_with_its_first_request_is_never_reordered() {
        // The three messages arrive in one burst. Were `notifications/initialized` handled on its own task, the
        // request behind it could be served first and refused as "no session open".
        for round in 0..200 {
            let mut client = connect();
            client
                .send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#)
                .await;
            client.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).await;
            client.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).await;
            let init = client.next().await;
            assert_eq!(init["id"], 1, "round {round}");
            let list = client.next().await;
            assert_eq!(list["id"], 2, "round {round}");
            assert!(list.get("error").is_none(), "round {round}: {list}");
            assert!(client.finish().await.is_empty());
        }
    }

    #[tokio::test]
    async fn a_message_over_the_limit_is_refused_and_the_next_one_is_served() {
        let mut client = connect();
        client.send(&"x".repeat(MAX_MESSAGE_BYTES + 1)).await;
        let refused = client.next().await;
        assert_eq!(refused["error"]["code"], -32600);
        client.send(&call("3", "brief")).await;
        assert_eq!(client.next().await["id"], 3);
        assert!(client.finish().await.is_empty());
    }

    // ----- asking the person ----------------------------------------------------------------------------------

    const LEGACY_INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":CAPS,"clientInfo":{"name":"t","version":"0"}}}"#;

    /// A legacy session opened with the given client capabilities.
    async fn legacy_client(capabilities: &str) -> Client {
        let mut client = connect();
        client.send(&LEGACY_INIT.replace("CAPS", capabilities)).await;
        client.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).await;
        assert_eq!(client.next().await["id"], 1);
        client
    }

    fn ask_call(id: u32) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"ask","arguments":{{}}}}}}"#)
    }

    fn answer(id: &str, result: &str) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":"{id}","result":{result}}}"#)
    }

    fn text(reply: &Value) -> String {
        reply["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("no text in {reply}")).to_string()
    }

    #[tokio::test]
    async fn a_yes_from_the_person_is_an_approval_and_the_question_is_the_one_the_server_chose() {
        let mut client = legacy_client(r#"{"elicitation":{}}"#).await;
        client.send(&ask_call(2)).await;
        let question = client.next().await;
        assert_eq!(question["method"], "elicitation/create");
        assert_eq!(question["params"]["message"], "Create the app `Errands`?");
        let schema = &question["params"]["requestedSchema"];
        assert_eq!(schema["properties"]["approve"]["type"], "boolean");
        assert_eq!(schema["properties"]["approve"]["default"], false, "the form must not come pre-approved");
        assert_eq!(schema["required"][0], "approve");
        let id = question["id"].as_str().unwrap().to_string();
        client.send(&answer(&id, r#"{"action":"accept","content":{"approve":true}}"#)).await;
        let reply = client.next().await;
        assert_eq!((reply["id"].clone(), text(&reply)), (json!(2), "approved".to_string()));
        assert!(client.finish().await.is_empty());
    }

    #[tokio::test]
    async fn anything_that_is_not_a_clear_yes_is_a_no() {
        for (label, result) in [
            ("unticked", r#"{"action":"accept","content":{"approve":false}}"#),
            ("decline", r#"{"action":"decline"}"#),
            ("cancel", r#"{"action":"cancel"}"#),
            ("accept with no content", r#"{"action":"accept"}"#),
            ("a string that says yes", r#"{"action":"accept","content":{"approve":"true"}}"#),
            ("the number one", r#"{"action":"accept","content":{"approve":1}}"#),
            ("content without the field", r#"{"action":"accept","content":{"other":true}}"#),
            ("an unknown action", r#"{"action":"approve","content":{"approve":true}}"#),
            ("not an object", r#""yes""#),
            ("null", "null"),
        ] {
            let mut client = legacy_client(r#"{"elicitation":{}}"#).await;
            client.send(&ask_call(2)).await;
            let id = client.next().await["id"].as_str().unwrap().to_string();
            client.send(&answer(&id, result)).await;
            assert_eq!(text(&client.next().await), "declined", "{label}");
            client.finish().await;
        }
    }

    #[tokio::test]
    async fn a_client_that_answers_with_an_error_means_the_person_could_not_be_asked() {
        let mut client = legacy_client(r#"{"elicitation":{}}"#).await;
        client.send(&ask_call(2)).await;
        let id = client.next().await["id"].as_str().unwrap().to_string();
        client.send(&format!(r#"{{"jsonrpc":"2.0","id":"{id}","error":{{"code":-32601,"message":"Method not found"}}}}"#)).await;
        let reply = text(&client.next().await);
        assert!(reply.starts_with("unavailable: the client refused the question: Method not found"), "{reply}");
        client.finish().await;
    }

    #[tokio::test]
    async fn a_client_that_declared_no_elicitation_is_never_sent_a_question() {
        for capabilities in ["{}", r#"{"roots":{}}"#, r#"{"elicitation":{"url":{}}}"#, r#"{"elicitation":null}"#, r#"{"elicitation":true}"#] {
            let mut client = legacy_client(capabilities).await;
            client.send(&ask_call(2)).await;
            // The first and only message is the tool's result: nothing was asked.
            let reply = client.next().await;
            assert_eq!(reply["id"], 2, "{capabilities}: {reply}");
            let said = text(&reply);
            assert!(said.starts_with("unavailable: the client did not declare support for elicitation"), "{capabilities}: {said}");
            assert!(said.contains("Nothing was done"));
            assert!(client.finish().await.is_empty(), "{capabilities}");
        }
    }

    #[tokio::test]
    async fn the_form_capability_alone_is_enough() {
        let mut client = legacy_client(r#"{"elicitation":{"form":{}}}"#).await;
        client.send(&ask_call(2)).await;
        assert_eq!(client.next().await["method"], "elicitation/create");
        client.finish().await;
    }

    #[tokio::test]
    async fn the_modern_generation_is_never_asked_mid_call_and_says_why() {
        let mut client = connect();
        client
            .send(&format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"ask",{META_WITH_ELICITATION}}}}}"#
            ))
            .await;
        let reply = client.next().await;
        assert_eq!(reply["id"], 1, "no request may precede the result: {reply}");
        assert!(text(&reply).contains("multi round-trip"), "{}", text(&reply));
        assert!(client.finish().await.is_empty());
    }

    const META_WITH_ELICITATION: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{"elicitation":{"form":{}}}}"#;

    #[tokio::test]
    async fn cancelling_the_call_while_the_person_is_being_asked_withdraws_the_question() {
        let mut client = legacy_client(r#"{"elicitation":{}}"#).await;
        client.send(&ask_call(2)).await;
        let question = client.next().await;
        client.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}"#).await;
        let withdrawn = client.next().await;
        assert_eq!(withdrawn["method"], "notifications/cancelled");
        assert_eq!(withdrawn["params"]["requestId"], question["id"]);
        // A late answer to the withdrawn question changes nothing and gets no reply.
        let id = question["id"].as_str().unwrap().to_string();
        client.send(&answer(&id, r#"{"action":"accept","content":{"approve":true}}"#)).await;
        assert!(client.finish().await.is_empty());
    }

    #[tokio::test]
    async fn an_answer_with_an_id_nobody_is_waiting_for_is_ignored() {
        let mut client = legacy_client(r#"{"elicitation":{}}"#).await;
        client.send(&answer("srv-999", r#"{"action":"accept","content":{"approve":true}}"#)).await;
        client.send(&ask_call(2)).await;
        let id = client.next().await["id"].as_str().unwrap().to_string();
        assert_ne!(id, "srv-999");
        client.send(&answer("srv-999", r#"{"action":"accept","content":{"approve":true}}"#)).await;
        client.send(&answer(&id, r#"{"action":"decline"}"#)).await;
        assert_eq!(text(&client.next().await), "declined", "only the answer to the real question counts");
        client.finish().await;
    }

    #[tokio::test]
    async fn a_client_that_leaves_while_being_asked_is_a_no_and_the_server_still_exits() {
        let mut client = legacy_client(r#"{"elicitation":{}}"#).await;
        client.send(&ask_call(2)).await;
        let _question = client.next().await;
        let rest = client.finish().await;
        assert_eq!(rest.len(), 1, "{rest:?}");
        assert!(text(&rest[0]).contains("went away"), "{}", text(&rest[0]));
    }

    #[tokio::test(start_paused = true)]
    async fn a_question_nobody_answers_times_out_as_unavailable_and_is_withdrawn() {
        let mut client = legacy_client(r#"{"elicitation":{}}"#).await;
        client.send(&ask_call(2)).await;
        let question = client.next_when_time_is_paused().await;
        // Time is paused and auto-advances while everything waits, so the approval timeout elapses at once.
        let withdrawn = client.next_when_time_is_paused().await;
        assert_eq!(withdrawn["method"], "notifications/cancelled");
        assert_eq!(withdrawn["params"]["requestId"], question["id"]);
        let reply = client.next_when_time_is_paused().await;
        assert!(text(&reply).contains("did not answer in time"), "{}", text(&reply));
        client.finish().await;
    }

    #[tokio::test]
    async fn two_questions_at_once_are_answered_to_the_right_calls() {
        let mut client = legacy_client(r#"{"elicitation":{}}"#).await;
        client.send(&ask_call(2)).await;
        client.send(&ask_call(3)).await;
        let (a, b) = (client.next().await, client.next().await);
        let (first, second) = (a["id"].as_str().unwrap().to_string(), b["id"].as_str().unwrap().to_string());
        assert_ne!(first, second);
        // Answer them in the opposite order: the second question gets a yes, the first a no.
        client.send(&answer(&second, r#"{"action":"accept","content":{"approve":true}}"#)).await;
        client.send(&answer(&first, r#"{"action":"decline"}"#)).await;
        let mut by_call = std::collections::HashMap::new();
        for _ in 0..2 {
            let reply = client.next().await;
            by_call.insert(reply["id"].as_i64().unwrap(), text(&reply));
        }
        let call_of = |question: &Value| if question["id"] == a["id"] { 2 } else { 3 };
        assert_eq!(by_call[&call_of(&b)], "approved");
        assert_eq!(by_call[&call_of(&a)], "declined");
        client.finish().await;
    }
}
