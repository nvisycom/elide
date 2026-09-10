import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import wasm from "vite-plugin-wasm";

// `elide-wasm` resolves to the raw wasm-bindgen output in `pkg/` (its `.js`
// glue, with `.d.ts` types picked up alongside). Aliasing here keeps the import
// specifier clean and means `pkg/` needs no synthesized `package.json`.
const elideWasm = fileURLToPath(
  new URL("./pkg/elide_wasm.js", import.meta.url),
);

// The demo is deployed to GitHub Pages under `/<repo>/`, so the build needs a
// matching base path. `BASE_PATH` is set by the Pages workflow; locally it
// defaults to root so `npm run dev` and `npm run preview` work unchanged.
export default defineConfig({
  base: process.env.BASE_PATH ?? "/",
  plugins: [tailwindcss(), wasm()],
  resolve: {
    alias: { "elide-wasm": elideWasm },
  },
  build: {
    target: "esnext",
    outDir: "dist",
    emptyOutDir: true,
  },
});
