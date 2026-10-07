---
name: local-app
description: Use when the user asks about their Local Apps (small apps kept in a local data root), wants to find one, or asks what state one is in: its record, logs, checkpoints or background tasks. Covers which Local App tools exist and what they cannot do yet.
---

# Local App

A Local App is a small app that lives in a data root on this machine. The `local-app` MCP server reads that data
root. Its tools are named `LocalApp…`; your client may show them with a prefix for the plugin and the server.

## What you can do

These tools only read. None of them changes an app.

| Tool | Use it to |
| --- | --- |
| `LocalAppList` | Find apps and their ids. Arguments: `query`, `limit` (default 50, at most 100). When `has_more` is true, narrow the list with `query` instead of paging blindly. |
| `LocalAppGet` | Read one app: its record, runtime state, dependency install state and checkpoints. Needs `app_id`. |
| `LocalAppLogs` | Read the tail of one of an app's log files. Needs `app_id`; `log` names the file and `max_bytes` bounds the read. |
| `LocalAppCheckpointList` | List the code checkpoints of one app. Needs `app_id`. |
| `LocalAppBackgroundList` | List an app's background tasks and their last results. Needs `app_id`. |
| `LocalAppBackgroundStatus` | Read one background task. Needs `app_id` and `task_id`. |

Find the id with `LocalAppList` first, then pass that exact id to the others. Do not guess ids or build one from the
app's name.

## What you cannot do yet

This server does not create, build, run, edit, or inspect the screen of an app, and the tools for those are not
listed. If the user asks for one of those, say that this plugin cannot do it yet. Do not try to do it by writing files
into the data root by hand: the app store keeps records that hand-written files would not match, and the build needs a
verified toolchain that only the `local-app` command provides.

## When a tool fails

A tool that fails returns an error result with a message; read the message and tell the user what it says. If the
`local-app` tools are missing altogether, or the server will not start, use the `local-app-setup` skill.
