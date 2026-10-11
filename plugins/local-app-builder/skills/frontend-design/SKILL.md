---
name: frontend-design
description: Produce a platform-specific design spec for a local app, separating iOS/iPadOS, Android, desktop, and DOM versus canvas overlay presentation.
---

# Frontend design

Turn the confirmed product brief into one compact `design_spec` concrete enough
to implement without guessing. In `create-local-app` this is the UI part of the
plan the user approves.

Normalize each target's OS to lower case before routing. For an iOS/iPadOS
target, load `local-app-builder:apple-design` once before producing that
target's output and apply it to iOS/iPadOS only; skip it for code-only work.
Adapt the guidance to the Ionic shell the templates ship, keep an explicit brand
or reference authoritative, and do not add APIs, dependencies, or nested
token/state fields.

Always:

- start from the confirmed product goal, target matrix, and surface choice, then
  map those decisions into one explicit `design_spec`;
- keep business logic shared but describe presentation per target;
- output distinct presentations for every requested target in `targets[]`;
- separate DOM screen design from canvas HUD/overlay design;
- refuse a single "responsive" answer when multiple platforms are in scope.

Return a `design_spec` with at least:

```yaml
design_spec:
  surface: dom | canvas
  targets:
    - os: ios | android | desktop
      form_factor: iphone | ipad | phone | tablet | desktop
      presentation: ""
      navigation: ""
  information_architecture:
    screens: []
    states:
      loading: ""
      empty: ""
      error: ""
      success: ""
      permission: ""
  visual_system:
    design_direction: ""
    tokens:
      color: []
      typography: []
      spacing: []
      radius: []
      elevation: []
      motion: []
      safe_area: []
  interaction_model:
    pointer_touch: []
    keyboard_mouse: []
    back: []
    reduced_motion: []
  adapter_boundary:
    shared_logic: []
    platform_specific_shell: []
  acceptance_checks: []
  summary: ""
```

Read [references/router.md](references/router.md) first, then only the
renderer and platform profiles that match the confirmed scope. Do not edit
source files, install packages, or invent unsupported platform behavior inside
this skill. Bring material new design decisions to the user before
implementation; this skill does not write the implementation itself.
