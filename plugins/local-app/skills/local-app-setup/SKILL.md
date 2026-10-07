---
name: local-app-setup
description: Use when the Local App tools are missing or the local-app MCP server fails to start, or when the user asks to install or check the local-app command, its data root or its Node and pnpm toolchain.
---

# Local App setup

The plugin starts the `local-app` command (`local-app mcp`). It is installed separately and must be on the `PATH` of
the program that runs the MCP server. Nothing in this plugin installs it.

## Check the machine

Run these in a terminal. They change nothing except `toolchain install`, which downloads the toolchain.

```text
local-app version
local-app doctor
local-app toolchain status
```

- `local-app: command not found` means the command is not installed, or is not on the `PATH` the MCP client uses.
  Tell the user; do not try to install it yourself.
- `doctor` lists what it found: the data root and whether it is writable, and whether the toolchain is installed.
  A line that says `fail` stops everything; a line that says `warn` stops only the features that need it.
- `toolchain status` says whether the pinned Node and pnpm are installed and verified. Builds use that copy and never
  the Node or pnpm on `PATH`. `local-app toolchain install` downloads and verifies it (about 80 MB); on a machine with
  no network, `local-app toolchain install --from DIR` takes the same archives from a directory, and `status` lists
  their file names and checksums. Ask the user before running `install`.

## The data root

Apps live in one data root. It is chosen by `--data-root DIR`, then the `LOCAL_APP_DATA_ROOT` environment variable,
then the platform default (on macOS, `~/Library/Application Support/local-app`). The path must be absolute. Codex and
Claude Code each start their own `local-app mcp` process; both read the same data root.

## What the plugin needs

- macOS (Apple silicon or Intel) for the toolchain; the read-only tools only need the data root.
- The `local-app` command on the `PATH` the MCP client uses.
