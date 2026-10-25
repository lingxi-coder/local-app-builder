# 餐盘笔记 — AG-UI 1.0 Local App example

This static Local App demonstrates a one-shot image run with the official AG-UI 1.0 client. Copy this directory into a project and publish it with `LocalApp({"bundle_path":"/absolute/copied/project"})`. In trusted Apps UI, select a configured model that accepts image input and structured output. Provider credentials and model calls stay in the host.

Each recognition creates a fresh `threadId` and `runId`. The app converts the resized browser `File` with `localApp.agent.file(file)`, adds an official user `Message` containing text and image `ContentPart`s, and runs it with the standard `AbstractAgent.runAgent()` client. The optional schema, tool IDs, turn limit and timeout use only `forwardedProps['com.lingxi.localApp']`. The example requests no tools, so results remain model estimates rather than nutrition database measurements.

The host validates the requested output schema; `nutrition.mjs` also checks calorie ranges, portions and honest provenance before rendering. Users can edit names and gram weights, then explicitly click Save. The record keeps original estimates and user edits, never photo bytes. `RUN_ERROR`, interrupt and cancellation outcomes stay visible; a cancellation request is not reported as success unless the host returns a successful terminal outcome. Page startup never runs an Agent request.

## Files

- `index.html`, `styles.css`, `app.mjs`: camera/file input, AG-UI messages and events, result editing, and app-local records.
- `nutrition.mjs`: JSON Schema, prompt and app-specific range/provenance validation.
- `lingxi-app.json`: schema-2 static entry with `agent` and `camera` capabilities and no host tool grants.

Mobile pages open in the OS-selected external browser, never an embedded WebView. The app feature-detects camera and file APIs without selecting a browser brand. iOS uses a finite foreground continuation lease. The host SDK preserves the exact AG-UI input by `runId` during its foreground wake-and-return handoff; ordinary refresh does not replay it. Browser permissions and site data remain under the browser's control. Source tests and standalone previews do not prove physical-device behavior or live-model output.

## Checks

From the builder repository:

```sh
node --test scripts/tests/test-desktop-calorie-agent.mjs
```

These tests cover nutrition validation, standard message shape and cancellation/error handling with a fake AG-UI client. They do not prove live model behavior. Device acceptance still needs a registered App, configured model, real photo, cancellation and refresh while a run is active.
