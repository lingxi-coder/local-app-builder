---
name: canvas-2d-local-app
description: Build local apps whose primary interaction surface is a Canvas 2D scene with a deterministic loop, native-feeling overlays, and robust input handling.
---

# Canvas 2D Local App

Use this skill when the app's surface is canvas and it was scaffolded from the
`canvas-2d-r4` template (family `canvas_2d`).

Read [references/router.md](references/router.md) first, then only the profiles
needed for the scene you are building.

Always:

- use the template-managed `createFrameLoop` from `lib/frame-loop.js`; start it
  inside an effect and stop it in that effect's cleanup
- do not call `requestAnimationFrame` directly or hand-write a replacement loop
- model an explicit phase machine
- define both pointer or touch and keyboard behavior for every core action
- keep per-frame simulation state out of React UI state
- support resize, DPR changes, suspend or resume, reduced motion, and overlay UI
- keep deterministic logic test seams for phase and simulation checks

This skill is only for the engine-free `canvas_2d` runtime. Do not switch to
`three_3d`, `phaser_2d`, or `babylon_3d`, and do not add a package-based scene
runtime.

Use browser and scaffold primitives only. Do not add a separate game engine or
rendering wrapper.
