// Optimization worker. Owns one wasm instance. Listens for "slice" messages
// from the workerPool — each one runs `optimizeInventorySlice` for a chunk
// of candidate combos and replies with the slice's best assignment.
//
// The main thread (workerClient.ts) does the candidate enumeration and the
// final breakdown re-score; this worker only does the WASM-heavy SA work
// plus any per-candidate pluto-swap.

import {
  optimizeInventorySlice,
  type OptimizeInput, type SliceResult,
} from "../lib/optimize";
import type { CoreSpec } from "../lib/types";

interface SliceMsg {
  id:   number;
  type: "slice";
  input: OptimizeInput;
  candidatesSlice: CoreSpec[][];
  plutoSpec: CoreSpec | null;
}

interface ReplyOk  { id: number; ok: true;  result: SliceResult; }
interface ReplyErr { id: number; ok: false; error: string; }

self.onmessage = async (ev: MessageEvent<SliceMsg>) => {
  const { id, type, input, candidatesSlice, plutoSpec } = ev.data;
  try {
    if (type !== "slice") {
      throw new Error(`unknown message type: ${type}`);
    }
    const result = await optimizeInventorySlice(input, candidatesSlice, plutoSpec);
    const reply: ReplyOk = { id, ok: true, result };
    (self as unknown as Worker).postMessage(reply);
  } catch (err) {
    const reply: ReplyErr = {
      id, ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
    (self as unknown as Worker).postMessage(reply);
  }
};
