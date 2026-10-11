# Assets, performance, and QA

- Preload image, atlas, tilemap, and audio assets intentionally; when an asset
  is missing, fall back to procedural shapes, gradients, or text instead of a
  broken scene.
- Import every image, atlas, tilemap, and audio file from source (for example
  `src/assets/`) and hand the imported URL to the Phaser loader; the build
  inlines each import as a data URL, which the loader reads directly. Never
  load assets from `public/`: the page is opened from `file://`, where the
  loader cannot fetch a file beside it.
- Keep frame cost conservative on phone-class hardware: small texture budgets,
  limited concurrent tweens, and no unnecessary full-screen post effects.
- Reduced motion should reduce decorative camera shake, particle count, or
  tween intensity while keeping the app playable.
- Publish score, phase, objective, and errors through readable overlay text.
- Keep at least one deterministic logic path testable without a live renderer.

## Sources

Reviewed: 2026-08-27

- Phaser loader docs: https://docs.phaser.io/api-documentation/class/loader-loaderplugin
- Phaser tween concepts: https://docs.phaser.io/phaser/concepts/tweens
- WCAG 2.2: https://www.w3.org/TR/WCAG22/
