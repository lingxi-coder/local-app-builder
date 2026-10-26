# Host Agent runs with AG-UI 1.0

The host's built-in `local-app` skill and current native capability advertisement are authoritative. Use `window.localApp.agent` only when the host advertises it. This bridge uses the official AG-UI 1.0 JavaScript client and protocol types.

The host owns model selection, provider credentials, authorization, tool execution, approval and questions. An App runs with the model selected for that App; it never inherits the chat that created it. Never call an LLM provider from page code or put provider credentials, host settings or connection details in an App.

## AG-UI input and host options

An AG-UI 1.0 `RunAgentInput` has required `threadId`, `runId` and ordered `messages: Message[]`; it may also carry `protocolVersion`, `state`, `tools`, `context`, `forwardedProps` and `resume`. Use the official definitions from `@ag-ui/core` when type-checking TypeScript. Keep the full protocol input and its standard fields intact.

The current Local App host supports `threadId`, `runId`, `protocolVersion`, `messages` and one namespaced host extension in `forwardedProps`. Send host-specific options only as:

```js
const forwardedProps = {
  'com.lingxi.localApp': {
    version: 1,
    outputSchema,         // optional JSON Schema for structured output
    tools: [],            // optional host tool IDs declared by this release
    skills: [],           // optional packaged Skill IDs declared by this release
    maxTurns: 6,           // optional bounded turn limit
    timeoutMs: 120000      // optional bounded duration
  }
};
```

The first-release stateless run profile accepts exactly one user `Message` on a fresh `threadId`/`runId`. Do not send an existing assistant transcript to a stateless run or reuse its client after it has accumulated assistant messages. Use a host Chat resource when the user needs multiple turns and durable history. AG-UI syntax alone does not imply host support: this profile rejects top-level `tools`, `context`, `state` and `resume`; remote-URL and provider-file-handle sources; and `audio` and `video` content parts. For local files, use only the standard content part returned by `agent.file(file)`.

Do not put host settings in AG-UI `metadata`. The top-level AG-UI `tools` field describes frontend tools that the App itself executes. It is not a host permission grant. To request host tools, discover exact opaque public IDs with `LocalApp({"action":"agent_catalog"})` from the creating chat, declare them in the App release and pass the approved IDs in the namespaced extension. The catalog is read-only and grants nothing. A packaged Skill is immutable instruction text declared in the manifest and selected by its ID in the same extension. Skills grant no tools. Host policy rechecks the release, requested IDs, user grant, platform and enterprise restrictions for every run.

`threadId` names a conversation; `runId` names one run in that conversation. Use a new random `threadId` for an independent one-shot request. Reuse the stable ID returned by the host Chat resource for a persistent conversation. Supply a new random `runId` for each new turn. If a prior request's delivery is uncertain, keep its original `runId` and exact input for a user-confirmed retry; never silently replace it with a new run.

## A short image and structured-output run

The example below uses official `Message` and `ContentPart` values. `agent.file(file)` returns one official content part and handles binary encoding. It also records the filename in versioned `metadata['com.lingxi.localApp']`; preserve that metadata. Do not construct base64 data or provider file handles yourself.

```js
const agent = window.localApp?.agent;
if (!agent) throw new Error('Open this App in a host that supports Agent runs.');

const threadId = crypto.randomUUID();
const runId = crypto.randomUUID();
const imagePart = await agent.file(photoFile); // photoFile is a browser File
const input = {
  protocolVersion: '1.0',
  threadId,
  runId,
  messages: [{
    id: crypto.randomUUID(),
    role: 'user',
    content: [
      {type: 'text', text: 'Estimate the foods, portions and calorie ranges.'},
      imagePart
    ]
  }],
  forwardedProps: {
    'com.lingxi.localApp': { version: 1, outputSchema, tools: [], maxTurns: 4, timeoutMs: 120000 }
  }
};

const {result, newMessages} = await agent.run(input); // rejects unless the host reports successful completion
const checked = validateNutritionResult(result);      // app-specific checks still apply
renderWithTextNodes(checked);
```

User message `content` may be a string or an ordered `ContentPart[]`. AG-UI defines `text`, `image`, `audio`, `video` and `document` parts, but this Local App profile accepts text plus the `image`/`document` parts returned by `agent.file(file)`; remote-URL, provider-file-handle, audio and video parts are unsupported. Use `localApp.files.pick({multiple,accept})` or a standard file input to obtain ordinary browser `File` objects. The browser SDK accepts up to eight files, 25 MiB per file and 50 MiB total; model limits may be stricter. Keep selected files in message parts, not in URLs or invented filesystem paths.

