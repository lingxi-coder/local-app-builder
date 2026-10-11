# Engine lifecycle and performance

- Use the template-managed `createBabylonRuntime` adapter as the one explicit
  engine, scene, camera, lighting, render-loop, resize, and disposal path; app
  code supplies behavior callbacks instead of creating another engine.
- Resize from the template-owned canvas box, update camera parameters, and avoid
  accidental duplicate render loops after hot reload or resume.
- Dispose meshes, materials, textures, observers, and engine resources on
  teardown.
- Handle WebGL context loss or restoration intentionally instead of leaving the
  app on a blank frame.
- Keep geometry, texture, and shader complexity conservative on phone-class
  hardware unless the brief clearly requires more.
- Import textures and models from source (for example `src/assets/`); the
  build inlines every import as a data URL. Never load them from `public/`: the
  page is opened from `file://`, where a file beside it cannot be fetched and
  WebGL rejects an image loaded from one. Vite does not treat model formats as
  assets, so import them with `?url` (`import modelUrl from
  "@/src/assets/model.glb?url"`) and load with
  `LoadAssetContainerAsync(modelUrl, scene)`. Prefer a self-contained `.glb`: a
  `.gltf` cannot fetch the separate `.bin` or texture files it references.

## Sources

Reviewed: 2026-08-27

- Babylon scene docs: https://doc.babylonjs.com/features/featuresDeepDive/scene
- Babylon optimize your scene: https://doc.babylonjs.com/features/featuresDeepDive/scene/optimize_your_scene
