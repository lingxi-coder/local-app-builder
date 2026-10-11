import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";

const projectRoot = path.dirname(fileURLToPath(import.meta.url));

/// The user opens `dist/index.html` straight from disk. A `file://` page has an
/// opaque origin, so the browser refuses `<script type="module">` and anything
/// marked `crossorigin` there (the CORS check cannot pass). The bundle is a
/// classic script already (see `format` below); this only rewrites the tag Vite
/// injects, keeping `defer` so it still runs after the document is parsed.
function classicEntryScript() {
  return {
    name: "classic-entry-script",
    apply: "build",
    transformIndexHtml: {
      order: "post",
      handler(html) {
        return html.replace(
          /<script type="module" crossorigin src=/g,
          "<script defer src=",
        );
      },
    },
  };
}

export default {
  base: "./",
  plugins: [react(), classicEntryScript()],
  resolve: {
    alias: {
      "@": projectRoot,
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    target: "safari17",
    reportCompressedSize: false,
    // Every imported asset (images, fonts, audio, models, Babylon's Havok
    // WebAssembly) goes into the bundle as a data URL. A page opened from
    // `file://` cannot fetch a file that sits next to it, and WebGL refuses an
    // image loaded from one (SecurityError); the same bytes as a data URL work
    // in both.
    assetsInlineLimit: () => true,
    rollupOptions: {
      output: {
        // NOT a stylistic choice: `dist/index.html` must work from `file://`,
        // which rules out ES modules (see `classicEntryScript`). An `iife`
        // bundle is a classic script; the app's CSS is injected at runtime
        // through a `<style>` element.
        //
        // It also builds where WebAssembly is missing. Vite's
        // `vite:build-import-analysis` runs its bundled es-module-lexer (WASM)
        // over every chunk, but only for `es` output: its `generateBundle`
        // opens with `if (format !== "es") return;` before it awaits the
        // lexer's init. A jitless V8 has no WebAssembly, and there every `es`
        // build failed after transforming every module.
        //
        // And it fixes the only Ionic import style that works here.
        // `@ionic/core/components/ion-*.js` (the tree-shakeable per-component
        // entry points) dynamically import one another, and rolldown rejects
        // that under `iife` with "UMD and IIFE are not supported for
        // code-splitting builds" — `output.codeSplitting: false` does not lift
        // it. Import from the `@ionic/react` barrel instead; measured, that is
        // a flat ~1.4 MB whether the app uses one component or twelve.
        format: "iife",
        // An iife bundle is a single scope and cannot code-split, so any
        // dynamic import has to be folded in rather than emitted as a chunk.
        inlineDynamicImports: true,
      },
    },
  },
};
