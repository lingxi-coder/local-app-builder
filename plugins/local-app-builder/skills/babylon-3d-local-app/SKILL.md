---
name: babylon-3d-local-app
description: Build local apps whose primary scene uses the Babylon 3D template runtime with explicit engine lifecycle, input parity, and disposal rules.
---

# Babylon 3D Local App

Use this skill when the app's surface is canvas and it was scaffolded from the
`babylon-3d-r4` template (family `babylon_3d`).

Read [references/router.md](references/router.md) first, then only the profiles
needed for the current scene work.

Use only the template-managed Babylon runtime already in the app
(`lib/babylon-runtime.js`): `@babylonjs/core@9.27.1`,
`@babylonjs/loaders@9.27.1`, and `@babylonjs/havok@1.3.14` for physics. If the
app was not scaffolded from the Babylon template, do not assume it exists and
do not request package installs from this skill.

Always:

- use the template-managed `createBabylonRuntime` from
  `lib/babylon-runtime.js`; it is the single engine, scene, render-loop,
  resize, Havok initialization, and teardown owner, so app code supplies
  business callbacks and never creates a second engine or loop
- keep one Babylon scene lifecycle through that adapter
- define both pointer or touch and keyboard behavior for every core action
- handle resize, DPR changes, context loss, disposal, and pause or resume
  explicitly
- keep data, storage, and overlay UI outside engine globals and surface
  failures clearly
- keep deterministic scene-update seams for logic checks

Do not add npm installs, alternate engines, React 3D wrappers, or external
physics or asset-service dependencies.
