// High-level orchestrator: candidate-core enumeration + wasm SA dispatch +
// breakdown re-score + pluto post-SA swap. Mirrors `optimize_inventory` in
// src/inventory_optimize.py.
//
// Split into three pieces so the main thread can fan out candidates across
// a pool of workers:
//
//   * `enumerateCandidates(input)` (sync, main thread): pre-flight + builds
//     candidate combos + decides if pluto-swap is eligible.
//   * `optimizeInventorySlice(input, candidatesSlice)` (async, worker):
//     runs WASM SA for a slice of candidates and optionally a per-candidate
//     pluto-swap, returning the slice's best.
//   * `finalizeResult(input, sliceBest)` (sync, main thread): materializes
//     the per-slot Map and runs the TS-side breakdown re-score.
//
// `optimizeInventory(input)` is the legacy single-thread path: it composes
// all three sequentially. The parallel orchestrator lives in workerClient.ts.

import init, { runSaInventory } from "../wasm/ndm_core";
import wasmUrl from "../wasm/ndm_core_bg.wasm?url";

import {
  CardClass, CardType, CoreType,
  GREED_TYPES, POSITIONAL_TYPES,
  type CoreSpec, type Placed,
} from "./types";
import type { Deck } from "./deck";
import type { ResolvedConfig } from "./config";
import {
  candidateCoresInventory, pureMult, deluxeCoreMult, staticMult,
} from "./cores";
import { simulateInventoryBreakdown, type BreakdownResult } from "./breakdown";

export interface InventoryCounts {
  // Map key = `${cardType}|${color}` so we can serialize across worker boundary.
  // Browser callers use the typed helper `setCount` to update.
  [stack: string]: number;
}

export function stackKey(t: string, c: string): string { return `${t}|${c}`; }
export function parseStackKey(k: string): [string, string] {
  const i = k.indexOf("|");
  return [k.slice(0, i), k.slice(i + 1)];
}

export interface OptimizeInput {
  deck:        Deck;
  cardClass:   CardClass;
  inventory:   InventoryCounts;        // regular pool (per-(type,color) upper bound)
  /** Forced pool — per-(type, color) lower bound. Cap = inventory + forced. */
  forcedCounts: InventoryCounts;
  /** true = arcane slots locked to ARCANE; false = SA may swap to DEAD for void. */
  autoPlaceArcane: boolean;
  cores:       CoreSpec[];             // user's available cores (CoreInventory)
  nIter:       number;
  restarts:    number;
  cfg:         ResolvedConfig;
}

export interface OptimizeResult {
  // Per-slot placed cards as a Map keyed by `${r},${c}`.
  assignment:  Map<string, Placed>;
  /** Best-combo NDM from the wasm SA — canonical (it's also what we re-score). */
  wasmScore:   number;
  /** TS-side re-score of the wasm assignment; should equal wasmScore within 1e-6. */
  tsScore:     number;
  coresUsed:   CoreSpec[];
  breakdown:   BreakdownResult;
}

/** What one worker returns after running its slice — pre-breakdown. */
export interface SliceResult {
  score:     number;
  assignment: [string, string][];      // parallel to deck.slots
  cores:     CoreSpec[];
}

/** Output of `enumerateCandidates` — captures everything the worker needs to
 *  invoke `optimizeInventorySlice` without re-running pre-flight. */
export interface CandidateBundle {
  candidates: CoreSpec[][];
  plutoSpec:  CoreSpec | null;         // null if pluto-swap is not eligible
}

let _wasmReady: Promise<void> | null = null;

/** Idempotent wasm boot. Each thread/worker has its own _wasmReady. */
export async function initWasm(): Promise<void> {
  if (_wasmReady) return _wasmReady;
  _wasmReady = init({ module_or_path: wasmUrl }).then(() => undefined);
  return _wasmReady;
}

// ──────────────────────────────────────────────────────────────────────────────
// Phase 1 — candidate enumeration + pre-flight (sync, main thread)
// ──────────────────────────────────────────────────────────────────────────────

