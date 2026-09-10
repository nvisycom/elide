import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import wasm from "vite-plugin-wasm";

// The demo is deployed to GitHub Pages under `/<repo>/`, so the build needs a
// matching base path. `BASE_PATH` is set by the Pages workflow; locally it
// defaults to root so `npm run dev` and `npm run preview` work unchanged.
export default defineConfig({
  base: process.env.BASE_PATH ?? "/",
  plugins: [tailwindcss(), wasm()],
  build: {
    target: "esnext",
    outDir: "dist",
    emptyOutDir: true,
  },
});
