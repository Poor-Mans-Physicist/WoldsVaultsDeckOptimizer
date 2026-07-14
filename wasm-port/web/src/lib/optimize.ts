// High-level orchestrator: candidate-core enumeration + wasm SA dispatch +
// breakdown re-score. Mirrors `optimize_inventory` in
// src/inventory_optimize.py.
//
// Split into three pieces so the main thread can fan out candidates across
// a pool of workers:
//
//   * `enumerateCandidates(input)` (sync, main thread): pre-flight + builds
//     candidate combos.
//   * `optimizeInventorySlice(input, candidatesSlice)` (async, worker):
//     runs WASM SA for a slice of candidates and optionally a per-candidate
//     returning the slice's best.
//   * `finalizeResult(input, sliceBest)` (sync, main thread): materializes
//     the per-slot Map and runs the TS-side breakdown re-score.
//
// `optimizeInventory(input)` is the legacy single-thread path: it composes
// all three sequentially. The parallel orchestrator lives in workerClient.ts.

import init, { runSaInventory, runSaTagged, scoreTagged } from "../wasm/ndm_core";
import wasmUrl from "../wasm/ndm_core_bg.wasm?url";

import {
  CardClass,
  type CoreSpec, type Placed,
} from "./types";
import type { Deck } from "./deck";
import type { ResolvedConfig } from "./config";
import { candidateCoresInventory } from "./cores";
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

/**
 * Strip stack entries whose CardType is in `hidden`. Returns the filtered
 * dict plus the total count of cards that were dropped.
 *
 * The GUI's row visibility is just a render gate; without this filter, cards
 * stocked while in another (mode, class) — e.g. positional shiny cards added
 * during a Wold's run — would silently land in the SA's candidate pool when
 * the user flipped to a mode/class that hides those rows. Apply this right
 * before passing inventory to the optimizer to keep behavior consistent with
 * what's on screen.
 */
export function filterInventoryByHidden(
  inv: InventoryCounts,
  hidden: Set<string>,
): { kept: InventoryCounts; dropped: number } {
  const kept: InventoryCounts = {};
  let dropped = 0;
  for (const [k, v] of Object.entries(inv)) {
    const [t, _c] = parseStackKey(k);
    if (hidden.has(t)) {
      if (v > 0) dropped += v;
    } else {
      kept[k] = v;
    }
  }
  return { kept, dropped };
}

export interface OptimizeInput {
  deck:        Deck;
  cardClass:   CardClass;
  inventory:   InventoryCounts;        // regular pool (per-(type,color) upper bound)
  /** Forced pool — per-(type, color) lower bound. Cap = inventory + forced. */
  forcedCounts: InventoryCounts;
  /** Lower bound on placed stat-giving cards. Stat-giving =
   *  ROW/COL/SURR/DIAG/DELUXE/TYPELESS (i.e. anything non-greed, non-arcane,
   *  non-dead). 0 = unconstrained. */
  minRegularPlaced: number;
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
}

let _wasmReady: Promise<void> | null = null;

/** Idempotent wasm boot. Each thread/worker has its own _wasmReady. */
export async function initWasm(): Promise<void> {
  if (_wasmReady) return _wasmReady;
  _wasmReady = init({ module_or_path: wasmUrl }).then(() => undefined);
  return _wasmReady;
}

// ── Optimizer 2.0 kernel entries ─────────────────────────────────────────────

/** Raw per-slot card as the tagged kernel returns it. */
export interface RawTaggedPlaced {
  t: string; color: string; scale_color: string; groups: string[];
}

/** One tagged SA run (a candidate combo × a restart chunk). Worker-side. */
export async function runTaggedPayload(
  payload: Record<string, unknown>,
): Promise<{ assignment: RawTaggedPlaced[]; score: number }> {
  await initWasm();
  return runSaTagged(payload) as { assignment: RawTaggedPlaced[]; score: number };
}

