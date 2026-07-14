// A lazily-spawned pool of `optimize.worker.ts` workers. Each call to
// `dispatchSlice` sends one slice of candidates to one worker; the caller
// uses `Promise.all` to fan out across the pool.
//
// The pool size defaults to `navigator.hardwareConcurrency` clamped to
// [1, 12]. Workers are spawned on first use and live forever (closed on
// page unload). Each worker has its own wasm instance.

import OptimizeWorker from "../worker/optimize.worker?worker";
import type { OptimizeInput, SliceResult } from "./optimize";
import type { CoreSpec } from "./types";

interface PendingSlot {
  resolve: (r: SliceResult) => void;
  reject:  (e: Error) => void;
}

interface WorkerSlot {
  worker: Worker;
  pending: Map<number, PendingSlot>;
}

let _pool: WorkerSlot[] | null = null;
let _nextId = 1;

function makeWorker(): WorkerSlot {
  const w = new OptimizeWorker();
  const pending = new Map<number, PendingSlot>();
  w.onmessage = (ev: MessageEvent) => {
    const { id, ok, result, error } = ev.data;
    const slot = pending.get(id);
    if (!slot) return;
    pending.delete(id);
    if (ok) slot.resolve(result as SliceResult);
    else    slot.reject(new Error(error));
  };
  w.onerror = (e) => {
    const err = new Error(`worker crashed: ${e.message ?? "unknown"}`);
    for (const s of pending.values()) s.reject(err);
    pending.clear();
    // Drop this worker from the pool; next dispatch will respawn.
    if (_pool) _pool = _pool.filter((slot) => slot.worker !== w);
    w.terminate();
  };
  return { worker: w, pending };
}

function ensurePool(): WorkerSlot[] {
  if (_pool) return _pool;
  const hw = (typeof navigator !== "undefined" && navigator.hardwareConcurrency)
    ? navigator.hardwareConcurrency
    : 4;
  // Cap at 12: pool sizes beyond that hit diminishing returns on the
  // candidate counts we typically see (≤ 50 combos per run) and waste memory
  // on cold-start wasm instances.
  const size = Math.min(12, Math.max(1, hw));
  _pool = Array.from({ length: size }, makeWorker);
  return _pool;
}

export function workerPoolSize(): number {
  return ensurePool().length;
}

/** Send one slice to a specific worker. Returns the slice's best result. */
export function dispatchSlice(
  workerIdx:        number,
  input:            OptimizeInput,
  candidatesSlice:  CoreSpec[][],
): Promise<SliceResult> {
  const pool = ensurePool();
  const slot = pool[workerIdx % pool.length];
  const id = _nextId++;
  return new Promise<SliceResult>((resolve, reject) => {
    slot.pending.set(id, { resolve, reject });
    slot.worker.postMessage({
      id, type: "slice", input, candidatesSlice,
    });
  });
}

/** Optimizer 2.0: send one pre-built tagged payload (candidate × restart
 *  chunk) to a specific worker. */
export function dispatchTagged<T>(
  workerIdx: number,
  payload:   Record<string, unknown>,
): Promise<T> {
  const pool = ensurePool();
  const slot = pool[workerIdx % pool.length];
  const id = _nextId++;
  return new Promise<T>((resolve, reject) => {
    slot.pending.set(id, {
      resolve: resolve as (r: SliceResult) => void,
      reject,
    });
    slot.worker.postMessage({ id, type: "tagged", payload });
  });
}

/** Split `items` into `n` contiguous chunks of as-even-as-possible sizes. */
export function chunkInto<T>(items: readonly T[], n: number): T[][] {
  if (n <= 1 || items.length <= 1) return [items.slice()];
  const m = Math.min(n, items.length);
  const out: T[][] = Array.from({ length: m }, () => []);
  // Round-robin assignment so each chunk gets roughly the same workload
  // even when candidates' SA cost varies slightly.
  for (let i = 0; i < items.length; i++) {
    out[i % m].push(items[i]);
  }
  return out;
}
