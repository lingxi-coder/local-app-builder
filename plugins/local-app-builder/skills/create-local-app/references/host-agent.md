# Desktop host Agent calls

The host's built-in `local-app` skill is authoritative. This contract is Desktop-only; do not add these calls to mobile document apps until their host advertises them. The capability can be detected with `typeof window.localApp?.agent?.run === 'function'`. An ordinary browser can preview the UI, but cannot run a host Agent.

## Declare, then let the host authorize

```json
{
  "schema": 1,
  "name": "Meal Notes",
  "entry": { "type": "static", "path": "index.html" },
  "capabilities": ["camera", "agent"],
  "agent": { "tools": [] }
}
```

The user configures this application's Agent model, tool grants, limits and optional background permission in LingXi Apps. A manifest requests access; it cannot grant access. No-tool image recognition uses `tools: []`. For tool tasks, the creating chat can call `LocalApp({"action":"agent_catalog"})` to discover public model and exact opaque tool IDs; it needs no app ID, contains no connection configuration or credentials, and grants no permission. Declare the desired IDs in the app. Do not guess MCP IDs from server or tool display names, and do not add arbitrary MCP connections, tool installers or delegation as a workaround for denial. Catalog discovery is an authoring tool action, not an additional method on `window.localApp.agent`.

Each invocation is a fresh application-owned Harness execution. It does not read the creating chat, personal/project memories, skills or automatic hooks. Pass only the context the user expects this app to use. Credentials, model configuration and permission responses stay in the host. `appId`, chat IDs, workspace paths, permission modes and provider keys are not invocation parameters.

## Call contract

```js
const input = {
  requestId: crypto.randomUUID(),
  prompt: 'Describe the visible food and state uncertain portions.',
  images: [{ media_type: 'image/jpeg', base64: jpegBase64 }],
  outputSchema: {
    type: 'object', additionalProperties: false,
    required: ['description'],
    properties: { description: { type: 'string' } }
  },
  tools: [],
  maxTurns: 4,
  timeoutMs: 120000
};
// Save the exact request before run; an uncertain retry uses the same payload.
const { runId } = await window.localApp.agent.run(input);
// Save runId; after reload reattach, do not create a second request.
const unsubscribe = window.localApp.agent.subscribe(runId, event => {
  if (event.type === 'state' && event.run.state === 'succeeded') {
    renderValidatedOutput(event.run.result.output);
  }
});
const current = await window.localApp.agent.get(runId);
// Cancel is an explicit user action, not a side effect of rendering a status.
// await window.localApp.agent.cancel(runId);
// unsubscribe() removes the listener; it does not cancel the host task.
```

- `run(input)` returns `Promise<{ runId: string }>`, not the final model result. Required fields are `requestId` and `prompt`; `images`, `outputSchema`, `tools`, `maxTurns` and `timeoutMs` are optional. Limits can be lowered by the app but cannot override the host's settings. Default host limits are one active task per App, 20 turns and 10 minutes.
- `images` uses `{ media_type, base64 }`: base64 bytes only, no `data:` prefix, URL or filesystem path. Use standard camera/file APIs and resize images before sending. The host validates type, encoding, file signature and limits. Do not fetch local private files by inventing a path.
- `outputSchema` is a JSON Schema object (Draft 7). A successful schema task has validated `result.output`; an unstructured task may return `result.text`. Never recover a failed schema result by stripping Markdown fences or parsing arbitrary model text as success. Validate app-specific invariants too, such as ordered ranges.
- `get(runId)` returns `{ runId, requestId, state, sequence, createdAt, updatedAt, model, limits, result?, usage?, error? }`. `result` has optional `text` and `output`; `usage` has optional `inputTokens`, `outputTokens`, `costUsd`, `turns`; `error` has `code` and `message`.
- States are `queued`, `running`, `succeeded`, `failed`, `cancelled`, `interrupted`. All four terminal states must leave the busy UI. The host does not automatically replay an interrupted task. A user-initiated new attempt gets a new request ID.
- `subscribe(runId, listener)` synchronously returns an unsubscribe function. Events are `{ type: 'state', runId, sequence, time, run }` or `{ type: 'progress', runId, sequence, time, phase, text? }`. It replays current state, then live events. Discard duplicate/out-of-order sequence numbers; a racing `get` result must not replace a newer event. Progress phases are `starting`, `thinking`, `tool`, `waiting_for_permission`, `waiting_for_answer`.
- `cancel(runId)` returns `Promise<void>`; show the recorded terminal state rather than claiming cancellation before confirmation. The app can access only its own tasks. It cannot answer tool approvals or user questions; direct the user to LingXi's trusted UI when waiting.

## Lifecycle and data

Persist request identity before crossing IPC. Retrying an identical request ID and payload is idempotent; reusing it with different input is an error. When a submission response is lost, let the user explicitly retry the same request rather than manufacturing a new ID. Refresh must not silently resubmit a pending request: an earlier rejection might otherwise start unexpectedly after the user changes permissions. Offer an explicit discard-and-replace flow for rejected or unconfirmed input, explaining that removing its recovery record does not cancel a possibly accepted task. Until the user retries or discards, do not silently substitute a new photo. Cancellation must not first call `run` to acquire a run ID. Once accepted, remove any temporary retained image payload and reconnect using the saved `runId`. Keep sensitive photos out of long-lived app storage unless the user explicitly wants a photo history.

Page reload only reconnects. Closing the window cancels tasks by default; background execution needs a separate Agent background grant. App stop, permission revocation, deletion and code-version replacement cancel tasks. Unsubscribe listeners and stop camera tracks on page teardown; do not call `cancel` during a normal reload. Do not silently recreate a cancelled/interrupted task or auto-save model output.

For nutrition, show food candidates, portion assumptions and calorie ranges; let the user adjust grams before saving. The included example has no nutrition tools and labels all output as model estimates. If a later version adds a host-approved nutrition tool, keep its actual source references with the result and distinguish sourced density values from image-estimated portions. Never claim a database lookup merely because a model returned a citation.

## Verification

Use the sample's `agent-client.mjs` for request/reload/event-order handling and `nutrition.mjs` for app-specific schema/range checks. Run `node --test scripts/tests/test-desktop-calorie-agent.mjs` from the plugin repository. In the registered Desktop app additionally test a real photo and model, configured-model errors, refusal, cancellation, reload while running, editing grams, explicit save and preserved records after reload. Tool-enabled apps also need actual allow/deny/revocation tests. Do not describe mock tests as a provider or signed-package pass.
