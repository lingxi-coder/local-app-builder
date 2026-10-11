---
name: create-local-app
description: Plan a local web app with the user, scaffold it from this plugin's templates, implement it, build it with pnpm, and open the built page in the desktop browser.
---

# Create or modify a local app

A local app is a self-contained web page. It builds into `dist/index.html` plus
one script, the user opens it straight from disk in their desktop browser, and
it keeps its data in that browser. It uses standard web APIs only: no server,
no account, no native bridge.

The MAIN session does all of the work with the ordinary tools: plan mode and
`AskUserQuestion` to agree on the app, Write and Edit for source, Bash for the
copy, install and build. Nothing scores the result and there is no automatic
verification; the user tries the built app.

Specialist skills are references for the source you write: `$frontend-design`,
`$accessibility`, `$react-best-practices`, `$ionic-react-local-app`, and the
drawn-surface specialists `$canvas-2d-local-app`, `$threejs-local-app`,
`$phaser-2d-local-app`, `$babylon-3d-local-app`. Load the ones the work needs;
none of them is a mandatory stage.

The templates ship with this plugin:

- catalog: `${LINGXI_SKILL_DIR}/../../assets/templates/catalog.json`
- the template at a catalog `path`:
  `${LINGXI_SKILL_DIR}/../../assets/templates/<path>/`

## Where this runs

This flow needs a desktop: Node.js, a shell, and a browser that opens a local
file. If the session carries a `Mobile runtime environment` reminder, you are in
the LingXi mobile app, where it does not run. Tell the user to create the app
from LingXi on their computer, and stop.

## 1. Plan

Call `EnterPlanMode`. Plan mode is read-only: read the catalog and, for a
change, the existing app. Do not copy a template, write source, or build until
the user approves the plan.

Converge the following, asking only materially unresolved questions with
`AskUserQuestion` — never a question quota, never re-asking an answered
decision, and never leaving an open question in ordinary assistant text:

- **Product** — the goal, the main features, and the success condition.
- **Targets** — the app runs in the desktop browser that opens it, so the
  default target is `desktop`. Add an iPhone, iPad, Android phone or tablet
  presentation only when the user asks for one, and show it in the plan.
- **Data** — what the app stores, and that it stays in this browser on this
  computer (see Data below). Say so plainly when the brief expects sync,
  sharing, or an account.
- **UI** — a COMPLETE recommendation, presented in full: page structure and
  navigation, layout, light/dark/system theme and accent, style and density,
  the complete states, and how it adapts across the targets. State it
  concretely (screens, back/navigation semantics, tokens, adaptive behavior);
  this is what the user approves. Use `$frontend-design` to produce it.

Choose the template from the catalog and SHOW the choice in the plan; the user
corrects it if wrong. Its `surface` decides the shape: `canvas` when the whole
interface is one drawn surface that owns a frame loop (a game, a simulation, a
3D scene, a live visualization), `dom` for everything assembled from screens,
lists and forms. The template cannot be swapped later — an app that needs a
different one is created again.

If a target is iOS or iPadOS and the change affects the UI, load
`local-app-builder:apple-design` once before proposing the UI and apply it to
the iOS/iPadOS output only, never to Android or desktop.

When the brief needs what a web page cannot do, say so in the plan and propose
the nearest browser-only design instead of promising it. A page does no work
while it is closed (no background sync, no scheduled reminders), cannot reach
the system calendar or contacts (export an `.ics` or `.vcf` file instead), and
makes no AI model calls — a page opened from disk has nowhere safe to keep an
API key.

Write the plan file with the app's name and folder, the template, the product,
the targets, the data, the full UI recommendation, and a SHORT trial checklist:
the steps the user will take to try the app. Then call `ExitPlanMode`. The
user's approval of the plan is the go-ahead; do not ask for a second
confirmation.

## 2. Scaffold

Create the app in a new folder named after the app in lower-case kebab case
(`errand-list`), inside the current working directory unless the user named a
place. If that folder already exists and is not empty, ask before using it;
never copy a template over existing work.

```bash
mkdir errand-list
cp -R "${LINGXI_SKILL_DIR}/../../assets/templates/react-dom/r4/." errand-list/
```

Then set the app's identity, and change nothing else in these two files:

- `package.json` `name` — the folder name;
- `index.html` `<title>` — the app's display name, and `<html lang>` — the
  user's language.

## 3. Implement