export function enumerateCandidates(input: OptimizeInput): CandidateBundle {
  const { deck, cardClass, inventory, forcedCounts, cores, cfg } = input;

  // Quick pre-flight (also done in the slice runner — duplication is cheap and
  // we want to fail fast on the main thread before fanning out workers).
  let totalForced = 0;
  let forcedArcane = 0;
  let anyRegular = false;
  for (const [, n] of Object.entries(inventory)) {
    if (n > 0) { anyRegular = true; break; }
  }
  for (const [k, n] of Object.entries(forcedCounts ?? {})) {
    if (n <= 0) continue;
    const [t] = parseStackKey(k);
    totalForced += n;
    if (t === "arcane") forcedArcane += n;
  }
  if (!anyRegular && totalForced === 0) {
    throw new Error("Inventory is empty — set some card counts first.");
  }
  if (totalForced > deck.slots.length) {
    throw new Error(
      `Forced inventory (${totalForced} cards) exceeds deck capacity ` +
      `(${deck.slots.length} slots) — remove some forced entries.`,
    );
  }
  if (forcedArcane > deck.arcaneSlots.length) {
    throw new Error(
      `Forced ARCANE inventory (${forcedArcane} cards) exceeds arcane slot ` +
      `count (${deck.arcaneSlots.length}) — arcane cards can only go in arcane slots.`,
    );
  }

  let candidates = candidateCoresInventory(
    cores, cardClass, deck.core_slots, deck.slots.length, cfg,
  );
  if (candidates.length === 0) candidates = [[]];   // run with no cores

  // Pluto-swap eligibility for the post-SA cheap check.
  let plutoSpec: CoreSpec | null = null;
  if (cfg.pluto.allow && cardClass === CardClass.EVO) {
    plutoSpec = cores.find((s) => s.core_type === CoreType.PLUTO_CORE) ?? null;
  }

  return { candidates, plutoSpec };
}

// ──────────────────────────────────────────────────────────────────────────────
// Phase 2 — run a slice of candidates through WASM (async, worker thread)
// ──────────────────────────────────────────────────────────────────────────────

function buildBasePayload(input: OptimizeInput) {
  const { deck, cardClass, inventory, forcedCounts, autoPlaceArcane,
          nIter, restarts, cfg } = input;
  const invList: [string, string, number][] = [];
  for (const [k, n] of Object.entries(inventory)) {
    if (n <= 0) continue;
    const [t, c] = parseStackKey(k);
    invList.push([t, c, n]);
  }
  const forcedList: [string, string, number][] = [];
  for (const [k, n] of Object.entries(forcedCounts ?? {})) {
    if (n <= 0) continue;
    const [t, c] = parseStackKey(k);
    forcedList.push([t, c, n]);
  }
  return {
    slots:      deck.slots.map(([r, c]) => [r, c] as [number, number]),
    row_peers:  deck.rowPeers,
    col_peers:  deck.colPeers,
    surr_peers: deck.surrPeers,
    diag_peers: deck.diagPeers,
    arcane_slot_indices: deck.arcaneSlotIndices,
    auto_place_arcane:   autoPlaceArcane,
    is_shiny:   cardClass === "shiny",
    inventory:  invList,
    forced_inventory: forcedList,
    n_iter:     nIter,
    restarts:   restarts,
    mult_dir_vert:          cfg.greed.dir_vert,
    mult_dir_horiz:         cfg.greed.dir_horiz,
    mult_evo_greed:         cfg.greed.evo,
    mult_surr_greed:        cfg.greed.surr,
    mult_dir_diag_up:       cfg.greed.dir_diag_up,
    mult_dir_diag_down:     cfg.greed.dir_diag_down,
    mult_pure_base:         cfg.cores.pure_base,
    mult_pure_scale:        cfg.cores.pure_scale,
    mult_equilibrium:       cfg.cores.equilibrium,
    mult_foil:              cfg.cores.foil,
    mult_steadfast:         cfg.cores.steadfast,
    mult_color:             cfg.cores.color,
    mult_deluxe_flat:       cfg.deluxe.flat,
    mult_deluxe_core_base:  cfg.deluxe.core_base,
    mult_deluxe_core_scale: cfg.deluxe.core_scale,
    mult_void_core_base:    cfg.cores.void_base,
    mult_void_core_scale:   cfg.cores.void_scale,
    mult_pluto:             cfg.pluto.multiplier,
    greed_additive:         cfg.stacking.greed_additive,
    additive_cores:         cfg.stacking.additive_cores,
  };
}

