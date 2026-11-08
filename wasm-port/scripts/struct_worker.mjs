// Worker thread for bench_struct.ts: one WASM instance; runs tagged and construction payloads.
import { parentPort, workerData } from "node:worker_threads";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require(workerData.wasmPath);
const fns = {
  tagged: (p) => wasm.runSaTagged(p),
  cc_enum: (p) => wasm.constructionEnumerate(p),
  cc_eval: (p) => wasm.constructionEval(p),
  cc_chain: (p) => wasm.constructionChain(p),
};

parentPort.on("message", ({ id, type, payload }) => {
  try {
    const t = performance.now();
    const r = fns[type](payload);
    parentPort.postMessage({ id, ok: true, r, ms: performance.now() - t });
  } catch (e) {
    parentPort.postMessage({ id, ok: false, error: String(e) });
  }
});
