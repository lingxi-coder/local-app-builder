# local-app-builder

A LingXi plugin for creating schema-2 Local Apps on Desktop, Android and iOS. Projects may use static frontends or approved Node/Python services when the native execution target supports them. Keep dependencies app-local, record the install/build/start recipe in `lingxi-app.json`, and publish through LocalApp into the device Apps library.

The built-in host skill supplies the current runtime contract. Desktop, Android and iOS use schema 2 with platform-specific execution targets and device capabilities. Create and Modify work in the App's bound source Project/Code Session, then publish and test a release. The bundled single-file templates remain optional building blocks. Existing design and browser-testing tools are reused.

This plugin contains skills and template assets. It uses ordinary Bash/Read/Write/Edit tools plus the host's LocalApp tool, rather than embedding another agent runtime.

Hosts that expose `window.localApp.agent` use AG-UI 1.0 `RunAgentInput`, `Message`, `ContentPart`, `BaseEvent` and the official JavaScript client. A host helper turns a browser `File` into a standard content part. Model selection, provider credentials, host tools and approvals stay in trusted LingXi UI; the chat that created an App is not the App's Agent context.

Interactive Apps also expose selected business operations through official WebMCP `document.modelContext`. The existing create-local-app skill covers registration, input validation, lifecycle and discovery/call checks for both Create and Modify. Host injection supplies the pinned polyfill on Desktop and Mobile static pages; Mobile server pages explicitly load `/__host/webmcp.js` before App scripts. Browser integrations remain user-installed choices in Public settings. No additional Skill, MCP server or global debugging port is bundled here.

The repository is public (`lingxi-coder/local-app-builder`) and licensed under the MIT License (see `LICENSE`).

## Install

The repository is a marketplace named `lingxi`, owned by LingXi, so the plugin's id is `local-app-builder@lingxi`. No LingXi
app bundles the plugin. The Android and iOS apps add the `lingxi` marketplace when they first start (in mainland China from
a domestic mirror, once one exists) and install nothing; the user installs the plugin from Settings → Plugins. On a
computer, add this repository as a marketplace (`lingxi-coder/local-app-builder`) and install `local-app-builder@lingxi`.

On a phone, static apps use the web target. Framework builds and Node/Python services require the managed Mobile Linux runtime and exact-release consent. Dependencies stay app-local. Agent runs follow OS lifecycle limits: Android keeps a user-visible runtime notification and iOS uses a finite foreground continuation lease; neither is an always-on background worker.

Version 6.1.0 adds WebMCP authoring and verification to Create and Modify, retaining AG-UI 1.0 inputs, messages, events and official client semantics.
Host-specific options live only in `forwardedProps['com.lingxi.localApp']`; they are not protocol fields or provider APIs.
An installed copy updates only when the
`version` in `plugins/local-app-builder/.lingxi-plugin/plugin.json` changes, so bump it with every plugin release.

## Layout

| path | what it is |
| --- | --- |
| `.lingxi-plugin/marketplace.json` | the marketplace `lingxi`; it lists this one plugin |
| `plugins/local-app-builder` | the plugin: `.lingxi-plugin/plugin.json`, `skills/`, `assets/templates/` |
| `plugins/local-app-builder/skills/create-local-app` | source binding, Create/Modify, WebMCP business tools, build, publish and verify |
| `plugins/local-app-builder/skills/*` | the references the flow loads: design, accessibility, React, Ionic, and one per drawn-surface engine |
| `plugins/local-app-builder/assets/templates` | `catalog.json` and the five templates, each pinned to exact versions with a pnpm lockfile |
| `plugins/local-app-builder/assets/examples/desktop-calorie-agent` | camera/choose-photo AG-UI run with estimated calorie ranges, editable portions and explicit app-local save |
| `plugins/local-app-builder/assets/examples/attachment-chat-skill` | persistent AG-UI thread, user-selected file parts and a packaged Skill with no host tool grants |
| `scripts/checks` | the gates; `scripts/check-all.sh` runs every one of them |
| `scripts/tests` | the gates' negative probes (`test_plugin_gates.py`) and the template library tests (`test-local-app-r4-libs.mjs`) |
| `docs/local-apps` | design history (Chinese) from when Local Apps were a host feature; it describes code that no longer exists |

## The templates

`react-dom` is a routed Ionic app; `canvas-2d`, `three-3d`, `phaser-2d` and `babylon-3d` are one drawn surface with Ionic
overlays. All five are React 19, Ionic 9 and Vite 8, and they detect the device (OS, form factor, input mode, safe areas,
appearance) from the browser.

A build is ONE file, `dist/index.html`, that works when it is opened from disk and when `LocalApp` publishes it alone. Every
template's `vite.config.mjs` therefore builds one classic script (`format: "iife"`), inlines it at the end of `<body>`,
inlines every imported asset as a data URL, uses relative URLs and copies nothing from `public/`; `check-plugin.sh` fails if
any of those settings is lost, and CI fails a build that writes a second file. The files the templates share are byte-identical, and the
gate holds them to that.

To try a template by hand, copy it out first (the gate rejects build output inside the plugin):

    cp -R plugins/local-app-builder/assets/templates/react-dom/r4 /tmp/try-react-dom
    cd /tmp/try-react-dom
    npm exec --yes --package=pnpm@12.5.1 -- pnpm install --frozen-lockfile
    npm exec --yes --package=pnpm@12.5.1 -- pnpm build
    open dist/index.html

## Checks

    ./scripts/check-all.sh
    node --test scripts/tests/test-local-app-r4-libs.mjs
    node --test scripts/tests/test-desktop-calorie-agent.mjs
    node --test scripts/tests/test-attachment-chat-recovery.mjs

## History

Local Apps used to be a host feature. Rust crates in this repository (`local-apps`, `local-app-builder-service`, a CLI and a
plugin packer) backed the host tools the old skills called, a page bridge (`window.lingxi`) and an MCP widget; all of that is
gone. The four libraries the engine shared with it (`device-api`, `json-projection`, `mcp-wire`, `rooted-fs`) moved into the
engine repository, `harness-runtime`.

In October 2026 the engine regained a small built-in tier: the `LocalApp` tool and the `local-app` skill
(`harness-runtime/crates/local-app`), which publish one HTML document per app to an on-device library. With 2.1.0 this plugin
became its extension, and its builds became single-file so they publish the same way.
