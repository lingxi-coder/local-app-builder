# Lifecycle and performance

- Create scene, camera, renderer, and lights in one owned initialization path.
- Keep default materials and lighting readable on phone-class displays before
  adding heavier post effects or complex shader work.
- Drive `renderer.render(scene, camera)` from the template-managed
  `createFrameLoop` in `lib/frame-loop.js`; do not start
  `renderer.setAnimationLoop` or a second
  `requestAnimationFrame` owner alongside it.
- On resize, call `renderer.setSize(width, height, false)` and update camera
  aspect or projection parameters and any dependent picking math. The third
  argument is required: `createFrameLoop` owns the drawing buffer and the CSS
  box, and `setSize`'s default `updateStyle: true` writes inline
  `style.width`/`style.height` onto the canvas, pinning the box the helper's
  ResizeObserver watches — after which a rotation or Split View change resizes
  the container while the canvas keeps its old size and the observer never
  fires again. Use the `width`/`height` the helper reports rather than reading
  the element, and do not call `setPixelRatio` above the helper's clamp.
- Cap effective pixel ratio when necessary for thermals and frame stability.
- On teardown, dispose geometries, materials, textures, render targets, and
  event listeners, then release renderer resources.
- Handle WebGL context loss and restoration intentionally instead of leaving the
  app in a blank frame.
- Keep default scenes inside mobile-friendly geometry, texture, and draw-call
  budgets, with simple materials and lighting unless the brief clearly warrants more.
- Import textures and models from source (for example `src/assets/`); the
  build inlines every import as a data URL, which `TextureLoader` and
  `GLTFLoader` load directly. Never load them from `public/`: the page is
  opened from `file://`, where a file beside it cannot be fetched and WebGL
  rejects an image loaded from one. Vite does not treat model formats as
  assets, so import them with `?url`, and prefer a self-contained `.glb`.
- Avoid per-frame object allocation in hot paths.

## Sources

Reviewed: 2026-08-27

- WebGLRenderer docs: https://threejs.org/docs/#api/en/renderers/WebGLRenderer
- Disposal guide: https://threejs.org/manual/en/how-to-dispose-of-objects.html
- MDN WebGL context-lost event: https://developer.mozilla.org/en-US/docs/Web/API/HTMLCanvasElement/webglcontextlost_event
