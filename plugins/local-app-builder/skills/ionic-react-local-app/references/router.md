# Router

Read this file first.

- App shell, routing, and page-container work ->
  `profiles/app-shell-and-routing.md`
- Device context, data, or permissions work ->
  `profiles/device-and-data.md`

## Scope

This skill assumes the app was scaffolded from this plugin's template and
already ships:

- writable source under `app/`, `src/`, `components/`, `lib/`, `styles/`, and
  `public/`
- pinned dependencies for React 19, Ionic 9, Vite 8, zod 4, and Zustand 5
- the template-managed platform provider, device context, and platform adapter
  under `lib/`

This skill applies to routed DOM apps and to canvas apps that use Ionic only
for overlays. Keep the existing scaffold shape; do not create a second shell.

Do not rewrite `index.html`, `vite.config.mjs`, the files under `lib/` that
came with the template, or the dependency manifests.

## Sources

Reviewed: 2026-10-10

- Ionic React docs: https://ionicframework.com/docs/react
- Template package: the plugin's `assets/templates/react-dom/r4/package.json`
- Platform provider: the plugin's `assets/templates/react-dom/r4/lib/platform-provider.jsx`
