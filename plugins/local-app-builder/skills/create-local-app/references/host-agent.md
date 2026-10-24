# Host Agent tasks, files and persistent Chats

The host's built-in `local-app` skill and the current native wire contract are authoritative. Detect the capability with `typeof window.localApp?.agent?.run === 'function'`. A standalone browser can preview the app UI, but it has no host Agent.

## Declare access, then let the host authorize it

```json
{
  "schema": 2,
  "targets": [{"platform":"macos","environment":"web"},{"platform":"android","environment":"web"},{"platform":"ios","environment":"web"}],
  "package": {"files":["index.html","app.mjs","skills/review/SKILL.md"],"dependencyLocks":[]},
  "name": "Document Desk",
  "entry": {"type":"static","path":"index.html"},
  "capabilities": ["agent"],
  "agent": {"tools":[],"skills":[{"id":"review","path":"skills/review/SKILL.md"}]}
}
```

The user configures the App's model, grants, limits and optional background permission in trusted LingXi Apps UI. A manifest requests access; it grants nothing. Discover exact public tool IDs with `LocalApp({"action":"agent_catalog"})` in the creating chat. Never guess MCP IDs or put connection data, credentials, provider IDs or host settings in the App. A Skill is immutable release instruction text. It does not grant tools; every requested tool and Skill is checked against the release, host grant, platform and enterprise policy.

Each task is an independent App-owned Harness execution. It does not inherit the creating chat, its memory or its model credentials. Persistent App Chats use their own durable Harness Session and history. The native host owns model selection, approvals, questions and tool execution.

## File attachments

Use `localApp.files.pick({multiple?,accept?})` or a standard browser file input. `accept` may be one string or a list of extensions/MIME patterns. It returns ordinary browser `File` objects; this SDK does not accept arbitrary objects or paths. Pass files directly in `attachments`; do not convert them to base64, send a data URL, invent a filesystem path or fetch private files. The browser SDK sends JSON metadata and all selected binary parts in one multipart request. It accepts at most eight files, 25 MiB per file and 50 MiB total; model or host limits may be stricter. Unsupported file and model combinations fail with a task error.

```js
const files = await localApp.files.pick({multiple: true, accept: '.pdf,.txt,image/*'});
const input = {
  requestId: crypto.randomUUID(),
  prompt: 'Summarize these documents and list questions for the reviewer.',
  attachments: files,
  skills: ['review'],
  tools: [],
  maxTurns: 6,
  timeoutMs: 120000
};
```

## One-shot tasks

`localApp.agent.run(input)` resolves to the final reply. Start a durable task directly when you need its ID before the reply:

```js
const {runId} = await localApp.agent.tasks.start(input);
const unsubscribe = localApp.agent.tasks.subscribe(runId, event => {
  if (event.type === 'connection') {
    showHostUnavailable(event.error.message);
    return;
  }
  const task = event.task;
  if (task.state === 'succeeded') renderValidatedOutput(task.result);
  if (task.state === 'failed' || task.state === 'cancelled' || task.state === 'interrupted') {
    showTaskError(task.error);
  }
});
const current = await localApp.agent.tasks.get(runId);
// localApp.agent.tasks.cancel(runId) is an explicit user action.
// unsubscribe() stops observation; it does not cancel the task.
```

The public input is `{requestId?,prompt,attachments?,outputSchema?,tools?,skills?,maxTurns?,timeoutMs?}`. The SDK creates a request ID when absent. It is idempotent: the same ID and same content resolves to the same task; reusing it with changed content is an error. `tasks.start` returns `{runId}`; `tasks.get` returns a durable `Task`; `tasks.list(options?)` lists this App's tasks. `subscribe` reports ordered task state snapshots and connection errors. Task states are `queued`, `running`, `succeeded`, `failed`, `cancelled` and `interrupted`. A terminal failure is not rerun automatically. Successful `Task.result` has validated `{text,output?,attachments:[],chatId?,messageId?}` and `Task.usage` is a separate required object. Its token counts, turns, cost and estimated flag may be absent when the host cannot report them; preserve that absence instead of displaying invented zero values. The SDK's `agent.run` reply combines the result and usage.

