---
name: threejs-local-app
description: Build local apps whose primary scene uses Three.js 0.186.0 with explicit lifecycle, disposal, input, and performance rules.
---

# Three.js Local App

Use this skill when the app's surface is canvas and it was scaffolded from the
`three-3d-r4` template (family `three_3d`).

Read [references/router.md](references/router.md) first, then only the profiles
needed for the current scene work.

Use only `three@0.186.0` from the shipped scaffold.

Always:

- own one renderer and one animation loop
- handle scene, camera, lights, materials, resize, pixel ratio, and teardown
  explicitly
- dispose geometries, materials, textures, listeners, and renderer resources
- define pointer and keyboard behavior for the core actions
- keep overlays and platform chrome outside the scene

This skill is only for the `three_3d` template. Do not switch to `canvas_2d`,
`phaser_2d`, or `babylon_3d`, and do not ask for a second 3D runtime.

Do not introduce React Three Fiber, drei, helper scene wrappers, external
physics stacks, postprocess dependency piles, or any extra 3D runtime layer.