Write the source the approved plan describes. Edit app-owned files only:
`app/app.jsx`, `app/screens/`, `app/globals.css`, and new files under `app/`,
`src/` and `components/`. The rest is template-managed; leave it as it is:
`index.html` (apart from the title and language), `vite.config.mjs`,
`jsconfig.json`, `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`,
`app/main.jsx`, `app/providers.jsx`, `app/error-boundary.jsx`,
`styles/foundation.css`, and every file the template put under `lib/`.

The specialist skills are guidance; the approved plan decides. If a material
detail is unresolved, ask with `AskUserQuestion` rather than inventing it.

### Data

Keep the app's data in the browser: IndexedDB for records, `localStorage` only
for small settings. Namespace both with the folder name — the database is
`errand-list`, every `localStorage` key starts with `errand-list:` — because the
browser gives every page opened from `file://` one shared storage, and a plain
key such as `settings` collides with every other local app. Validate what you
read back (zod is pinned): stored data outlives the code that wrote it. Surface
a failed save as a retryable error; never report success for it.

When the data matters, offer export and import: a JSON file download and an
`<input type="file">`.

### Web APIs only

Everything comes from a standard web API, CSS, or a pinned package: no native
bridge, no Capacitor or other wrapper, no CDN script or font (the page must work
offline), and no secret in source. Ask for camera, microphone, location,
clipboard or notifications only from a user action, and keep the screen usable
when the browser or the user refuses; a browser may grant a `file://` page less
than a website.

A request to an external API leaves from origin `null`, so only a service that
allows any origin answers it. Use an external service only when the user asked
for it, and say in the plan that the app needs the network for it.

### Assets

Import every image, font, sound or model from source (for example
`src/assets/`); the build inlines each import into the bundle, which is what
lets it load from `file://`. Vite does not treat model formats as assets, so
import a `.glb` with `?url`. Never load a runtime file from `public/` or by an
absolute path such as `/logo.png`: from `file://` it cannot be fetched, and
WebGL rejects an image loaded that way. Use inline SVG or CSS for ordinary
icons.

Only when the brief needs an original bitmap (photo, illustration, texture,
hero, background) and an image-generation tool is available, generate it into
`src/assets/` and import it. Otherwise use CSS, gradients, the user's own
images, or an honest placeholder.

### Dependencies

The template pins React, Ionic, Vite, the renderer engines, zod and Zustand;
keep them. Prefer a source-only implementation to a new package. Add one only
after the user agrees to it with `AskUserQuestion`, at an exact version:
`pnpm add --save-exact <name>@<version>` (see Build for how to run pnpm). Never
edit `package.json` dependencies or `pnpm-lock.yaml` by hand, and never upgrade
or remove a pinned package.

## 4. Build

Run every command inside the app folder.

1. `node -v` must satisfy `^20.19.0 || >=22.12.0`. If Node is missing or older,
   tell the user to install the current Node.js LTS from https://nodejs.org and
   stop; do not install it yourself.
2. Run pnpm at the version the template pins. If `pnpm --version` prints
   `12.5.1`, use `pnpm`; otherwise use
   `npm exec --yes --package=pnpm@12.5.1 -- pnpm` in its place. Do not use
   Corepack: it cannot start pnpm 12.
3. `pnpm install --frozen-lockfile`. The first install downloads packages and
   needs the network.
4. `pnpm build` writes `dist/index.html` and one script under `dist/assets/`.

A successful build is the completion condition. When it fails, read the error;
fix your source and build again as long as the failure is a defect you can
identify, and do not loop blindly. If the cause is the environment — no network
for the install, a missing or broken Node — report it to the user and stop.

## 5. Deliver

Open the built page in the user's default browser: `open dist/index.html` on
macOS, `xdg-open dist/index.html` on Linux, `start "" dist\index.html` on
Windows.

Then tell the user, in their language:

- the app's name and the full path of its `dist/index.html`, which they can
  bookmark or open again from there;
- that its data stays in this browser on this computer: another browser, a
  private window, or clearing site data starts empty;
- the SHORT trial checklist from the plan, as steps they can perform;
- the status line exactly: `已构建，待你试用`.

Do not run UI operations, take screenshots, or score the app on your own. The
user tries it; a problem they report is a change request.

## Changing an existing app

When the user asks to change an app that already exists (its folder holds the
template's `package.json`), do not scaffold again and do not repeat the opening
interview. Plan the change in plan mode, keeping the app's template, get it
approved, implement it, build, and deliver again. Stored data must survive the
change: when the shape of stored records changes, migrate them (raise the
IndexedDB version and convert in `onupgradeneeded`) instead of dropping them.
