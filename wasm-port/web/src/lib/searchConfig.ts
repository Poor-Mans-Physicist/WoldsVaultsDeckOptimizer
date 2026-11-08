// SA schedule and core-set search budgets for the tagged kernel (web channel).
//
// The phase-1 / phase-2 target schedules start from the spreadsheet CLI's calibration (config.yaml `testing`,
// scripts/bench_search.py 2026-10-15) and are re-checked for the web budgets by wasm-port/scripts/bench_web.mjs.

import { Depth } from "./types";

export interface Schedule {
  kind: "geometric" | "target";
  params: number[];
}

/** One kernel call's annealing budget. `phase2` makes the kernel re-anneal its own top ends. */
export interface SearchBudget {
  nIter: number;
  schedule: Schedule | null;      // null = the kernel's legacy geometric schedule
  phase2?: { topK: number; nIter: number; schedule: Schedule } | null;
}

/** Payload fields for a budget (`schedule` / `phase2`); empty = the legacy schedule. */
export function searchSchedule(budget?: SearchBudget): Record<string, unknown> {
  if (!budget || !budget.schedule) return {};
  const out: Record<string, unknown> = { schedule: budget.schedule };
  if (budget.phase2) {
    out.phase2 = { top_k: budget.phase2.topK, n_iter: budget.phase2.nIter, schedule: budget.phase2.schedule };
  }
  return out;
}

export const SCHEDULE_PHASE1: Schedule = { kind: "target", params: [0.5, 0.001, 0.02, 0.01] };
export const SCHEDULE_PHASE2: Schedule = { kind: "target", params: [0.1, 0.001, 0.005, 0.01] };

/**
 * Core-set search budgets per depth:
 *  - screen: every set the beam visits gets `restarts × nIter`;
 *  - beam:   sets kept per size level;
 *  - halve:  the best `pool` full-size sets get a second screen; those within `margin` of the leader (at least
 *            `keepMin`, at most `keepMax`) go on;
 *  - full:   survivors get `restarts × nIter`, then their `topK` ends are re-annealed `p2Iter` steps on the
 *            phase-2 schedule.
 */
export interface SearchDepth {
  /** `warm`: walks of an extended set start from its parent set's best screened layout. */
  screen: { restarts: number; nIter: number; warm?: boolean };
  beam: number;
  /** Keep the best set holding each coordination core (Pluto, Jupiter, Nitwit, adjacency cores) in the beam and the
   *  halving pool even when its screen ranks it lower. */
  diverse?: boolean;
  halve: { pool: number; restarts: number; nIter: number; margin: number; keepMin: number; keepMax: number };
  full: { restarts: number; nIter: number; topK: number; p2Iter: number };
}

// Presets tuned on bench_web.ts (wasm-port/scripts/BENCH_WEB.md). 2026-11-08 shift: fast = the first port's default,
// default = its deep, deep = a ~4x heavier search (deeper core-set screening, 96 x 60k full restarts). Full panel
// (37 decks x evo/shiny x 2 reps, every core owned, 24 lanes): fast 1.44% mean gap to best-known at 0.98 s/run,
// default 1.17% at 2.01 s, deep 0.38% at 6.16 s. Partial rounds showed extra effort pays in long full-run walks
// backed by a deeper screen; spending it on core-set selection alone or on a lighter screen did worse.
export const SEARCH_DEPTHS: Record<Depth, SearchDepth> = {
  fast: {
    screen: { restarts: 2, nIter: 30_000, warm: true },
    beam: 8,
    diverse: true,
    halve: { pool: 10, restarts: 4, nIter: 40_000, margin: 0.05, keepMin: 3, keepMax: 4 },
    full: { restarts: 32, nIter: 30_000, topK: 6, p2Iter: 150_000 },
  },
  default: {
    screen: { restarts: 3, nIter: 40_000, warm: true },
    beam: 10,
    diverse: true,
    halve: { pool: 14, restarts: 6, nIter: 50_000, margin: 0.05, keepMin: 3, keepMax: 6 },
    full: { restarts: 48, nIter: 30_000, topK: 8, p2Iter: 150_000 },
  },
  deep: {
    screen: { restarts: 4, nIter: 80_000, warm: true },
    beam: 12,
    diverse: true,
    halve: { pool: 18, restarts: 8, nIter: 80_000, margin: 0.05, keepMin: 3, keepMax: 8 },
    full: { restarts: 96, nIter: 60_000, topK: 12, p2Iter: 300_000 },
  },
};