For an interactive stream, use the official `AbstractAgent` client. It builds a complete `RunAgentInput` from its `threadId` and current `Message[]`, processes typed `BaseEvent`s and returns the official result:

```js
const client = agent.createClient({threadId: crypto.randomUUID()});
client.addMessage({
  id: crypto.randomUUID(),
  role: 'user',
  content: [{type: 'text', text: prompt}]
});

let failure = null;
let outcome;
const response = await client.runAgent({
  runId: crypto.randomUUID(),
  forwardedProps: {
    'com.lingxi.localApp': {version: 1, outputSchema, tools: [], maxTurns: 6, timeoutMs: 120000}
  }
}, {
  onTextMessageContentEvent({event}) { appendPlainText(event.delta); },
  onRunErrorEvent({event}) { failure = new Error(event.message); },
  onRunFinishedEvent({outcome: terminalOutcome}) { outcome = terminalOutcome; }
});
if (failure) throw failure;
if (outcome === 'cancelled') showCancelled();
else if (outcome === 'interrupt') showWaitingForHost();
else if (outcome === 'success') renderValidatedResult(response.result);
else throw new Error('The run ended without a successful AG-UI terminal outcome.');
```

`RunAgentResult` contains `result` and `newMessages`. Use `result` for structured output requested through `outputSchema`; validate app-specific rules before acting on it. Do not scrape Markdown to recover structured data. Render model text through text nodes, never executable markup. A `RUN_ERROR` is a valid AG-UI event; raw `client.runAgent()` follows official client semantics and may resolve while delivering that event to `onRunErrorEvent`, so handle the callback as a failure. Convenience `agent.run(input)` and Chat `send(input)` resolve only after successful terminal completion and reject on `RUN_ERROR`, interruption, cancellation or a missing terminal event. The official event carries its standard `event.outcome` object; the Local App subscriber also provides normalized `params.outcome` (`success`, `interrupt` or `cancelled`). Use the normalized callback value for raw client runs and report success only for `success`. `abortRun()` requests cancellation; it does not prove cancellation until the terminal event confirms it.

Every incoming event is an official `BaseEvent` union member. Events include run lifecycle (`RUN_STARTED`, `RUN_FINISHED`, `RUN_ERROR`), streamed text (`TEXT_MESSAGE_*`), tool calls (`TOOL_CALL_*`), state (`STATE_SNAPSHOT`, `STATE_DELTA`, `MESSAGES_SNAPSHOT`), activity, reasoning, subagents and passthrough. Handle the event types your UI needs and safely ignore other supported event families. Use the protocol's standard result and event values.

## Persistent Chat threads

The host Chat resource creates a stable thread ID and maps that ID to its durable Harness Session. Open the resource, construct the same official client with its ID, and restore the official history before rendering or allowing a new turn:

```js
const chat = savedChatId
  ? await agent.chats.open(savedChatId)
  : await agent.chats.create({title: 'Attachment review'});
savedChatId = chat.id;
async function readAllHistory(chat) {
  const messages = [];
  let cursor;
  do {
    const options = {limit: 50};
    if (cursor) options.cursor = cursor;
    const page = await chat.history(options); // {messages: Message[], nextCursor?}
    messages.push(...page.messages);
    cursor = page.nextCursor;
  } while (cursor);
  return messages;
}
let history = await readAllHistory(chat); // official Message[]; read-only
const client = agent.createClient({threadId: chat.id, initialMessages: history});
renderMessages(history);
// Opening or refreshing reads history; it does not call runAgent or start a model request.
```

When the user submits a new turn, keep `history` separately for display and set the run client's messages to only the new official user `Message` before `runAgent()`:

```js
const message = {
  id: crypto.randomUUID(),
  role: 'user',
  content: [
    {type: 'text', text: prompt},
    ...fileParts
  ]
};
client.setMessages([message]); // The RunAgentInput contains only this new user turn.
const response = await client.runAgent({
  runId: crypto.randomUUID(),
  forwardedProps: {
    'com.lingxi.localApp': {version: 1, tools: [], skills: [], maxTurns: 6, timeoutMs: 120000}
  }
}, subscriber);

// The host Chat's durable Harness Session already owns the prior conversation.
history = await readAllHistory(chat);
client.setMessages(history);
renderMessages(history);
```