/**
 * Run WASM SA for a slice of candidate combos and return the best assignment
 * from this slice. No final breakdown — that's `finalizeResult`'s job.
 *
 * Called from `optimize.worker.ts` (per-worker slice) or from
 * `optimizeInventory` (legacy single-thread fallback).
 */
export async function optimizeInventorySlice(
  input: OptimizeInput,
  candidatesSlice: CoreSpec[][],
  plutoSpec: CoreSpec | null,
): Promise<SliceResult> {
  await initWasm();
  const { deck, cardClass, cfg } = input;
  const basePayload = buildBasePayload(input);

  let bestScore = -1;
  let bestAssign: [string, string][] = [];
  let bestCores: CoreSpec[] = [];

  for (const combo of candidatesSlice) {
    const corePayload: [string, string, number][] = combo.map((s) => [
      s.core_type,
      s.color ?? "",
      s.override === null ? -1.0 : s.override,
    ]);
    const out = runSaInventory({ ...basePayload, cores: corePayload }) as {
      assignment: [string, string][];
      score:      number;
    };

    let asgn   = out.assignment;
    let score  = out.score;
    let coresUsed: CoreSpec[] = combo;

    // Post-SA pluto swap: only on deluxe-free + pluto-free combos.
    const hasDeluxe = combo.some((s) => s.core_type === CoreType.DELUXE_CORE);
    const hasPluto  = combo.some((s) => s.core_type === CoreType.PLUTO_CORE);
    if (plutoSpec !== null && !hasDeluxe && !hasPluto) {
      const swap = tryPlutoSwap(deck, asgn, score, combo, plutoSpec, cardClass, cfg);
      asgn      = swap.asgn;
      score     = swap.score;
      coresUsed = swap.cores;
    }

    if (score > bestScore) {
      bestScore  = score;
      bestAssign = asgn;
      bestCores  = coresUsed;
    }
  }

  return { score: bestScore, assignment: bestAssign, cores: bestCores };
}

// ──────────────────────────────────────────────────────────────────────────────
// Phase 3 — materialize result + final breakdown (sync, main thread)
// ──────────────────────────────────────────────────────────────────────────────

/** Materialize the slice winner into the full OptimizeResult (with per-slot
 *  breakdown for the UI). Called once after all slices return. */
export function finalizeResult(
  input: OptimizeInput,
  slice: SliceResult,
): OptimizeResult {
  const { deck, cardClass, cfg } = input;
  const asgnMap = new Map<string, Placed>();
  for (let i = 0; i < deck.slots.length; i++) {
    const [tStr, cStr] = slice.assignment[i];
    asgnMap.set(
      `${deck.slots[i][0]},${deck.slots[i][1]}`,
      [tStr as any, (cStr ? cStr : null) as any],
    );
  }
  const breakdown = simulateInventoryBreakdown(deck, asgnMap, cardClass, slice.cores, cfg);
  return {
    assignment: asgnMap,
    wasmScore:  slice.score,
    tsScore:    breakdown.total,
    coresUsed:  slice.cores,
    breakdown,
  };
}

// ──────────────────────────────────────────────────────────────────────────────
// Legacy single-thread entry (used as a fallback / for the parity script).
// The parallel orchestrator in workerClient.ts uses the three pieces above.
// ──────────────────────────────────────────────────────────────────────────────

