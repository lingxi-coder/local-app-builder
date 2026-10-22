---
name: create-local-app
description: Build and verify local applications; on desktop install dependencies, run frontends and local backends, and publish into LingXi Apps with LocalApp.
---

# Create or modify a local app

Load the host's built-in `local-app` skill first when available. Its tool schema and host capability contract are authoritative. Desktop and mobile use different application runtimes.

## Desktop project flow

When `LocalApp` advertises `bundle_path`, use the Desktop project flow. A local app may be a static page, a multi-file Vite/TypeScript project, or a frontend with Node/Python/local-command backends. Network access, modules, workers and declared native capabilities are supported. Do not impose single-HTML or offline-only requirements.

1. Converge the user's requirements. If they already gave a clear request or approved a plan, proceed without repeating approval. Use the installed frontend-design, accessibility, React and engine skills when relevant.
2. Inspect the environment using Bash. Install missing Node, Python, package managers, uv, Vite, TypeScript and other required dependencies, then continue. Use an existing compatible runtime when available. Use project-local npm/pnpm/Bun dependencies, Python virtual environments and lockfiles. Follow the ordinary Bash permission mechanism; do not invent another Local App installation confirmation.
3. Use an existing project or an appropriate scaffold. The bundled catalog at `${LINGXI_SKILL_DIR}/../../assets/templates/catalog.json` is optional. Its five templates remain useful for single-file artifacts, but desktop projects may choose other frameworks and ordinary multi-file builds.
4. Write `lingxi-app.json` using the host skill's manifest contract. Record repeatable install/build steps, process commands, virtual environments, dependencies between services, and readiness checks. Servers bind to `127.0.0.1` and read their assigned `PORT`. Keep durable data in `LOCAL_APP_DATA_DIR` or browser storage.
5. Run the project and verify it with existing browser tools/skills. Test the user's important interactions, reload/storage behavior and errors. For a backend, also verify its readiness endpoint and logs.
6. Call `LocalApp` with `bundle_path`. Updating uses the same app `id`. Call `start`, inspect `status` and `logs`, and test the registered independent window. Deliver the app card and its entry in Apps; a source path or a temporary development server alone is not delivery.

First publication does not grant permissions. The desktop host asks once for requested application capabilities. A later launch reuses its recorded install/start recipe without a model. Closing normally stops it; the user may enable background running in Apps. Quitting LingXi stops all app processes.

## Mobile and standalone documents

A host whose LocalApp schema only accepts `html`/`file_path` uses its document-only skill and limits. For a framework build destined for that host, the existing templates produce `dist/index.html`; publish that file. Never offer desktop native/process APIs on mobile. Without a LocalApp tool, a user-requested standalone web page may open in the browser, and must be described as such.

## Modify and recover

Use LocalApp list/read to locate the app, edit its source, test, and republish to the same id. Preserve data and explain application data migrations when needed. Use status/logs to diagnose installation or startup failures, repair with Bash, and republish. Do not clear application data as a repair step.