JavaScript Promises live only in the current page. On reload, use `tasks.list/get/subscribe` to restore host-owned work. Persist an app's draft and request ID only when a user may explicitly retry. Never submit from page startup, silently replace an uncertain request with new input, or claim a Promise survived reload. A task `runId` is not a request ID.

## Persistent Chats

Create a Chat once and retain its opaque Chat ID in the App's own storage. Reopen it after refresh:

```js
let chat = savedChatId
  ? await localApp.agent.chats.open(savedChatId)
  : await localApp.agent.chats.create({title: 'Review notes'});
savedChatId = chat.id;

const reply = await chat.send({
  requestId: crypto.randomUUID(),
  prompt: 'Compare the attached drafts and summarize the differences.',
  attachments: await localApp.files.pick({multiple: true, accept: '.pdf,.docx'}),
  skills: ['review'],
  tools: []
});
const history = await chat.history({limit: 50});
const unsubscribe = chat.subscribe(event => renderChatEvent(event));
// chat.cancel() is explicit; archive/delete are separate user choices.
```

`chats.list(options?)`, `archive(chatId)` and `delete(chatId)` manage Chat records. A Chat handle exposes `id`, `send(input)`, `history({cursor?,limit?})`, `subscribe(listener)` and `cancel()`. `send` resolves to a final reply that includes `chatId` and the host-assigned `messageId`. Chat messages expose names, media types and sizes for attachments, not host paths or provider IDs. Only one turn per Chat is active at a time by default. Reopening the Chat restores its durable history; it does not revive a page Promise.

## Mobile browser and iOS recovery

Mobile pages run in the actual system-selected external browser, never an embedded Android WebView or iOS WKWebView. Detect file, camera, IndexedDB and recovery support from web APIs; do not assume a browser brand or package name. Load the SDK before app calls: `if (!globalThis.localApp) await import('/__local_app_sdk.js')`. Use standard `File` inputs when the picker API is unavailable.

The SDK handles iOS foreground wake-and-return. Before handing off, it stores the request metadata and `File` objects in browser IndexedDB. Native LingXi starts the foreground task, obtains the actual finite execution lease, then returns to the saved external-browser App URL. The SDK submits the full prompt and files once after that matching readiness return. A link carries only the App, opaque request ID and integer revision; never put prompts, file bytes, keys or permissions in it. Ordinary refresh has no readiness marker and never replays a request. If handoff or response is uncertain, let the user explicitly retry the same request ID and unchanged files.

The SDK also registers a recovery service worker only when the browser supports it. It is scoped to the App's exact loopback origin. It fetches live navigations first and can display its fixed cached recovery page after an offline error or stale-run response. It never caches App pages, Agent POST requests, prompts, files or results. The recovery page lists only pending request IDs and offers user-clicked return links to LingXi. `localApp.info().recovery.status` is `checking`, `ready` or `unavailable`; listen for `localApp:recovery-unavailable` as well, because registration may finish after page startup. Show when recovery is unavailable, but keep normal online Agent use available.

The SDK retains an unaccepted request and its files for at most 24 hours, and keeps an accepted idempotency receipt for at most 7 days. Its IndexedDB ledger is capped at 32 records and 100 MiB; a full store rejects a new request instead of discarding an uncertain one. Expired requests are deleted and never replayed. Definitive host admission rejection clears the SDK's prompt and file copy immediately. An App that keeps its own retry draft must apply a similar expiry, explain it to the user, and clear it after definitive rejection; do not make an unbounded second copy of attachments. The bundled examples expire their own File drafts after 24 hours and report when cleanup removed one.

Use `localApp.host.open()` only from an explicit user action to return to trusted LingXi approvals or settings. Display connection loss as host unavailable or paused. Android keeps its user-started runtime notification while the App is running. iOS has a finite OS budget and can interrupt a task; do not promise indefinite background execution or an unattended wake. Browser file/camera permissions and browser storage are controlled by that browser. Stopping or deleting the Local App does not clear its browser site data.

Never call an LLM provider from the generated page or include provider keys. Keep output in text nodes, validate app-specific invariants even when the host validates `outputSchema`, and never scrape Markdown to recover a failed structured result.
