---
name: create-local-app
description: Build and verify schema-2 local applications for Desktop and mobile, including independent host Agent tasks and approved local services.
---

# Create or modify a local app

Load the host's built-in `local-app` skill first when available. Its tool schema and host capability contract are authoritative. Desktop, Android and iOS share manifest schema 2. Execution targets and native capabilities remain platform-specific.

## Schema-2 project flow

When `LocalApp` advertises `bundle_path`, use the project flow. A local app may be a static page, a multi-file Vite/TypeScript project, or a frontend with Node/Python/local-command backends. Network access, modules, workers and declared native capabilities are supported. Do not impose single-HTML or offline-only requirements when the host supports projects.

1. Converge the user's requirements. If they already gave a clear request or approved a plan, proceed without repeating approval. Use the installed frontend-design, accessibility, React and engine skills when relevant.
2. Inspect the current host with its permitted shell tool (Bash or Shell). Use project-local npm/pnpm/Bun dependencies, Python virtual environments and lockfiles. Do not modify a shared Mobile Linux toolchain; if its managed execution runtime is missing, use the native runtime settings. Follow existing host permission mechanisms.
3. Use an existing project or an appropriate scaffold. The bundled catalog at `${LINGXI_SKILL_DIR}/../../assets/templates/catalog.json` is optional. Its five templates remain useful for single-file artifacts, but projects may choose other frameworks and ordinary multi-file builds.
4. Write schema-2 `lingxi-app.json` using the host skill's manifest contract: explicit targets, package source allowlist/lockfiles and data compatibility. Record repeatable install/build steps, process commands, virtual environments, dependencies between services, and readiness checks. Servers bind to `127.0.0.1` and read their assigned `PORT`. Keep durable data in `LOCAL_APP_DATA_DIR` or browser storage.
5. Run the project and verify it with existing browser tools/skills. Test the user's important interactions, reload/storage behavior and errors. For a backend, also verify its readiness endpoint and logs.
6. Call `LocalApp` with `bundle_path`. Updating uses the same app `id` plus the latest `expectedRevision` from `read`. Call `start`, inspect `status` and `logs`, and test the registered app in its actual platform container (Desktop window; external Safari/Chrome on Mobile). Deliver the app card and its entry in Apps; a source path or a temporary development server alone is not delivery.

First publication does not grant permissions. The native Apps host asks for requested capabilities and exact-release execution consent. A later launch reuses its recorded install/start recipe without a creating-chat model. New code digests do not inherit existing capability grants. Desktop closing normally stops its application task. Mobile application pages open in the external system browser, never an embedded WebView. Closing the native management page does not close a browser tab or stop its run: use the explicit Stop action. Android uses a user-visible running notification; iOS has only its finite system background budget and explicitly pauses on expiry. Do not promise an always-running local server. Quitting LingXi stops local app processes.

## Apps that call an Agent

When an app needs image recognition, structured model output or authorized tool use, read `${LINGXI_SKILL_DIR}/references/host-agent.md`. Use `window.localApp.agent` only when the current host skill advertises it. The host runs an independent task with its selected model and explicit tool grants; the app must not embed provider keys or reuse the chat that created it. Do not use Remote Control as a same-device Agent API.

When authoring a tool-enabled app, call `LocalApp({"action":"agent_catalog"})` from the creating chat to discover public model and opaque tool IDs. This read-only catalog requires no app ID, contains no connection configuration or credentials, and grants no permission. Do not guess MCP IDs; the app runtime still exposes only `agent.run/get/subscribe/cancel`.

Start from `${LINGXI_SKILL_DIR}/../../assets/examples/desktop-calorie-agent` for a camera/choose-photo → structured result → editable portions → explicit save example. Copy it into a new project before editing; publish the complete directory with `bundle_path`. This schema-2 example supports Desktop camera/file input and mobile external-browser camera/photos; it intentionally has no business tools. Its nutrition estimates and recording logic belong to the app, not to the generic host API.

Persist a request ID before submitting, preserve it on explicit uncertain retries, and reconnect accepted tasks using `get`/`subscribe` after reload. Reload must not automatically resubmit an unconfirmed or rejected request. Let the user explicitly discard its recovery record before replacing input, with a clear notice that this does not cancel a possibly accepted task. Handle all terminal states, failed capability checks, denial and cancellation. Display only validated structured output on `succeeded`. Keep approval and questions in the trusted host UI. Test both the registered app and actual configured model; unit tests and standalone browser rendering are not end-to-end proof.

## Mobile targets and standalone documents

Static schema-2 projects may target mobile `web` without a Linux service. Node/Python/process projects require the available Mobile Linux execution environment, compatible architecture/channel and native exact-release consent. Do not assume Desktop executables, arbitrary host paths or undeclared device APIs exist on a phone. Use `localApp.serviceUrl/serviceSocketUrl` for registered app services rather than hardcoding ports.

Mobile pages run in the external system browser, not an Android WebView/iOS WKWebView. Load the host SDK before application calls: `if (!globalThis.localApp) await import('/__local_app_sdk.js')` from the app's main module. Desktop retains its preload SDK. `localApp.camera.capture()` and `photos.pick()` use browser file inputs and return normalized `{mimeType,base64}`; browser/OS permissions are managed by Safari/Chrome, not LingXi's former native device toggles. Convert to the Agent image DTO before sending (complete request limit: 24 MiB).

Only `agent.run/get/cancel` cross the browser-to-host boundary; subscribe polls get. Identity comes from the host's exact app/run route, not page-owned IDs. App pages cannot configure models/grants or approve tools. Use `localApp.host.open()` from an explicit user action to return to trusted LingXi approvals, then let the user return to the browser. Display connection failures as host unavailable/paused, preserve drafts and never silently replay a task. On iOS, a camera picker or Safari can outlive LingXi's finite background budget: return to LingXi to resume and explicitly reopen the fresh run. Do not claim indefinite Mobile background Agent execution. Browser storage belongs to the external browser; stopping/removing an app does not clear that storage.

For a simple static document, the current host may wrap `html`/`file_path` into schema 2. Agent/device declarations need a project manifest. No schema-1 manifest or old mobile document runtime is generated. Without LocalApp, a standalone browser page is a preview or browser deliverable and must be described accordingly.

## Modify and recover

Use LocalApp list/read to locate the app, edit its authoring source, test, and republish to the same id with expectedRevision. If authoring source is unavailable, use native Customize/Remix; never edit the immutable release snapshot. Preserve data and explain application data migrations when needed. Use status/logs to diagnose installation or startup failures, repair with Bash, and republish. Do not clear application data as a repair step.
