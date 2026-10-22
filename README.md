# local-app-builder

A LingXi plugin for creating and maintaining local applications. Desktop projects can use multiple files, Vite/TypeScript, Node/Python backends and declared native capabilities. Missing runtimes and dependencies are installed through Bash. The app's install, build and startup recipe is recorded in `lingxi-app.json`, then published with `LocalApp` into the desktop Apps library and opened in an independent window.

The built-in host skill supplies the current runtime contract. Android/iOS retain their document-only workflow; desktop does not inherit those restrictions. The bundled single-file templates remain optional building blocks. Existing design and browser-testing skills are reused, and the registered app is tested before delivery.

This plugin contains skills and template assets. It uses ordinary Bash/Read/Write/Edit tools plus the host's LocalApp tool, rather than embedding another agent runtime.

The repository is public (`lingxi-coder/local-app-builder`) and licensed under the MIT License (see `LICENSE`).

## Install

The repository is a marketplace named `lingxi`, owned by LingXi, so the plugin's id is `local-app-builder@lingxi`. No LingXi
app bundles the plugin. The Android and iOS apps add the `lingxi` marketplace when they first start (in mainland China from
a domestic mirror, once one exists) and install nothing; the user installs the plugin from Settings → Plugins. On a
computer, add this repository as a marketplace (`lingxi-coder/local-app-builder`) and install `local-app-builder@lingxi`.

On a phone, `create-local-app` sends simple apps to the built-in `local-app` skill and asks the user to create a
framework or game-engine app from LingXi on a computer: the pnpm build needs Node.js.

An installed copy updates only to a new `version` in `plugins/local-app-builder/.lingxi-plugin/plugin.json`, so bump it with
every change to the plugin.

## Layout

| path | what it is |
| --- | --- |
| `.lingxi-plugin/marketplace.json` | the marketplace `lingxi`; it lists this one plugin |
| `plugins/local-app-builder` | the plugin: `.lingxi-plugin/plugin.json`, `skills/`, `assets/templates/` |
| `plugins/local-app-builder/skills/create-local-app` | the flow: plan, scaffold, implement, build, deliver |
| `plugins/local-app-builder/skills/*` | the references the flow loads: design, accessibility, React, Ionic, and one per drawn-surface engine |
| `plugins/local-app-builder/assets/templates` | `catalog.json` and the five templates, each pinned to exact versions with a pnpm lockfile |
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

## History

Local Apps used to be a host feature. Rust crates in this repository (`local-apps`, `local-app-builder-service`, a CLI and a
plugin packer) backed the host tools the old skills called, a page bridge (`window.lingxi`) and an MCP widget; all of that is
gone. The four libraries the engine shared with it (`device-api`, `json-projection`, `mcp-wire`, `rooted-fs`) moved into the
engine repository, `harness-runtime`.

In October 2026 the engine regained a small built-in tier: the `LocalApp` tool and the `local-app` skill
(`harness-runtime/crates/local-app`), which publish one HTML document per app to an on-device library. With 2.1.0 this plugin
became its extension, and its builds became single-file so they publish the same way.
