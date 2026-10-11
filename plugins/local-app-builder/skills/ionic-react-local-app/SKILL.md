---
name: ionic-react-local-app
description: Implement local apps inside the pinned React 19, Ionic 9, and Vite 8 scaffold, keeping its platform provider and adapter intact.
---

# Ionic React Local App

Use this skill when writing or repairing source inside a local app scaffolded
from this plugin's templates.

For a `dom` surface (`react-dom-r4`), this skill owns the routed shell, screen
structure, and Ionic navigation. For a `canvas` surface it covers only the
overlays, forms, and HUD UI around the scene; the scene lifecycle stays with the
matching runtime skill.

Read [references/router.md](references/router.md) first, then only the profiles
the current work needs.

Always preserve these project facts:

- React `19.3.0`
- Ionic React `9.0.4`
- Vite `8.3.0`
- React Router `6.30.6`
- platform facts come from `usePlatform()` in `lib/platform-provider.jsx`

Implement the app through the checked-in Ionic shell, route containers, and
platform adapter. Treat `deviceContext` as live input, surface failures
explicitly, and keep platform differences behind the adapter boundary instead of
scattering them through feature JSX.

The app is a web page: every capability comes from a standard web API. Do not
add or swap dependencies, rely on Capacitor, Tailwind, React Native, SwiftUI,
Jetpack Compose, or any native-plugin runtime, or require a native
reimplementation path.
