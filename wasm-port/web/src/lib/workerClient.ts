// Main-thread orchestrator for parallel optimization. Splits the candidate
// list across the worker pool, fans out, gathers results, picks the global
// best, then runs the TS-side breakdown re-score on this thread.
//
// On a machine with N effective threads this drops the wall-clock time of
// runs with K >= N candidates by roughly N× compared to the serial path
// the PR used to ship.

import {
  enumerateCandidates, finalizeResult,
  type OptimizeInput, type OptimizeResult, type SliceResult,
} from "./optimize";
import { dispatchSlice, chunkInto, workerPoolSize } from "./workerPool";

/** Async wrapper — fans the candidate list across the worker pool. */
export async function optimizeInventoryAsync(
  input: OptimizeInput,
): Promise<OptimizeResult> {
  const _T0 = performance.now();

  // Phase 1 — pre-flight + enumerate (sync, this thread).
  const { candidates, plutoSpec } = enumerateCandidates(input);
  const _T_enum = performance.now();

  // Phase 2 — fan out across workers. chunkInto round-robins so each slice
  // is the same size up to ±1.
  const poolSize = workerPoolSize();
  const slices = chunkInto(candidates, poolSize);
  const _T_split = performance.now();

  const sliceResults: SliceResult[] = await Promise.all(
    slices.map((slice, i) => dispatchSlice(i, input, slice, plutoSpec)),
  );
  const _T_workers = performance.now();

  // Phase 3 — pick the global best across slice winners.
  let best: SliceResult = sliceResults[0];
  for (let i = 1; i < sliceResults.length; i++) {
    if (sliceResults[i].score > best.score) best = sliceResults[i];
  }

  // Phase 4 — materialize + breakdown re-score (sync, this thread).
  const result = finalizeResult(input, best);
  const _T_done = performance.now();

  // Diagnostic — surface how the wall-clock time was spent so regressions
  // are easy to spot. Remove once perf is satisfactory.
  console.log(
    `[optimize] total=${(_T_done - _T0).toFixed(0)}ms  ` +
    `enum=${(_T_enum - _T0).toFixed(0)}ms (${candidates.length} candidates)  ` +
    `split=${(_T_split - _T_enum).toFixed(0)}ms (${slices.length} slices)  ` +
    `workers=${(_T_workers - _T_split).toFixed(0)}ms  ` +
    `finalize=${(_T_done - _T_workers).toFixed(0)}ms`,
  );

  return result;
}