/** Score-only pass on a fixed assignment (what-if popup cross-check). */
export async function scoreTaggedPayload(
  payload: Record<string, unknown>,
): Promise<number> {
  await initWasm();
  return scoreTagged(payload) as number;
}

// ──────────────────────────────────────────────────────────────────────────────
// Phase 1 — candidate enumeration + pre-flight (sync, main thread)
// ──────────────────────────────────────────────────────────────────────────────

export function enumerateCandidates(input: OptimizeInput): CandidateBundle {
  const { deck, cardClass, inventory, forcedCounts, minRegularPlaced,
          autoPlaceArcane, cores, cfg } = input;

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

  // Minimum-stat-giving feasibility. Stat-giving = ROW/COL/SURR/DIAG/DELUXE/
  // TYPELESS (everything that isn't a greed, ARCANE, or DEAD). The constraint
  // is infeasible iff we don't own enough stat cards OR not enough non-arcane
  // slots are free after the forced non-stat cards land.
  const minStat = minRegularPlaced | 0;
  if (minStat > 0) {
    const isStat = (t: string) =>
      t === "row" || t === "col" || t === "surr" || t === "diag" ||
      t === "deluxe" || t === "typeless";

    let availStat = 0;
    let forcedNonStatNonArcane = 0;
    for (const [k, n] of Object.entries(inventory)) {
      if (n <= 0) continue;
      const [t] = parseStackKey(k);
      if (isStat(t)) availStat += n;
    }
    for (const [k, n] of Object.entries(forcedCounts ?? {})) {
      if (n <= 0) continue;
      const [t] = parseStackKey(k);
      if (isStat(t))            availStat += n;
      else if (t !== "arcane")  forcedNonStatNonArcane += n;
    }

    const reservedArcane = autoPlaceArcane ? deck.arcaneSlots.length : 0;
    const slotCap = Math.max(0, deck.slots.length - reservedArcane - forcedNonStatNonArcane);
    if (minStat > availStat) {
      throw new Error(
        `Minimum stat-giving (${minStat}) exceeds available stat-giving ` +
        `inventory (${availStat}) — raise inventory counts or lower the minimum.`,
      );
    }
    if (minStat > slotCap) {
      throw new Error(
        `Minimum stat-giving (${minStat}) exceeds the ${slotCap} non-arcane ` +
        `slots left after forced placements — lower the minimum or remove forced cards.`,
      );
    }
  }

  let candidates = candidateCoresInventory(
    cores, cardClass, deck.core_slots, deck.slots.length,
    deck.arcaneSlots.length, cfg,
  );
  if (candidates.length === 0) candidates = [[]];   // run with no cores

  return { candidates };
}

// ──────────────────────────────────────────────────────────────────────────────
// Phase 2 — run a slice of candidates through WASM (async, worker thread)
// ──────────────────────────────────────────────────────────────────────────────

function buildBasePayload(input: OptimizeInput) {
  const { deck, cardClass, inventory, forcedCounts, minRegularPlaced,
          autoPlaceArcane, nIter, restarts, cfg } = input;
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
    min_regular_placed: Math.max(0, minRegularPlaced | 0),
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
    mult_sparkling:         cfg.cores.sparkling,
    mult_color:             cfg.cores.color,
    mult_deluxe_flat:       cfg.deluxe.flat,
    mult_deluxe_core_base:  cfg.deluxe.core_base,
    mult_deluxe_core_scale: cfg.deluxe.core_scale,
    mult_void_core_base:    cfg.cores.void_base,
    mult_void_core_scale:   cfg.cores.void_scale,
    mult_archive_core:      cfg.cores.archive_core,
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
): Promise<SliceResult> {
  await initWasm();
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

    const asgn  = out.assignment;
    const score = out.score;
    if (score > bestScore) {
      bestScore  = score;
      bestAssign = asgn;
      bestCores  = combo;
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
  const { candidates } = enumerateCandidates(input);
  const slice = await optimizeInventorySlice(input, candidates);
  return finalizeResult(input, slice);
}
