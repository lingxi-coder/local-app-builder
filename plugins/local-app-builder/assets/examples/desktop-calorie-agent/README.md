# 餐盘笔记 — schema-2 Local App Agent example

This dependency-free static Local App demonstrates the current task SDK. Copy the directory into a project and publish it with `LocalApp({"bundle_path":"/absolute/copied/project"})`. In the trusted Apps UI, enable the App Agent and select a configured model that supports image input and structured output. The app uses the browser's standard file picker or camera API; provider credentials and model calls stay in the host.

The selected photo is resized to a JPEG `File` before the user presses Recognize. The app sends it in `attachments` with a schema for food candidates, estimated grams, calorie ranges, assumptions and questions. It grants no tools, so results remain model estimates rather than nutrition database measurements. Users can correct names and gram weights before explicitly saving. Saved meal records keep original estimates and edits, never photo bytes.

`agent-client.mjs` saves a temporary app draft and its `File` in revision-scoped IndexedDB before `tasks.start`; that copy expires after 24 hours and is cleared on definitive host rejection. The browser SDK also preserves the exact request during the iOS foreground wake-and-return handoff. It submits only after the native host returns with the matching request ID readiness marker. A plain reload never resubmits; the app checks `tasks.list` for the same `requestId` and reconnects to an admitted task. If no task exists, the user can explicitly retry the unchanged draft while it remains saved. The SDK's separate ledger keeps pending data at most 24 hours and accepted idempotency receipts for 7 days, with 32-record/100 MiB caps. Page Promises are not treated as durable.

The sample cleans up the temporary photo after task admission and never adds it to saved meal history. Task result output is validated by both the host schema and app-specific range/provenance checks. Failures, interruption and cancellation remain visible; none are automatically rerun. Text renders through text nodes.

## Files

- `index.html`, `styles.css`, `app.mjs`: file/camera input, task progress and cancellation, result editing, and app-local records.
- `agent-client.mjs`: revision-scoped IndexedDB draft recovery and durable task reconnect.
- `nutrition.mjs`: JSON Schema and app-specific range/provenance checks.
- `lingxi-app.json`: schema-2 static entry with `agent` and `camera` capabilities and no tool grants. Camera access still uses the browser's and OS's capture permission prompts.

Mobile pages open in the system-selected external browser, never an embedded WebView. The app feature-detects browser camera and file APIs; it does not select Safari or Chrome by name. Browser permissions and browser site data remain under the browser's control. `localApp.host.open()` returns to trusted approvals/settings and does not expose a page approval API. Android uses a user-visible runtime notification. iOS execution has a finite foreground continuation lease; do not describe it as an always-on background service. Source tests and standalone previews do not prove physical-device behavior.

## Checks

From the builder repository:

```sh
node --test scripts/tests/test-desktop-calorie-agent.mjs
```

These tests cover app-side nutrition validation and durable task recovery with a fake host. They do not prove live model behavior. Device acceptance still needs a registered App, configured model, real photo, cancellation and refresh while a task runs.
