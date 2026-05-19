import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import wasm from "vite-plugin-wasm";
import topLevelAwait from "vite-plugin-top-level-await";

// Deployed to https://<user>.github.io/woldsvaultsdeckoptimizer/ so all
// asset URLs must be prefixed with the repo name. Override with VITE_BASE
// for custom deployments (e.g. local preview or a custom domain).
const base = process.env.VITE_BASE ?? "/woldsvaultsdeckoptimizer/";

export default defineConfig({
  base,
  plugins: [
    svelte(),
    wasm(),
    topLevelAwait(),  // wasm-pack's web target uses top-level await for init
  ],
  worker: {
    format: "es",
    plugins: () => [wasm(), topLevelAwait()],
  },
  build: {
    target: "esnext",
    sourcemap: true,
  },
});
