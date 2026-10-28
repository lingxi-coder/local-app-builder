# 餐盘笔记 — AG-UI 1.0 Local App example

This static Local App demonstrates a one-shot image run with the official AG-UI 1.0 client and a small set of app-scoped WebMCP tools. Copy this directory into a project and publish it with `LocalApp({"bundle_path":"/absolute/copied/project"})`. In trusted Apps UI, select a configured model that accepts image input and structured output. Provider credentials and model calls stay in the host.

Each recognition creates a fresh `threadId` and `runId`. The app converts the resized browser `File` with `localApp.agent.file(file)`, adds an official user `Message` containing text and image `ContentPart`s, and runs it with the standard `AbstractAgent.runAgent()` client. The optional schema, tool IDs, turn limit and timeout use only `forwardedProps['com.lingxi.localApp']`. The example requests no tools, so results remain model estimates rather than nutrition database measurements.

The host validates the requested output schema; `nutrition.mjs` also checks field/count bounds, calorie ranges, portions and honest provenance before rendering. Users can edit names and gram weights, then explicitly click Save. After the write succeeds, the editable draft closes; a failed write keeps it available for correction and retry. The record keeps original estimates and user edits, never photo bytes. `RUN_ERROR`, interrupt and cancellation outcomes stay visible; a cancellation request is not reported as success unless the host returns a successful terminal outcome. Page startup never runs an Agent request.

When the host's browser exposes official `document.modelContext.registerTool`, `webmcp.mjs` registers three tools using `registerTool(tool, {signal})`: read the current estimate and edits, read up to 20 recent saved meal summaries, and edit one portion in a completed but unsaved draft. The tools reuse this app's checked nutrition/range logic and bounded record validation. Portion edits are validated and stay in the visible draft; only the person can save. The API receives no photo, file bytes, arbitrary storage access, host controls, or permission to start paid recognition. A partial native API can register tools even if page-side discovery/execution is unavailable; no native API is replaced. The App reports registration availability separately while keeping its manual UI usable. The registration signal is released on a real page exit and retained across back/forward-cache restores to avoid duplicate registration.

## Files

- `index.html`, `styles.css`, `app.mjs`: camera/file input, AG-UI messages and events, result editing, app-local records, and feature-detected WebMCP registration.
- `nutrition.mjs`: JSON Schema, prompt and app-specific range/provenance validation.
- `webmcp.mjs`: strict tool input/output validation, bounded app-local record summaries, draft portion updates, and `document.modelContext` lifecycle handling.
- `lingxi-app.json`: schema-2 static entry with `agent` and `camera` capabilities and no host tool grants. `webmcp.mjs` is included in the app bundle files.

Mobile pages open in the OS-selected external browser, never an embedded WebView. The app feature-detects camera and file APIs without selecting a browser brand. On supported iOS 26+ hosts, a user-initiated task may receive a finite continued-processing lease; interruption or termination can still occur. The Mobile SDK preserves the exact AG-UI input by `runId` during foreground wake-and-return; ordinary refresh does not replay it. This short one-shot example does not persist its own run identity or business draft and therefore cannot restore an in-progress recognition after a page reload. For an interruption-safe App, follow the host Agent recovery guidance and the persistent Chat example. Browser permissions and site data remain under the browser's control. Source tests and standalone previews do not prove physical-device behavior or live-model output.

## Checks

From the builder repository:

```sh
node --test scripts/tests/test-desktop-calorie-agent.mjs
```

These tests cover nutrition validation, standard message shape and cancellation/error handling with a fake AG-UI client. They do not prove live model behavior. Device acceptance still needs a registered App, configured model, real photo, cancellation and refresh while a run is active.
