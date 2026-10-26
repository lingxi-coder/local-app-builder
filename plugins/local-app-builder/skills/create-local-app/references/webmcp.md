# WebMCP in Local App creation and Modify

Read this while implementing or changing an interactive App's business operations. Use the [WebMCP draft](https://webmachinelearning.github.io/webmcp/) and the host-pinned [official polyfill](https://github.com/w3c-oss/webmcp-polyfill/tree/f629c99458514784b9104864e5535245ca7ac6b0) API: `document.modelContext.registerTool`, `getTools`, `executeTool`, and AbortSignals. Do not generate a `navigator.modelContext` alias, custom MCP transport or a new browser automation framework.

## Load before registration

| Entry | Host behavior | Author action |
| --- | --- | --- |
| Desktop Local App | Injects the official polyfill before App scripts, preserving any native implementation | Register tools after business state is ready |
| Mobile static HTML | Injects the current run's `/__run/<runId>/__host/webmcp.js` before App scripts | Register after state initialization; never persist or invent the run ID |
| Mobile server-backed HTML | Proxies HTML without rewriting it | Add `<script src="/__host/webmcp.js"></script>` in the document before App scripts |
| Standalone development preview | Host injection is absent | Use a permitted official-polyfill setup on HTTPS/localhost if needed; record that this is preview coverage |

The Mobile server script alias resolves only from a current-run, same-origin document. Keep the script same-origin and preserve its Referer; do not use `no-referrer`. Desktop uses `/__lingxi_webmcp.js`. For server pages targeting both hosts, load the Mobile fallback only if `document.modelContext` is absent, await its load, then start App modules. Keep preview handling explicit: that host route does not exist on an ordinary dev server.

Feature-detect the methods actually needed. A missing API or registration failure must leave the ordinary UI usable and report automation as unavailable. Never replace a partial native implementation. WebMCP registration needs no new manifest capability and grants no host Agent/tool/file permission.

## Expose business operations

Choose stable, task-oriented names, clear descriptions and a JSON Schema for each input. A small App might expose its current public state, search its own records and edit an unsaved draft. Reuse the same store/service functions, validation and UI update path as the visible controls. Tool callbacks must enforce types, bounds and unknown-field rules; the pinned polyfill does not enforce `inputSchema` for them. Return compact JSON-serializable business values, with failures clearly rejected or explicitly represented.

Do not expose arbitrary JavaScript, DOM selectors, filesystem paths, raw browser storage, credentials, native approvals or all host tools. Limit returned data to what the task needs. Do not return selected File/photo bytes merely because a caller discovered the tool. `readOnlyHint` and other annotations describe behavior; they are not authorization. Sending, deleting, purchasing or invoking a paid/file-bearing Agent run must follow the App's visible confirmation path and actual host/browser permissions. A tool description or `confirmed:true` argument cannot replace that decision.

Example: register a read-only business view after the App has initialized its state. Here `getPublicSummary()` is the same application function the UI uses, and `showAutomationUnavailable()` reports status without disabling manual use:

```js
const context = document.modelContext;
const registration = new AbortController();
if (typeof context?.registerTool === 'function') {
  try {
    await context.registerTool({
      name: 'app.get_summary',
      description: 'Read the current App summary without files or private settings.',
      inputSchema: {type: 'object', properties: {}, additionalProperties: false},
      annotations: {readOnlyHint: true},
      execute(input, {signal}) {
        signal.throwIfAborted();
        if (!input || typeof input !== 'object' || Array.isArray(input) || Object.keys(input).length) {
          throw new TypeError('Expected an empty object.');
        }
        return getPublicSummary();
      }
    }, {signal: registration.signal});
  } catch (failure) {
    registration.abort();
    showAutomationUnavailable(failure);
  }
} else {
  showAutomationUnavailable(new Error('WebMCP is unavailable in this browser context.'));
}
// Abort on component/unmount or explicit App teardown. In React, create a new
// controller per effect and return () => registration.abort() as its cleanup.
```

Keep registrations synchronized with actual readiness. Observe async registration errors; if a batch partially fails, abort its controller. React StrictMode/remount and hot reload must not accumulate names. When aborting on `pagehide`, recreate registrations on persisted `pageshow` so back/forward-cache restoration works. Do not unregister only because the tab is backgrounded. Long async callbacks observe their execution signal and avoid committing changes after cancellation.

## Modify and verify

Read the existing source, registrations and saved release revision first. Extend the existing business API rather than registering the same name in another component. Keep unchanged names/inputs stable; update the callback, Schema and tests together when behavior changes. Preserve current App data and the host authoring Project/Code Session. Publish the same App ID with the observed revision after source verification.

Use the browser/plugin that the user already installed and authorized, when it can reach the test page. Public settings entries help the user install/configure those tools. Installation alone does not make a Desktop App window or every external browser reachable. Do not enable a global debugging port, grant new cross-origin exposure or install an extension as a hidden build step.

Verify these observable behaviors before claiming WebMCP acceptance:

1. After App readiness, `await document.modelContext.getTools()` contains the expected names and current input Schemas. Discover the descriptor for the intended document/origin; do not identify a tool by name alone across frames.
2. Call a harmless operation with `await document.modelContext.executeTool(descriptor, input)`. The pinned implementation returns a JSON-serialized result: parse it, compare it to the UI/business state and check state again after an allowed edit.
3. Invalid/extra arguments are rejected without changing state. Test denial or cancellation for operations that need them; inspect actual effects, not just annotation text.
4. Repeat after reload and component/navigation lifecycle changes. Verify no duplicate/stale registrations and that a cancelled async call cannot commit later.
5. Repeat on the published App, with packaged modules and host script loading. Distinguish source/mock tests, real-browser local calls and an external Agent's invocation through its installed MCP/plugin; report the layers that actually ran.

If the browser lacks discovery/execution support, record that boundary and retain manual UI. If no installed browser tool can reach the page, complete available checks and explain how the user can choose an integration in Public settings. Do not label a stand-in object, a file preview or tool registration alone as successful external Agent invocation.
