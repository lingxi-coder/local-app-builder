---
name: local-app
description: Use when the user asks about their Local Apps (small apps kept in a local data root): to find one, read its state, logs or checkpoints, or create and build one. Lists which Local App tools exist and what they cannot do yet.
---

# Local App

A Local App is a small app that lives in a data root on this machine. The `local-app` MCP server works on that data
root. Its tools are named `LocalApp…`; your client may show them with a prefix for the plugin and the server. The
server's instructions name the data root.

## Reading

These tools only read.

| Tool | Use it to |
| --- | --- |
| `LocalAppList` | Find apps and their ids. Arguments: `query`, `limit` (default 50, at most 100). When `has_more` is true, narrow the list with `query` instead of paging blindly. |
| `LocalAppGet` | Read one app: its record, runtime state, dependency install state and checkpoints. Needs `app_id`. |
| `LocalAppLogs` | Read the tail of one of an app's log files. Needs `app_id`; `log` names the file (`build` is the build log) and `max_bytes` bounds the read. |
| `LocalAppCheckpointList` | List the code checkpoints of one app. Needs `app_id`. |
| `LocalAppBackgroundList` | List an app's background tasks and their last results. Needs `app_id`. |
| `LocalAppBackgroundStatus` | Read one background task. Needs `app_id` and `task_id`. |
| `LocalAppTemplateCatalog` | List the templates an app can start from, with what each is for. Read it before choosing a `template_id`. |

Find the id with `LocalAppList` first, then pass that exact id to the others. Do not guess ids or build one from the
app's name.

## Making an app

These tools change things. The steps and the plan file are in the `local-app-create` skill.

| Tool | Use it to |
| --- | --- |
| `LocalAppCreate` | Make an empty app from a `brief` (and optionally a `name`). It has no source yet. |
| `LocalAppPrepare` | Turn a plan the person approves into a prepared workspace. The person is asked, through the client, to approve the plan. |
| `LocalAppInstallDeps` | Install the app's dependencies, with the pinned toolchain. |
| `LocalAppBuild` | Build the app, with the pinned toolchain, offline. |
| `LocalAppManifest` | Declare the app's data collections, allowed network domains and capabilities. |
| `LocalAppConfirmDependencyChange` | Propose adding, updating or removing an npm dependency. The person is asked to approve it, and approval returns a receipt. |
| `LocalAppUpdateDependencies` | Apply an approved dependency change with its receipt: resolve, check, rebuild, and replace together or not at all. |

## What you cannot do yet

This server does not run an app or look at its screen, and the tools for those are not listed. If the user asks for one
of those, say that this plugin cannot do it yet. Do not try to do it by writing files into the data root by hand outside
an app's own workspace: the app store keeps records that hand-written files would not match, and a build needs the
verified toolchain that only the `local-app` command provides.

## When a tool fails

A tool that fails returns an error result with a message; read the message and tell the user what it says. Some
messages name a command for the person to run (`local-app toolchain install`, for one); do not run those yourself
without asking. If the `local-app` tools are missing altogether, or the server will not start, use the
`local-app-setup` skill.
