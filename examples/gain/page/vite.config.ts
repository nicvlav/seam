// The gain's page, built into the plugin. The plugin embeds the built files
// by name (`src/page.rs`), so they keep fixed names: index.html,
// assets/index.js and assets/index.css.

import { defineConfig } from "vite";

export default defineConfig({
  // Relative URLs, so the page loads from the plugin's own scheme.
  base: "./",
  // One React for the page and for `seam-page`, which is linked from the
  // repository and has a React of its own for its tests.
  resolve: { dedupe: ["react", "react-dom"] },
  oxc: { jsx: { runtime: "automatic" } },
  build: {
    // WebView2 is a current Chromium.
    target: "chrome120",
    rolldownOptions: {
      output: {
        entryFileNames: "assets/index.js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/index[extname]",
      },
    },
  },
});
