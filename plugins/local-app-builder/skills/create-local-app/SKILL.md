---
name: create-local-app
description: Create or modify schema-2 Local Apps in their source Project, expose business tools through WebMCP, and publish tested releases with optional host Agent runs.
---

# Create or modify a Local App

Load the host's built-in `local-app` skill first when available. Its current tool schema, execution targets and capability contract are authoritative. Use this existing workflow for both creation and Modify; no extra WebMCP plugin or Skill is required to author an App's tools.

## Choose the authoring source

- **Create:** work in the source Project and ordinary Code Session prepared by the native Apps flow. Otherwise use the user's selected project or a distinct directory in the current host's authoring root. Let the host create App/workspace/session identities. Mobile paths belong to the current brand's Linux user; obtain them from the host, never hardcode another brand's home.
- **Modify:** `LocalApp` list/read identifies the App, current revision and authoring binding. Resume its saved source Project/Code Session before editing. Preserve the App ID, business data and existing tool contracts. If the source is missing, use native restore/Customize/Remix; never edit an immutable release or silently move development to an unrelated chat. An App's runtime Agent Chat is separate from its authoring Session.

Proceed when the request is clear; ask only for missing decisions that affect the result. Reuse installed design, accessibility or engine guidance when relevant. A framework change, data reset or extra integration is not implied by a small Modify request.

## Build and publish

1. Inspect the host using its permitted shell tool. Use the simplest project that fits: static HTML, a multi-file frontend, or approved services. The optional templates at `${LINGXI_SKILL_DIR}/../../assets/templates/catalog.json` are starting points, not a single-file or offline-only constraint. Copy examples/templates into the authoring source before editing. Keep packages, lockfiles and Python virtual environments project-local; use native runtime settings if managed Mobile Linux is unavailable.
2. Maintain `lingxi-app.json` schema 2 with actual targets, necessary capabilities, package files/lockfiles, data compatibility and repeatable install/build/start recipes. Services bind `127.0.0.1` and their assigned `PORT`; durable backend records use `LOCAL_APP_DATA_DIR`. Include every new WebMCP/Agent source module in the package and build output. Do not invent manifest capabilities.
3. Implement UI and business operations together. For interactive Apps, expose a small, useful set of those operations through standard `document.modelContext`; read [WebMCP authoring and verification](references/webmcp.md). Modify the registrations alongside changed business behavior. A display-only document need not invent actions; explain when no useful tool applies.
4. Run the build and important user flows. Check reload/persistence, invalid input and errors. Verify WebMCP discovery and execution against the same business state as the UI, including rejected inputs and registration cleanup. Use an already available browser tool/plugin where supported; inability to connect it is an explicit untested item, not a reason to install or enable one silently.
5. Publish with `LocalApp({"bundle_path":"/absolute/authoring/source"})`. For Modify, send the same `id` and the revision observed before the edits as `expectedRevision`. On conflict, inspect and merge the competing changes before retrying; fetching a revision just to overwrite it is incorrect. A complete simple document may use the host's `html`/`file_path` shorthand.
6. Start the registered release and inspect `status`/`logs`. Test the actual Desktop App window or Mobile OS-selected external browser, especially host script loading, WebMCP discovery/calls and optional Agent runs. Native UI handles exact-release execution/capability consent. Deliver the App card/library entry and report what passed, what requires user setup, and what remains untested. A source directory, temporary dev server or successful build alone is not a delivered/accepted App.

## When the App calls an Agent

Read [the host Agent contract](references/host-agent.md) for AG-UI 1.0 messages, generic File parts, structured output, tools/Skills, independent runs, persistent Chat and recovery. Generate Agent calls only with the host's official SDK and standard AG-UI `RunAgentInput`, `Message`, `BaseEvent` and Schema contract; do not invent another request or event format. Provider credentials, model selection, approvals and questions stay with LingXi. WebMCP exposes **App operations to an Agent**; AG-UI lets **the App request Agent work**. They have different permission boundaries.

For both Create and Modify, start work only from a real user action. Before submission preserve the immutable Agent `runId`, `threadId`, exact input and needed business draft where storage is available. Restore an existing run with same-ID read-only task/event/Chat queries; never automatically resend it or repeat its side effects. Distinguish pending, running, needs-user-return, completed, interrupted and unknown states in the UI, and report completion only from a successful terminal event and validated result. The reference explains explicit retries, attachment privacy and storage failures.

Chat creation also needs a durable intent: `chats.create({requestId, title?})` requires a browser-generated lowercase UUIDv4. Save that ID and the exact original options before submission. Restore with read-only `chats.creationStatus` or inspect `chats.pendingCreations`; a missing result does not prove creation failed. Never create on refresh or automatically retry an ambiguous response. Only an explicit user retry may reuse the same ID and unchanged input. If either is missing, inspect saved Chats and require review before a separate new creation; do not invent a replacement ID or support the old no-ID API.

In `chats.list`, `unavailableChats` covers the current page; `hasUncertainChats` signals App-wide uncertainty on every page. Continue pagination through empty `chats`, retain any uncertainty signal, and never treat omitted fields as proof of healthy or nonexistent data. Preserve unavailable Chat IDs/input, show status guidance and offer read-only checks. Do not automatically replay work, create replacements, or switch Chats to hide an unavailable result.

If the host reports that existing Agent/Chat storage has an unsupported, missing or unverifiable format, show it as unavailable and preserve the existing data. Never automatically delete, reset or rebuild the store; direct the user to inspect its state in LingXi without promising migration.

Verify duplicate taps, interrupted or unavailable host reads, a transient `task_not_found`, page reload, storage failures and a user-chosen retry with the original input. Include ambiguous Chat creation, status `null`, absent/invalid request IDs, exact title omission, and same-ID retry; refresh must never issue a creation POST. Check that recovery alone starts no run and repeats no tool effect. Keep simulated/browser checks distinct from signed builds and physical-device tests of the actual OS handoff and expiration.

Start with `${LINGXI_SKILL_DIR}/../../assets/examples/desktop-calorie-agent` for photo recognition, validated estimates and explicit save, including WebMCP access to App state. For persistent file-backed conversations use `${LINGXI_SKILL_DIR}/../../assets/examples/attachment-chat-skill`. Keep the user's file confirmation, cancellation and business validation. A WebMCP call must not silently upload a selected file, start a paid model request or approve host tools.

## Platform and data rules

Desktop retains its App windows. Mobile uses the actual OS-selected external browser, including OEM/third-party browsers; never embed WebView/WKWebView or choose a browser package. Feature-detect files, camera, storage and recovery. Preserve navigation and drafts on photo/file return. Load the Mobile host SDK before App calls using `if (!globalThis.localApp) await import('/__local_app_sdk.js')`; this is separate from WebMCP loading described in its reference.

Mobile execution is finite: Android uses its running notification. On supported iOS 26+ hosts, native code may request system-managed continued processing for a user-initiated task; it can fail, expire or be terminated. Let the SDK and native UI handle foreground handoff and authorization; page code cannot create a background lease. After process death, expiration, device restart or force-quit, do not promise immediate wakeup or fabricated success: show the last confirmed or unknown state and how to return and check. Do not impose iOS-specific behavior on Android or Desktop. Browser data belongs to the chosen browser. Stop/removal does not clear that site data, and a release change must not erase business records. Data-schema changes follow the host's verified migration contract.

If `LocalApp` is unavailable, deliver a clearly labeled source/browser preview. Host-only Agent/WebMCP injection and physical-device behavior remain unverified.