Do not resend the historical user/assistant messages or their File bytes on every turn. The host Chat's stable `threadId` selects its canonical Harness transcript; `RunAgentInput.messages` supplies only the new user message. Replaying old files can exceed the 116 MiB run-input limit even though each new message is within its attachment limit and the Chat remains below its larger asset-storage cap. Do not create a new Chat or thread for each turn. To reconnect to the host's read-only stream/history, use the host adapter's `connectAgent({runId})` implementation with a fresh client that has no `initialMessages`; it replays existing protocol events and does not create a model request. `connectAgent()` is for restoration, while `runAgent()` is only for a user-submitted turn.

The host Chat history is paged as `{messages: Message[], nextCursor?}`. Continue with the returned cursor until all needed pages have been read before replacing the client history; `chats.list` summaries identify a resource with `chatId`, while an opened Chat handle's `id` is its AG-UI `threadId`. Do not present one page as the entire conversation. Messages use the official `Message[]` union, not a parallel `{text,attachments}` model. Read content from the message's string or ordered `ContentPart[]`; do not assume every message is plain text. A Chat ID is an opaque app-scoped conversation identity, not a credential or provider ID. Host tools still require their separate trusted UI grant.

## Optional run ledger

When a page needs a durable run ID before the first event, the host's task ledger accepts the same AG-UI `RunAgentInput`. `tasks.start(input)` returns that input's `runId`; task observation emits standard `BaseEvent`s. `tasks.events` reads events after a sequence, `tasks.subscribe` observes them, and `tasks.get/list/cancel` address the host ledger. These methods do not introduce a new request shape or event/state vocabulary. To reconnect to an existing run after reload, read its canonical `threadId`, create a client without initial messages, and use read-only `connectAgent({runId})`:

```js
const task = await agent.tasks.get(runId);
const client = agent.createClient({threadId: task.threadId});
await client.connectAgent({runId}, {onEvent({event}) { renderEvent(event); }}); // read-only; starts no model request
```

Never start a new run merely to recover a lost page Promise, and do not claim cancellation until a cancelled terminal outcome arrives.

### Recovery and terminal outcomes

A `task_not_found` from an immediate read-only `tasks.get` can be a transient lookup miss, not a model outcome. Keep the exact draft and `runId`; retry only read-only `tasks.get`/`connectAgent` or event reads with that same identity. Never call `tasks.start`, `agent.run` or Chat `send` to recover an existing request. When a task is queued or running, keep `connectAgent` or `tasks.subscribe` observing standard events until `RUN_FINISHED` or `RUN_ERROR`; a task status or refreshed Chat history is not a terminal outcome. On successful completion, validate `RunAgentResult.result` against the declared `outputSchema` and app-specific rules before side effects.

Mobile handoff may dispatch `localApp:agent-handoff-error`. For `detail.code === 'pending_request_missing'`, correlate `detail.runId` with the saved request and explain that the returned browser storage context cannot access its original input. Ask the user to return to the original tab or reopen the App in a context sharing that storage. Do not automatically resend, replace the `runId`, or call this a model failure; a new request requires explicit user review and action. Do not infer Private Browsing or select a browser brand.

## Mobile browser and host boundary

Mobile Local Apps run in the OS-selected external browser, never in an embedded Android WebView or iOS WKWebView. Do not hardcode Safari, Chrome or another browser brand. Detect file, camera, IndexedDB and recovery support from web APIs. Load the host SDK before application calls with `if (!globalThis.localApp) await import('/__local_app_sdk.js')` in the external browser; Desktop uses its preload SDK. The SDK saves the complete input for the iOS foreground wake-and-return flow and correlates it by `runId`; it keeps pending inputs for at most 24 hours and accepted run-ID receipts for 7 days, with 32-record/100 MiB caps. It submits only after the matching native readiness return. Ordinary refresh never replays a run. The app-origin recovery worker caches only its fixed recovery page, never App pages, prompts, files, requests or results. Use `localApp.info().recovery.status` and listen for `localApp:recovery-unavailable`. Use `localApp.host.open()` only from an explicit user action to return to trusted LingXi approvals or settings.

An App page cannot select another App, configure models, grant tools or approve an action. A tool call or state update is not user consent. Keep explicit confirmation before consequential app-local writes; for example, the calorie example lets the user edit the estimates and click Save. Android keeps its user-started runtime notification while the App runs. iOS has a finite OS execution lease and may interrupt a run; do not promise indefinite background work. Browser permissions and browser site data belong to the selected browser. Stopping or deleting an App does not clear browser site data.

Never call a model provider from generated pages, expose provider IDs or secrets, or render untrusted output as HTML. Validate result schemas and application invariants at every boundary. A standalone browser preview has no host Agent and is not proof of an installed App or live provider behavior.
