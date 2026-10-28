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

`threadId` names a conversation; `runId` names one run in that conversation. Use a new random `threadId` for an independent one-shot request. Reuse the stable ID returned by the host Chat resource for a persistent conversation. Supply a new random `runId` for each new turn. Keep that Agent `runId` distinct from the Local App launch `runId` returned by Mobile `localApp.info()`. If delivery is uncertain, keep the original Agent IDs and exact input for a user-initiated same-ID retry; never silently replace it with a new run.

## A short image and structured-output run

The example below illustrates the official API shape, not a complete recovery flow. Invoke it only for a user action and persist its IDs and exact input as described under Recovery before submission when recovery is needed. `agent.file(file)` returns one official content part and handles binary encoding. It also records the filename in versioned `metadata['com.lingxi.localApp']`; preserve that metadata. Do not construct base64 data or provider file handles yourself.

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

The host Chat resource creates a stable thread ID and maps that ID to its durable Harness Session. Chat creation is a host resource API, not an AG-UI standard operation. The current API is `agent.chats.create({requestId, title?})`; `requestId` is a required browser-generated lowercase canonical UUIDv4, and the returned handle's `id` equals that request ID. When present, `title` must be a string. Save the original options durably **before** the first submission, preserving exact whitespace and the distinction between an omitted title and an explicitly supplied title. Do not regenerate the ID, normalize the title, or fill in an omitted title on retry. The same ID with the same original input returns the same Chat; conflicting input rejects with `chat_create_conflict`, and a deleted creation rejects with `chat_create_deleted` rather than resurrecting it. There is no API compatibility path for a creation without `requestId`.

Create only from a user's explicit **New Chat** action. For example, after checking there is no unresolved saved creation:

```js
const {id: appId, revision} = await localApp.info();
const creationKey = `my-app:${appId}:${revision}:pending-chat-creation`;
const selectedKey = `my-app:${appId}:${revision}:selected-chat-id`;
const options = {requestId: crypto.randomUUID(), title: 'Attachment review'};
localStorage.setItem(creationKey, JSON.stringify(options)); // must succeed before submission
const created = await agent.chats.create(options);
if (created.id !== options.requestId) throw new Error('Unexpected Chat identity');
localStorage.setItem(selectedKey, created.id); // retain identity before clearing the intent
if (localStorage.getItem(creationKey) === JSON.stringify(options)) localStorage.removeItem(creationKey);
```

If a request fails with `host_unavailable`, `chat_storage_uncertain` or another ambiguous transport/storage error, keep the saved options. Use `await agent.chats.creationStatus(options)` to query read-only: it returns the same Chat handle or `null`. A `null` means only that the Chat is not currently visible; the original creation may still be in flight. On refresh or failure, never automatically call `create`, generate a replacement ID, or create from the absence of `selected-chat-id`. Only an explicit **Retry saved creation** action may call `create` again, after a read-only check, using the exact saved ID and original options. Stop and explain `chat_create_conflict` or `chat_create_deleted`; do not silently change input or allocate another ID.

`await agent.chats.pendingCreations()` returns this browser's saved same-App, same-revision creation intents as `{requestId, title?, state}` with state `pending`, `accepted`, `deleted` or `rejected`; it does not submit a request. The latter two require an explicit host deletion/rejection result; uncertain network outcomes stay `pending`. Let the user select a specific saved request ID rather than guessing by title or newest record, and clean up resolved local receipts only explicitly. It may help recover an intact original intent. Pass only `{requestId, title?}` to `creationStatus` or `create`, preserving title omission; never pass the record's `state` as a creation option. Keep your own UI/business record partitioned by `localApp.info().id` and `revision`; never replay an intent from another App or revision. Handle unavailable, cleared or corrupt browser storage. If the request ID or exact original input cannot be recovered, do not reconstruct a retry: list existing Chats read-only, explain the uncertainty, and require user review before discarding the record and separately starting a new Chat. Forgetting a local record does not cancel an in-flight creation or delete a host Chat. The complete [attachment Chat example](../../../assets/examples/attachment-chat-skill/README.md) implements explicit check/retry controls.

Once a saved or confirmed Chat ID is available, open the resource, construct the official client with its ID, and restore history before rendering or allowing a new turn. Ordinary startup with no ID should offer selection/creation controls without submitting anything:

