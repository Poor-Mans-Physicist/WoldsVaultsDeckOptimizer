// Node stand-in for web/src/lib/workerPool.ts, used by bench_struct.ts (esbuild redirects every workerPool
// import here). Same exports; jobs go to whichever worker_threads worker is free, like the browser pool.
import { Worker } from "node:worker_threads";

const LANES = Number(process.env.STRUCT_LANES ?? "24");
const WASM = process.env.STRUCT_WASM ?? "";
const WORKER = process.env.STRUCT_WORKER ?? "";

interface Slot { worker: Worker; busy: number }
let pool: Slot[] | null = null;
let nextId = 1;
const pending = new Map<number, { res: (r: unknown) => void; rej: (e: Error) => void; slot: Slot }>();
export let cpuMs = 0;

function ensure(): Slot[] {
  if (pool) return pool;
  if (!WASM || !WORKER) throw new Error("struct_shim: set STRUCT_WASM and STRUCT_WORKER");
  pool = Array.from({ length: LANES }, () => {
    const s: Slot = { worker: new Worker(WORKER, { workerData: { wasmPath: WASM } }), busy: 0 };
    s.worker.on("message", (m: { id: number; ok: boolean; r: unknown; error: string; ms: number }) => {
      const p = pending.get(m.id);
      if (!p) return;
      pending.delete(m.id);
      p.slot.busy--;
      cpuMs += m.ms ?? 0;
      if (m.ok) p.res(m.r); else p.rej(new Error(m.error));
    });
    return s;
  });
  return pool;
}

function send<T>(type: string, payload: Record<string, unknown>): Promise<T> {
  const p = ensure();
  let slot = p[0];
  for (const s of p) if (s.busy < slot.busy) slot = s;
  slot.busy++;
  const id = nextId++;
  return new Promise<T>((res, rej) => {
    pending.set(id, { res: res as (r: unknown) => void, rej, slot });
    slot.worker.postMessage({ id, type, payload });
  });
}

export function workerPoolSize(): number { return ensure().length; }
export function dispatchTagged<T>(_w: number, payload: Record<string, unknown>): Promise<T> {
  return send<T>("tagged", payload);
}
export function dispatchConstruction<T>(_w: number, type: string, payload: Record<string, unknown>): Promise<T> {
  return send<T>(type, payload);
}
export function dispatchSlice(): never { throw new Error("struct_shim: dispatchSlice is not used by the bench"); }
export function chunkInto<T>(items: readonly T[], n: number): T[][] {
  const m = Math.max(1, Math.min(n, items.length));
  const out: T[][] = Array.from({ length: m }, () => []);
  items.forEach((x, i) => out[i % m].push(x));
  return out;
}
export function closePool(): void { for (const s of pool ?? []) void s.worker.terminate(); }
