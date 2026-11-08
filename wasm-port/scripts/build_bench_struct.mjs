// Bundle bench_struct.ts against the web sources of <repo>, replacing every workerPool import with
// struct_shim.ts (the bench's own copy, so older commits can be benched with today's harness).
//   node build_bench_struct.mjs <repo> <out.mjs>
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { copyFileSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";

const [repo, outFile] = process.argv.slice(2);
const here = dirname(fileURLToPath(import.meta.url));
const { build } = createRequire(resolve(here, "../web/package.json"))("esbuild");
const dest = resolve(repo, "wasm-port/scripts");
mkdirSync(dest, { recursive: true });
for (const f of ["bench_struct.ts", "struct_shim.ts", "struct_worker.mjs"]) {
  if (resolve(here, f) !== resolve(dest, f)) copyFileSync(resolve(here, f), resolve(dest, f));
}
const shim = resolve(dest, "struct_shim.ts");

await build({
  entryPoints: [resolve(dest, "bench_struct.ts")],
  bundle: true, platform: "node", format: "esm", outfile: outFile, logLevel: "warning",
  plugins: [{
    name: "worker-pool-shim",
    setup(b) { b.onResolve({ filter: /workerPool$/ }, () => ({ path: shim })); },
  }],
});
