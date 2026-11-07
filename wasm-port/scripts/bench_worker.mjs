// Worker thread for bench_web.ts: one WASM instance, runs kernel payloads on request.
import { parentPort, workerData } from "node:worker_threads";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require(workerData.wasmPath);

parentPort.on("message", ({ id, payload }) => {
  try {
    const t = performance.now();
    const r = wasm.runSaTagged(payload);
    parentPort.postMessage({ id, ok: true, r, ms: performance.now() - t });
  } catch (e) {
    parentPort.postMessage({ id, ok: false, error: String(e) });
  }
});
