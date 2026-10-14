# local-app-builder

A LingXi plugin that helps the model build small local web apps. It plans the app with the user, scaffolds it from one of
five templates, implements it, builds it with pnpm, and opens the built `dist/index.html` in the user's desktop browser. The
generated app is a self-contained page: it uses standard web APIs only and keeps its data in that browser.

The plugin is skills and template assets only. It declares no agents, workflows, MCP servers or hooks, and its skills call no
host-specific tool: the model uses plan mode, `AskUserQuestion`, Write/Edit and Bash as for any other task.

The repository is public (`lingxi-coder/local-app-builder`) and licensed under the MIT License (see `LICENSE`).

## Install

The repository is a marketplace named `lingxi`, owned by LingXi, so the plugin's id is `local-app-builder@lingxi`.

- **Desktop:** add this repository as a marketplace (`lingxi-coder/local-app-builder`) and install `local-app-builder@lingxi`.
- **Android and iOS:** the LingXi apps carry this repository as the git submodule `resources/bundled-plugins/local-app-builder`
  of `lingxi-app`. They pack its `.lingxi-plugin/`, `plugins/` and `LICENSE` into the app and install the plugin when they
  start. On a phone, `create-local-app` still asks the user to create the app from LingXi on a computer.

The apps update an installed copy only when `plugins/local-app-builder/.lingxi-plugin/plugin.json` carries a new `version`,
so bump it with every change to the plugin.

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

A build must work when `dist/index.html` is opened from disk. Every template's `vite.config.mjs` therefore builds one classic
script (`format: "iife"`), rewrites the entry tag to `<script defer>`, uses relative URLs, and inlines every imported asset as a
data URL; `check-plugin.sh` fails if any of those settings is lost. The files the templates share are byte-identical, and the
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