```js
const chat = await agent.chats.open(savedChatId);
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

Across Desktop and Mobile, `chats.list()` may also return `unavailableChats: Array<{chatId, code: 'chat_storage_uncertain'}>` and `hasUncertainChats: boolean`. `unavailableChats` contains IDs **on that page only**; continue pagination even when `chats` is empty. `hasUncertainChats` is an App-wide signal on every page, so `true` can appear before the page containing any unavailable ID. Preserve an observed `true` while collecting pages; never replace it with the current page's empty array or a later omitted field. An omitted field does not prove there is no corruption or unconfirmed Chat. Show uncertainty when the host has not established the state, and keep the original Chat IDs and inputs. An unavailable or unlisted Chat is not proven deleted or nonexistent. Do not automatically replay its work, create a replacement, or silently select a different Chat as its continuation. Offer read-only list/status checks and direct the user to review the state in LingXi. `creationStatus` still requires the saved request ID and exact original creation options; do not fabricate them from an unavailable list entry.

This also applies when the host cannot open the existing Agent/Chat store as a whole because its format is unsupported, missing or cannot be verified. Display unavailable, retain the original data and saved identities, and direct the user to inspect the host state. Do not interpret a failed read as an empty store, automatically delete/reset/rebuild it, or create replacement Chats. Do not promise migration or infer a platform-specific repair from an error code.

## Optional run ledger

When a page needs a durable run ID before the first event, the host's task ledger accepts the same AG-UI `RunAgentInput`. `tasks.start(input)` returns that input's `runId`; task observation emits standard `BaseEvent`s. `tasks.events` reads events after a sequence, `tasks.subscribe` observes them, and `tasks.get/list/cancel` address the host ledger. These methods do not introduce a new request shape or event/state vocabulary. To reconnect to an existing run after reload, read its canonical `threadId`, create a client without initial messages, and use read-only `connectAgent({runId})`:

```js
const task = await agent.tasks.get(runId);
const client = agent.createClient({threadId: task.threadId});
await client.connectAgent({runId}, {onEvent({event}) { renderEvent(event); }}); // read-only; starts no model request
```

Never start a new run merely to recover a lost page Promise, and do not claim cancellation until a cancelled terminal outcome arrives.

### Recovery and terminal outcomes

Before the initial submission, save the Agent `threadId`, `runId`, message IDs, exact `RunAgentInput` or equivalent immutable official message/parts and options, plus the minimum business draft needed to restore the UI. Save them transactionally where possible and handle quota, unavailable storage and user-cleared data without claiming recovery is guaranteed. A JavaScript Promise or in-memory variable does not survive a reload. A new edit is a new request, not a mutation of a saved run. In particular, `agent.file(file)` creates a new content-part ID on each call: keep the original returned part for an exact retry instead of converting the same File again.

On reload, page return or an explicit check, use only read-only `tasks.get(runId)`, `tasks.events`, `connectAgent({runId})` and/or Chat history with the saved identity. Check that a returned task belongs to the saved thread. A `task_not_found` can be a transient lookup miss, not proof of non-admission or a model outcome. Keep the draft and retry the read-only check; if the host is unavailable, show an unknown or needs-return state. Do not call `tasks.start`, `agent.run`, `runAgent` or Chat `send` merely to restore an existing run. When queued or running, observe standard events until `RUN_FINISHED` or `RUN_ERROR`; a task status, timeout, browser return or refreshed Chat history is not a successful terminal outcome. Validate `RunAgentResult.result` against the declared `outputSchema` and app-specific rules before side effects.

If submission is still uncertain, only an explicit user retry may submit the exact saved input with the same IDs and method, after a read-only check. The host's matching-input admission deduplication is scoped to its available records; it is not a guarantee that arbitrary external tool effects are idempotent or reversible. Never automatically replay a run or repeat writes, payments, uploads or other tool actions after interruption. If a run is known to be admitted, reconnect to it rather than resubmitting. For a failed, cancelled or interrupted run, let the user review the known outcome and any potentially completed side effects before explicitly starting a new attempt with a new `runId`. If the original input, records or browser storage are missing or expired, explain the uncertainty and require review before any new request; do not infer failure or reconstruct a different request under the old ID.

Present states using real evidence, not invented SDK events: **pending** while the user-initiated handoff or admission is awaited; **running** when host events/status establish execution; **needs user return** when the host or an interrupt requires attention; **completed** only after a successful AG-UI terminal outcome and result validation; **interrupted/failed/cancelled** when actually reported; and **unknown** when the current outcome cannot be read. A request to cancel is not a confirmed cancellation. Progress must reflect observed work; do not turn a timer, turn limit or missing events into a percentage or success.

Mobile handoff may dispatch `localApp:agent-handoff-error`. For `detail.code === 'pending_request_missing'`, correlate `detail.runId` with the saved request and explain that the returned browser storage context cannot access its original input. Ask the user to return to the original tab or reopen the App in a context sharing that storage. Do not automatically resend, replace the `runId`, or call this a model failure; a new request requires explicit user review and action. Do not infer Private Browsing or select a browser brand.

## Mobile browser and host boundary

Mobile Local Apps run in the OS-selected external browser, never in an embedded Android WebView or iOS WKWebView. Do not hardcode Safari, Chrome or another browser brand. Detect file, camera, IndexedDB and recovery support from web APIs. Load the host SDK before application calls with `if (!globalThis.localApp) await import('/__local_app_sdk.js')` in the external browser; Desktop uses its preload SDK. The current Mobile SDK requires IndexedDB and secure hashing to reserve each Agent input; without them submission can fail. Do not promise a Mobile Agent send when storage is unavailable, and do not apply that implementation constraint to Desktop without checking its host.

The Mobile SDK stores the complete pending input, including inline attachment data, in browser IndexedDB; on iOS the foreground wake-and-return flow correlates that input by `runId` and submits it after the matching native readiness return. This is completion of the user's original handoff, not permission to replay a run on ordinary refresh. Pending records expire after 24 hours and accepted run-ID receipts after 7 days, with 32-record/100 MiB caps; cleanup is triggered by SDK activity, not a guaranteed deletion deadline. An accepted receipt retains identity and fingerprint, not the pending input. Storage can be full, inaccessible, cleared or separate in another browser context. The SDK does not provide a general business-draft store or guarantee recovery. The app-origin recovery worker caches only its fixed recovery page, never App pages, prompts, files, requests or results. Use `localApp.info().recovery.status` and listen for `localApp:recovery-unavailable`. Use `localApp.host.open()` only from an explicit user action to return to trusted LingXi approvals or settings.

Tell users when prompt or attachment data is temporarily saved in browser storage and when the host retains it in Chat history. Minimize additional copies and retention, provide an appropriate way to discard App-owned drafts, and explain that this does not necessarily delete host history or SDK-owned handoff data. Do not put prompts, file bytes, credentials or private paths in URLs, logs or notifications; do not claim browser storage is encrypted or survives clearing. The user-selected files remain subject to the host's size, media and model checks.

An App page cannot select another App, configure models, grant tools or approve an action. A tool call or state update is not user consent. Keep explicit confirmation before consequential app-local writes; for example, the calorie example lets the user edit the estimates and click Save. Android keeps its user-started runtime notification while the App runs. On supported iOS 26+ hosts, the native foreground handoff can request `BGContinuedProcessingTask` for a real user-initiated task; the page cannot grant or schedule it. The OS can decline, expire or abruptly terminate the lease. It is finite and does not guarantee background completion or immediate wakeup after process death, expiration, device restart or user force-quit. Return to LingXi when needed, then check the same run read-only; show interruption or unknown until confirmed. See Apple's [continued-processing guidance](https://developer.apple.com/documentation/backgroundtasks/performing-long-running-tasks-on-ios-and-ipados/) and [task contract](https://developer.apple.com/documentation/backgroundtasks/bgcontinuedprocessingtask). Android and Desktop have their own host lifecycles; do not invent iOS restrictions on those platforms. Browser permissions and browser site data belong to the selected browser. Stopping or deleting an App does not clear browser site data.

Never call a model provider from generated pages, expose provider IDs or secrets, or render untrusted output as HTML. Validate result schemas and application invariants at every boundary. A standalone browser preview has no host Agent and is not proof of an installed App or live provider behavior.
