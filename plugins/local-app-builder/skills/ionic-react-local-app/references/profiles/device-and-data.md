# Device and data

- Read device, safe-area, color-scheme, reduced-motion, and input-mode facts
  through `usePlatform()` or the platform adapter rather than ad hoc window
  probes.
- Treat `viewport`, `safeArea`, `colorScheme`, `reducedMotion`, and
  `inputMode` as live values that can change during a session.
- In CSS, read safe areas from `env(safe-area-inset-*)`;
  `styles/foundation.css` already feeds them into Ionic's variables.
- Keep the app's data in the browser: IndexedDB for records, `localStorage`
  only for small settings. Validate what you read back (zod is pinned): stored
  data outlives the code that wrote it.
- Ask for camera, microphone, location, or clipboard access only from a user
  action, through the standard web API, and keep the screen usable when the
  browser or the user refuses.
- Reflect rejected writes, permission refusals, and storage failures in UI state
  instead of swallowing them.
- Import images, fonts, and other assets from source (for example
  `src/assets/`); the build inlines each import, so it loads from a page opened
  via `file://`. Do not reference a file in `public/` by an absolute path such
  as `/logo.png`: from `file://` that resolves to the root of the disk.

## Sources

Reviewed: 2026-10-10

- Device context: the plugin's `assets/templates/react-dom/r4/lib/device-context.js`
- Platform adapter: the plugin's `assets/templates/react-dom/r4/lib/platform-adapter.js`
