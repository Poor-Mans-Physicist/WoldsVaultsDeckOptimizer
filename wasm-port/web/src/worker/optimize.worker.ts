// Optimization worker. Owns one wasm instance. Listens for "slice" messages
// from the workerPool — each one runs `optimizeInventorySlice` for a chunk
// of candidate combos and replies with the slice's best assignment.
//
// The main thread (workerClient.ts) does the candidate enumeration and the
// final breakdown re-score; this worker only does the WASM-heavy SA work.

import {
  optimizeInventorySlice, runTaggedPayload,
  type OptimizeInput, type SliceResult, type RawTaggedPlaced,
} from "../lib/optimize";
import type { CoreSpec } from "../lib/types";

interface SliceMsg {
  id:   number;
  type: "slice";
  input: OptimizeInput;
  candidatesSlice: CoreSpec[][];
}

/** Optimizer 2.0: one candidate combo × a restart chunk, pre-built payload. */
interface TaggedMsg {
  id:   number;
  type: "tagged";
  payload: Record<string, unknown>;
}

export interface TaggedTaskResult {
  assignment: RawTaggedPlaced[];
  score:      number;
}

interface ReplyOk  { id: number; ok: true;  result: SliceResult | TaggedTaskResult; }
interface ReplyErr { id: number; ok: false; error: string; }

self.onmessage = async (ev: MessageEvent<SliceMsg | TaggedMsg>) => {
  const { id, type } = ev.data;
  try {
    let result: SliceResult | TaggedTaskResult;
    if (type === "slice") {
      const { input, candidatesSlice } = ev.data as SliceMsg;
      result = await optimizeInventorySlice(input, candidatesSlice);
    } else if (type === "tagged") {
      const { payload } = ev.data as TaggedMsg;
      result = await runTaggedPayload(payload);
    } else {
      throw new Error(`unknown message type: ${type}`);
    }
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
