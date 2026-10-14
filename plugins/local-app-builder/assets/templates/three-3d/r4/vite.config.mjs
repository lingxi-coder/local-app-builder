import path from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";

const projectRoot = path.dirname(fileURLToPath(import.meta.url));

/// The build is ONE file, `dist/index.html`: LingXi's `LocalApp` tool publishes
/// that file as the whole app, and the user can also open it straight from
/// disk. The bundle is a classic script already (see `format` below); this
/// moves it from `dist/assets/` into a `<script>` at the end of `<body>`, where
/// it runs after the document is parsed, as a `defer` tag would. The tag Vite
/// writes could not stay anyway: a `file://` page has an opaque origin, so the
/// browser refuses `<script type="module">` and anything marked `crossorigin`.
function singleFileDocument() {
  return {
    name: "single-file-document",
    apply: "build",
    enforce: "post",
    generateBundle(_options, bundle) {
      const page = bundle["index.html"];
      if (!page || page.type !== "asset") {
        this.error("single-file-document: the build wrote no index.html");
      }
      const scripts = [];
      const html = String(page.source).replace(
        /\s*<script type="module" crossorigin src="\.\/([^"]+)"><\/script>/g,
        (_tag, fileName) => {
          const chunk = bundle[fileName];
          if (!chunk || chunk.type !== "chunk") {
            this.error(`single-file-document: ${fileName} is not a bundle chunk`);
          }
          scripts.push(chunk.code);
          delete bundle[fileName];
          return "";
        },
      );
      if (scripts.length !== 1) {
        this.error(`single-file-document: expected one entry script, found ${scripts.length}`);
      }
      const left = Object.keys(bundle).filter((name) => name !== "index.html");
      if (left.length > 0) {
        this.error(`single-file-document: the build also wrote ${left.join(", ")}`);
      }
      // `</script` would end the element early and `<!--` would change how
      // the browser finds its end; `\x3C` is the same `<` in every string,
      // template literal and regular expression of the bundle.
      const code = scripts[0].replace(/<(\/script|!--)/gi, "\\x3C$1");
      page.source = html.replace("</body>", () => `  <script>${code}</script>\n  </body>`);
    },
  };
}

export default {
  base: "./",
  plugins: [react(), singleFileDocument()],
  resolve: {
    alias: {
      "@": projectRoot,
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Nothing may sit next to the page: `public/` is never copied, and every
    // file the app needs is imported from source (see `assetsInlineLimit`).
    copyPublicDir: false,
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
        // which rules out ES modules (see `singleFileDocument`). An `iife`
        // bundle is a classic script; the app's CSS is injected at runtime
        // through a `<style>` element, so the script is the whole app.
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