export async function optimizeInventory(input: OptimizeInput): Promise<OptimizeResult> {
  const { candidates, plutoSpec } = enumerateCandidates(input);
  const slice = await optimizeInventorySlice(input, candidates, plutoSpec);
  return finalizeResult(input, slice);
}

// ──────────────────────────────────────────────────────────────────────────────
// Post-SA pluto-swap (cheap check for deluxe-free, pluto-free EVO permutations)
// ──────────────────────────────────────────────────────────────────────────────
//
// Port of `_try_pluto_swap_inventory` in src/inventory_optimize.py.
// VOID and PLUTO are never swap targets here: void's value depends on the
// pre-SA n_dead so swapping it out of a converged assignment may invalidate
// the score; pluto can't be swapped for itself.

const PLUTO_SWAP_INELIGIBLE = new Set<CoreType>([CoreType.VOID_CORE, CoreType.PLUTO_CORE]);

function tryPlutoSwap(
  deck:       Deck,
  asgn:       [string, string][],
  score:      number,
  cores:      readonly CoreSpec[],
  plutoSpec:  CoreSpec,
  cardClass:  CardClass,
  cfg:        ResolvedConfig,
): { asgn: [string, string][]; score: number; cores: CoreSpec[] } {
  const swappable = cores.filter((s) => !PLUTO_SWAP_INELIGIBLE.has(s.core_type));
  if (swappable.length === 0) return { asgn, score, cores: [...cores] };

  // Count post-SA quantities for analytic per-core value computation.
  let nGreed = 0, nRegular = 0, nDeluxe = 0, nTypeless = 0, nArcane = 0;
  for (const [t] of asgn) {
    const ct = t as CardType;
    if (POSITIONAL_TYPES.has(ct))     nRegular++;
    else if (ct === CardType.DELUXE)  nDeluxe++;
    else if (ct === CardType.TYPELESS) nTypeless++;
    else if (ct === CardType.ARCANE)  nArcane++;
    else if (GREED_TYPES.has(ct))     nGreed++;
  }
  const foilActive = cores.some((s) => s.core_type === CoreType.FOIL);
  // Mirrors simulate_inventory's n_ns rule. ARCANE always counts; on top of
  // that EVO-no-FOIL also counts every other non-shiny placement.
  const n_ns = (cardClass === CardClass.EVO && !foilActive)
    ? nRegular + nDeluxe + nTypeless + nArcane + nGreed
    : nGreed + nArcane;

  const coreVal = (spec: CoreSpec): number => {
    if (spec.core_type === CoreType.PURE)        return pureMult(spec, n_ns, cfg);
    if (spec.core_type === CoreType.DELUXE_CORE) return deluxeCoreMult(spec, nDeluxe, cfg);
    return staticMult(spec, cfg);   // COLOR / FOIL / EQUI / STEADFAST
  };

  // Pick weakest swappable core.
  let weakest = swappable[0];
  let weakestVal = coreVal(weakest);
  for (let i = 1; i < swappable.length; i++) {
    const v = coreVal(swappable[i]);
    if (v < weakestVal) { weakest = swappable[i]; weakestVal = v; }
  }

  // Build candidate cores: (cores - weakest) ∪ plutoSpec.
  const altCores: CoreSpec[] = cores.filter((s) => s !== weakest).concat(plutoSpec);

  // Re-score on the same assignment. The breakdown call mirrors WASM's
  // simulate() math exactly so we don't need to round-trip through WASM.
  const asgnMap = new Map<string, Placed>();
  for (let i = 0; i < deck.slots.length; i++) {
    const [tStr, cStr] = asgn[i];
    asgnMap.set(
      `${deck.slots[i][0]},${deck.slots[i][1]}`,
      [tStr as any, (cStr ? cStr : null) as any],
    );
  }
  const altScore = simulateInventoryBreakdown(deck, asgnMap, cardClass, altCores, cfg).total;
  if (altScore > score) {
    return { asgn, score: altScore, cores: altCores };
  }
  return { asgn, score, cores: [...cores] };
}
