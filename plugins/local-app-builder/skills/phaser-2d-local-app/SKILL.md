---
name: phaser-2d-local-app
description: Build local apps whose primary scene uses the Phaser 2D template runtime with explicit scene lifecycle and input parity.
---

# Phaser 2D Local App

Use this skill when the app's surface is canvas and it was scaffolded from the
`phaser-2d-r4` template (family `phaser_2d`).

Read [references/router.md](references/router.md) first, then only the profiles
needed for the current scene work.

Use only the template-managed Phaser runtime already in the app
(`lib/phaser-runtime.js`, `phaser@4.2.1`). If the app was not scaffolded from
the Phaser template, do not assume it exists and do not request package
installs from this skill.

Always:

- use the template-managed `createPhaserRuntime` from
  `lib/phaser-runtime.js`; it is the single Phaser `Game`, scheduler, resize,
  and teardown owner, so app code supplies scene callbacks and never creates a
  second `Game` or frame loop
- keep one explicit scene lifecycle through that adapter
- define both pointer or touch and keyboard behavior for every core action
- keep data, storage, and error handling explicit rather than hiding them in
  Phaser globals
- support resize, DPR changes, pause or resume, reduced motion, and procedural
  asset fallback
- keep deterministic scene or simulation seams for logic tests

Do not add npm installs, alternate engines, physics plugins, native wrappers,
or external asset-service dependencies.
