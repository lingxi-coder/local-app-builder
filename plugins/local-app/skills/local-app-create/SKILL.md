---
name: local-app-create
description: Use when the user asks to make a new Local App or to change the shape of one: write the plan file, get the person's approval through LocalAppPrepare, then install dependencies and build.
---

# Making a Local App

An app is made in a fixed order, and the person approves the plan before anything is created. Do each step and read
its result before the next.

1. **Choose a template.** Call `LocalAppTemplateCatalog`. Pick the `template_id` that fits the app (a screen-and-form
   app is `react-dom-r4`; the canvas templates are for apps that draw every frame).
2. **Create the empty app.** `LocalAppCreate` with a `brief` and optionally a `name`. Keep the `id` it returns.
3. **Write the plan.** A Markdown file, anywhere you can write, that the person can read: the product goal, the
   features, the screens and the data, in prose. It must also contain exactly one machine-readable block (below).
4. **Prepare.** `LocalAppPrepare` with the `app_id` and the **absolute** path of the plan file. The client shows the
   person the whole plan and asks them to approve it. Nothing is created unless they say yes, and the file must not
   change afterwards. Do not call this to "test" a half-written plan: every call asks the person.
5. **Install dependencies.** `LocalAppInstallDeps` with `wait` set to `true`.
6. **Write the app.** The workspace is `<data root>/<workspaceRel>`, where the server's instructions name the data root
   and `LocalAppGet` reports `workspaceRel`. Its `LINGXI.md` is the app's own contract: read it first and edit only the
   files it says are the app's. (It was written for the product the engine also serves and may name tools this server
   does not offer; only the tools listed in the `local-app` skill exist here.) Declare data collections and capabilities
   with `LocalAppManifest` before the app relies on them.
7. **Build.** `LocalAppBuild` with the `app_id`, and the `workflow_run_id` (the `execution_id` that prepare returned)
   and `contract_handle` it returned. A successful build is the completion condition. If it fails, read
   `LocalAppLogs` with `log` set to `build`, fix the source and build again.

## The plan's machine-readable block

Exactly one fenced code block whose info string is `authoring-spec`, holding one JSON object with the keys `name`,
`brief`, `template_id` and `spec`, and no others. The name and brief in it are what the app is created with.

- `template_id` is required to create an app and must be left out when the plan changes an app that is already built.
- `spec` has exactly `product`, `targets`, `ui`, `design` and `acceptance_checks`. Unknown keys anywhere are refused.
- `targets` is a non-empty list of `{id, os, form_factor}` with unique ids.
- `acceptance_checks` is a non-empty list of `{id, target_ids, required, preconditions, steps, expected, evidence}`
  with unique ids; `evidence` is a list of `inspect`, `ui_action` or `capture`. At least one check is `required`, and
  every target is named by some required check.
- Every `design.presentations[].target_id` names a target.
- The canvas templates also need `design.canvas`; `react-dom-r4` must not have it. A plan that does not fit is refused
  with a message that names the problem, before the person is asked.

A complete plan for `react-dom-r4`:

<!-- plan-example:start -->
````markdown
# Water tracker

A one-screen app to log glasses of water and see today's total.

```authoring-spec
{
  "name": "Water Tracker",
  "brief": "Log glasses of water and see today's total.",
  "template_id": "react-dom-r4",
  "spec": {
    "product": {
      "goal": "Track daily water intake",
      "tasks": ["log a glass", "show today's total"],
      "external_integrations": []
    },
    "targets": [{ "id": "phone", "os": "ios", "form_factor": "phone" }],
    "ui": {
      "structure": ["header", "log button", "today's total"],
      "theme": { "mode": "system", "accent": "teal" },
      "style": { "direction": "calm", "density": "comfortable" },
      "references": []
    },
    "design": {
      "presentations": [
        { "target_id": "phone", "presentation": "single screen", "navigation": "none" }
      ],
      "tokens": { "radius": "16px" },
      "states": {
        "loading": "skeleton total",
        "empty": "prompt to log the first glass",
        "error": "retry banner",
        "success": "total ticks up",
        "permission": "notifications not yet granted"
      },
      "inputs": {
        "pointer_touch": ["tap to log"],
        "keyboard_mouse": ["space to log"],
        "back": "system back leaves the app",
        "reduced_motion": "no counter animation"
      }
    },
    "acceptance_checks": [
      {
        "id": "log-a-glass",
        "target_ids": ["phone"],
        "required": true,
        "preconditions": [],
        "steps": ["tap the log button"],
        "expected": "today's total increases by one",
        "evidence": ["inspect"]
      }
    ]
  }
}
```
````
<!-- plan-example:end -->

## When the person cannot be asked

`LocalAppPrepare` fails with `approval_unavailable` when the client cannot ask the person (it does not support MCP
elicitation, or the person did not answer). Tell the user plainly that the plan could not be approved from here and
nothing was created. Do not work around it: not by writing the workspace yourself, and not by retrying in a loop.
`plan_not_approved` means the person said no: revise the plan with them.

## Dependencies and the toolchain

Installing and building run the pinned Node and pnpm, which the person installs once with `local-app toolchain
install`. If a call says `toolchain_not_installed`, tell the user to run that command and try again.

To add, update or remove a package later, call `LocalAppConfirmDependencyChange` with `changes` (a list of
`{kind, package, version}`; `version` is an exact version, and is left out for `remove`). The client asks the person to
approve the exact packages; if they approve you get a receipt, and `LocalAppUpdateDependencies` with that receipt
applies it. Never edit `package.json` or the lockfile by hand: they are host-managed.
