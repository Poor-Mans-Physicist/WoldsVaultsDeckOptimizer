// Structural optimizer (web only): searches every Construction Core layout
// the game can roll for the player's deck, with the run's core combo and
// every other scoring input held fixed, and reports how rare the player's
// own layout is.
//
// Pipeline (all kernel work in the worker pool, see construction.rs):
//   1. enumerate  — every reachable layout for the roll's k range, grouped
//                   into reflection classes with exact per-roll probabilities
//   2. screen     — every class scored once with a cheap SA: classes in cell order, each
//                   warm-started from its job neighbour's best layout
//   3. search     — outer layout-space SA chains (whole chain in wasm), half of
//                   them started from the best screened classes
//   4. refine     — finalists re-scored deep; best rendered as a full result
//   5. rarity     — P(one roll beats the player's layout): classes the
//                   screen already proves better + the band just below the
//                   player's score (closest classes refined outright, the
//                   rest estimated by probability-weighted sampling)
//
// Every score is an achieved assignment, i.e. a lower bound on that layout's
// optimum, so cached scores only ever improve. The cache survives until any
// scoring input changes (fingerprint); moving construction slots keeps it.
//
// Every anneal uses the core search's target schedules (searchConfig.ts):
// screen walks and warm re-scores phase 1, deep scores phase 1 then phase 2
// from their best ends; chain steps re-anneal the parent layout's best cards
// (phase 2); band and resolve re-scores start from the class's best kept
// layout; finalists also get a walk from the best layout's cards moved onto
// their cells. Job counts
// and seeds never depend on the worker count, so a seed gives the same
// report on any machine; jobs go through one queue to whichever worker is free.

import type { Deck } from "./deck";
import type { CoreSpec, Position } from "./types";
import { OptimizerMode } from "./types";
import { activeImplicits, buildTaggedPayload, type TaggedRunInput } from "./tagged";
import { dispatchConstruction, dispatchTagged, workerPoolSize } from "./workerPool";
import { finalizeTaggedResult, type TaggedOptimizeResult } from "./taggedClient";
import type { RawTaggedPlaced } from "./optimize";
import { SCHEDULE_PHASE1, SCHEDULE_PHASE2, type Schedule } from "./searchConfig";

export type StructEffort = "standard" | "deep";

interface EffortParams {
  /** Screen classes in cell order, each warm-started from its job neighbour's best layout (else cold, in
   *  class order). */
  warmScreen:      boolean;
  /** Even-numbered chains start from the best screened classes in rank order (else every chain starts from
   *  a random roll), and every finalist also gets a walk from the best layout's cards moved onto its cells. */
  seeded:          boolean;
  /** Outer layout chains (fixed count: the report must not depend on the worker count). */
  chains:          number;
  /** The first `hotChains` chains run like the pre-2026-11-08 chains: random start, every step a cold walk of
   *  `chainIter` on the legacy hot schedule (T 100 → 0.5). They keep the search wide where warm chains settle
   *  into one card arrangement (bench_struct convergence test: bishop k=3). */
  hotChains:       number;
  chainSteps:      number;
  /** Cold budget of a random-start chain's first layout. */
  chainIter:       number;
  /** Warm budget of every later chain step (re-anneals the parent layout's best cards). */
  chainWarmIter:   number;
  screenIter:      number;
  screenRestarts:  number;
  refineTop:       number;
  /** Deep score: `refineRestarts × refineIter` cold walks, then the best `refineP2Top` ends re-annealed
   *  `refineP2Iter` steps on the phase-2 schedule. */
  refineIter:      number;
  refineRestarts:  number;
  refineP2Top:     number;
  refineP2Iter:    number;
  /** Band re-score: `bandRestarts` warm walks of `bandIter` from the class's best screened layout. */
  bandIter:        number;
  bandRestarts:    number;
  bandDet:         number;
  bandSamples:     number;
  /** Second pass: undecided band classes re-scored warm at this budget. */
  resolveIter:     number;
  resolveMargin:   number;
}

/** Search depths. The first presets came from the native lab study (scratch, 2026-09-24: Standard found the
 *  exhaustive optimum on 4/4 test decks; the 10k×1 screen gave 13–60 % median relative error on P(better), the
 *  30k×2 Deep screen 0–22 %). 2026-11-08: target schedules, phase 2 inside deep scores at the same step total
 *  (12×150k → 12×100k + 3×200k), warm chain steps and warm band / resolve re-scores at half the old steps.
 *  Later 2026-11-08, Standard only: the screen warm-starts each class from its job neighbour (classes sorted by
 *  cells), half the warm chains start from the best screened classes, finalists get a walk from the best
 *  layout's cards, and 12 of the 24 chains run hot (pre-port style). 5-seed convergence test on 4 hard problems
 *  vs the pre-port Standard: mean ideal gap 0.75 % vs 0.76 %, worst 2.1 % vs 2.6 %, exact best 8/20 vs 10/20
 *  (bishop k=3), player score 1.8 % vs 4.5 % low, rarity error 0.056 vs 0.096, 17.8 s vs 25.4 s. Deep got the
 *  same (24 of 48 chains hot): same test vs the cold-screen / warm-chain Deep, mean gap 0.32 % vs 0.90 %, worst
 *  1.07 % vs 6.85 %, exact best 12/20 vs 8/20, rarity error 0.028 vs 0.042, 63.4 s vs 58.9 s.
 *  Successive halving in place of the chains was tried and lost (bench_struct: 6/12 best layouts at Standard vs
 *  9/12): per-layout re-scores cannot find the card arrangements the chains carry from layout to layout. */
export const STRUCT_EFFORT: Record<StructEffort, EffortParams> = {
  standard: {
    warmScreen: true, seeded: true,
    chains: 24, hotChains: 12, chainSteps: 300, chainIter: 20_000, chainWarmIter: 8_000,
    screenIter: 10_000, screenRestarts: 1,
    refineTop: 10, refineIter: 100_000, refineRestarts: 12, refineP2Top: 3, refineP2Iter: 200_000,
    bandIter: 50_000, bandRestarts: 4, bandDet: 200, bandSamples: 400,
    resolveIter: 100_000, resolveMargin: 0.02,
  },
  deep: {
    warmScreen: true, seeded: true,
    chains: 48, hotChains: 24, chainSteps: 500, chainIter: 30_000, chainWarmIter: 12_000,
    screenIter: 30_000, screenRestarts: 2,
    refineTop: 16, refineIter: 200_000, refineRestarts: 24, refineP2Top: 6, refineP2Iter: 400_000,
    bandIter: 50_000, bandRestarts: 4, bandDet: 300, bandSamples: 600,
    resolveIter: 200_000, resolveMargin: 0.015,
  },
};

/** Jobs a batch of layouts is split into (fixed, so seeds never depend on the worker count). */
const EVAL_JOBS = 96;
/** Cold restarts per deep-score job. */
const DEEP_CHUNK = 2;
/** keep_above that returns every layout. */
const KEEP_ALL = -1e300;

/** Screen band: classes whose screen score is within this fraction below the
 *  player's score may still beat it (screen undershoot p99 ≈ 14–21 %). */
const BAND_MARGIN = 0.2;
/** Strata over the band (by screen score / player score) for the estimate. */
const BAND_STRATA = 5;
/** Band-refine undershoot (100k×4 p99 ≈ 2–5 %): refined scores this close
 *  below the player's may still truly beat it. */
const UNDECIDED_MARGIN = 0.03;

export interface StructInputs {
  /** Run input as the player's last optimize used it (deck ignored). */
  run:        TaggedRunInput;
  combo:      CoreSpec[];
  /** Stock deck with the player's arcane conversions, no construction slots. */
  baseDeck:   Deck;
  /** Builds the full deck for a set of added cells. */
  deckFor:    (added: Position[]) => Deck;
  userAdded:  Position[];
  /** The player's optimize score for their own layout. */
  userScore:  number;
  fingerprint: string;
}

export interface LayoutStat {
  added:    Position[];
  score:    number;
  /** P(one roll produces exactly this layout). */
  rollProb: number;
  /** Same, counting its mirror images (score-identical). */
  classProb: number;
}

export interface StructReport {
  tier:        string;
  ks:          number[];
  ideal:       LayoutStat;
  idealResult: TaggedOptimizeResult;
  user:        LayoutStat;
  /** user.score / ideal.score. */
  ofMax:       number;
  /** Estimated P(one roll beats the player's layout) + 1σ. */
  pBetter:     number;
  pBetterSe:   number;
  /** Range for P(better): sampling ±2σ plus the undecided mass. */
  pBetterLo:   number;
  pBetterHi:   number;
  /** Mass proven better by an achieved score (lower bound). */
  pBetterCert: number;
  /** P(one roll is at least as good as the player's layout). */
  pAtLeast:    number;
  classes:     number;
  sets:        number;
  elapsedMs:   number;
  fromCache:   boolean;
}

interface ClassRow { k: number; rep: number[]; members: number; prob: number }

/** A kernel layout as a warm start: [type, color, scale_color, groups, stack] per slot. */
type Start = unknown[];
interface PlacedOutRaw { t: string; color: string; scale_color: string; groups: string[]; stack: number }
const startOf = (a: PlacedOutRaw[]): Start => a.map((x) => [x.t, x.color, x.scale_color, x.groups, x.stack]);
interface EvalOut { scores: number[]; layouts: (PlacedOutRaw[] | null)[] }

/** A layout of added cells `from` carried onto `to` (same size, cells ascending; base slots first): shared
 *  cells keep their cards, vacated cells' cards fill the new cells in order (= construction_wasm.rs remap). */
function remapStart(nb: number, from: number[], layout: Start, to: number[]): Start | null {
  if (from.length !== to.length || layout.length !== nb + from.length) return null;
  const at = new Map(from.map((c, j) => [c, layout[nb + j]]));
  const toSet = new Set(to);
  const spare = from.filter((c) => !toSet.has(c)).map((c) => at.get(c));
  let s = 0;
  return [...layout.slice(0, nb), ...to.map((c) => (at.has(c) ? at.get(c) : spare[s++]))];
}

interface Cache {
  fingerprint: string;
  classes:     ClassRow[];
  setCounts:   number[];
  /** Best achieved score per class (lower bound). */
  best:        Float64Array;
  /** Screen budget (iterations × restarts) already applied to every class. */
  screened:    number;
  /** Classes already refined at the band budget. */
  bandDone:    Uint8Array;
  /** Resolve budget (iterations) already applied per class (0 = none). */
  resolved:    Float64Array;
  /** Best layout per class at its best score (kept for classes that may land in a rarity band). */
  layouts:     Map<number, Start>;
  /** Best achieved score per exact set key (chains, refines, the player). */
  sets:        Map<string, number>;
  idealKey:    string | null;
  idealResult: TaggedOptimizeResult | null;
  searchedAt:  StructEffort | null;
}

let _cache: Cache | null = null;

export function clearStructCache(): void { _cache = null; }

/** True when a finished pass for this scoring setup + tier is cached, so a
 *  hand-placed layout's report only needs the cheap rarity refinement. */
export function hasStructCache(fingerprint: string, k: number): boolean {
  const tier = tierFor(k);
  return tier !== null && _cache !== null && _cache.searchedAt !== null
    && _cache.fingerprint === `${fingerprint}|${tier.ks.join(",")}`;
}

/** The construction core's slot count is fixed on the core item (rolled
 *  once when the core is generated: lesser 1, normal 2–3, greater 4–5), and
 *  every anvil reroll places exactly that many — so the layout space is the
 *  placed count k only. The tier name is display-only. */
export function tierFor(k: number): { name: string; ks: number[] } | null {
  if (k === 1) return { name: "Lesser", ks: [1] };
  if (k === 2 || k === 3) return { name: "Normal", ks: [k] };
  if (k === 4 || k === 5) return { name: "Greater", ks: [k] };
  return null;
}

const idx = ([r, c]: Position) => r * 9 + c;
const pos = (i: number): Position => [Math.floor(i / 9), i % 9] as const;
const keyOf = (cells: number[]) => [...cells].sort((a, b) => a - b).join(",");

/** Reflections that keep every score identical (see construction.rs::canon).
 *  Exact stacks and directional Targeted rules can tell left from right. */
function symmetryFlags(run: TaggedRunInput): { allowX: boolean; allowY: boolean } {
  if (run.mode === OptimizerMode.EXACT) return { allowX: false, allowY: false };
  const ruled = (keys: string[]) => run.mode === OptimizerMode.TARGETED && run.targetedRules.some(
    (r) => r.axis === "type" && keys.includes(r.key) && (r.min !== null || r.max !== null));
  const allowX = !ruled(["dir_greed_left", "dir_greed_right"]);
  const cake = activeImplicits(run).some((i) => i.kind === "row_pos");
  const allowY = !cake && !ruled(["dir_greed_up", "dir_greed_down"]);
  return { allowX, allowY };
}

function baseCells(deck: Deck): [number, number, boolean][] {
  const arc = new Set(deck.arcaneSlots.map(([r, c]) => `${r},${c}`));
  return deck.slots.map(([r, c]) => [r, c, arc.has(`${r},${c}`)]);
}

/** Split `items` round-robin into `n` chunks (skip empties). */
function chunks<T>(items: T[], n: number): T[][] {
  const out: T[][] = Array.from({ length: Math.max(1, n) }, () => []);
  items.forEach((x, i) => out[i % out.length].push(x));
  return out.filter((c) => c.length > 0);
}

/** Run jobs `lanes` at a time from one queue (the pool sends each to a free worker); results in job order. */
async function runQueue<T>(jobs: (() => Promise<T>)[], lanes: number): Promise<T[]> {
  const out: T[] = new Array(jobs.length);
  let next = 0;
  const lane = async () => { while (next < jobs.length) { const i = next++; out[i] = await jobs[i](); } };
  await Promise.all(Array.from({ length: Math.max(1, Math.min(lanes, jobs.length)) }, lane));
  return out;
}

export async function runStructural(
  inp: StructInputs,
  effort: StructEffort,
  onProgress: (msg: string) => void,
): Promise<StructReport> {
  const t0 = performance.now();
  const P = STRUCT_EFFORT[effort];
  const pool = workerPoolSize();
  const k = inp.userAdded.length;
  const tier = tierFor(k);
  if (!tier) throw new Error(`${k} construction slots isn't a roll any Construction Core makes (1, 2–3 or 4–5).`);
  const kProbs = tier.ks.map(() => 1 / tier.ks.length);
  const { allowX, allowY } = symmetryFlags(inp.run);
  const base = baseCells(inp.baseDeck);
  const template = buildTaggedPayload({ ...inp.run, deck: inp.baseDeck }, inp.combo, 1,
    { nIter: P.screenIter, schedule: SCHEDULE_PHASE1 });
  const warmBudget = (nIter: number, schedule: Schedule) => ({ n_iter: nIter, schedule });
  /** Same run without a schedule: the kernel's legacy hot geometric schedule (hot chains). */
  const legacyTemplate = buildTaggedPayload({ ...inp.run, deck: inp.baseDeck }, inp.combo, 1);
  const common = { base, ks: tier.ks, k_probs: kProbs, allow_x: allowX, allow_y: allowY };
  const userCells = inp.userAdded.map(idx);
  const userKey = keyOf(userCells);

  // 1. enumerate (cached) + locate the player's layout
  const fingerprint = `${inp.fingerprint}|${tier.ks.join(",")}`;
  let fromCache = true;
  if (!_cache || _cache.fingerprint !== fingerprint) {
    fromCache = false;
    onProgress("Enumerating every possible roll…");
    const en = await dispatchConstruction<{ classes: ClassRow[]; set_counts: number[] }>(
      0, "cc_enum", { ...common, user_added: [] });
    _cache = {
      fingerprint, classes: en.classes, setCounts: en.set_counts,
      best: new Float64Array(en.classes.length).fill(-Infinity), screened: 0,
      bandDone: new Uint8Array(en.classes.length),
      resolved: new Float64Array(en.classes.length),
      layouts: new Map(), sets: new Map(), idealKey: null, idealResult: null, searchedAt: null,
    };
  }
  const cache = _cache;
  const locate = async (cells: Position[]) => dispatchConstruction<{
    user_legal: boolean; user_prob: number; user_class: number | null;
  }>(0, "cc_enum", { ...common, user_added: cells.map(([r, c]) => [r, c]), classes_out: false });
  const userLoc = await locate(inp.userAdded);
  if (!userLoc.user_legal || userLoc.user_class === null) {
    throw new Error("This construction layout can't be rolled by the game (slots must grow orthogonally inside cols 0–8, rows 0–5).");
  }

  /** Score a batch of layouts in EVAL_JOBS fixed jobs. `starts[i]` (optional) warm-starts set i with `warm`;
   *  layouts scoring at least `keepAbove` come back with their best cards. */
  const evalSets = async (sets: number[][], nIter: number, restarts: number, seed: number,
                          opt: { starts?: (Start | null)[]; warm?: { n_iter: number; schedule: Schedule };
                                 keepAbove?: number; chainWarm?: boolean } = {}) => {
    const items = sets.map((s, i) => ({ s, i }));
    const nJobs = Math.min(sets.length, EVAL_JOBS);
    const parts = opt.chainWarm
      ? Array.from({ length: nJobs }, (_, j) => items.slice(Math.floor(j * items.length / nJobs),
          Math.floor((j + 1) * items.length / nJobs))).filter((c) => c.length > 0)
      : chunks(items, nJobs);
    const scores = new Array<number>(sets.length);
    const layouts = new Array<Start | null>(sets.length).fill(null);
    await runQueue(parts.map((part, w) => () =>
      dispatchConstruction<EvalOut>(w, "cc_eval", {
        template, base, sets: part.map((x) => x.s), n_iter: nIter, restarts, seed: seed + w * 7919,
        ...(opt.starts ? { starts: part.map((x) => opt.starts![x.i] ?? null) } : {}),
        ...(opt.warm ? { warm: opt.warm } : {}),
        ...(opt.chainWarm ? { chain_warm: true } : {}),
        ...(opt.keepAbove !== undefined ? { keep_above: opt.keepAbove } : {}),
      }).then((r) => part.forEach((x, j) => {
        scores[x.i] = r.scores[j];
        const l = r.layouts[j];
        if (l) layouts[x.i] = startOf(l);
      }))), pool);
    return { scores, layouts };
  };
  /** Deep scores for several sets through one queue: DEEP_CHUNK-restart cold jobs (phase 1), then the best
   *  refineP2Top ends of each set re-annealed on the phase-2 schedule. */
  const deepScores = async (cellsList: number[][], seed: number, extra: (Start | null)[] = [],
  ): Promise<{ score: number; layout: Start | null }[]> => {
    const p1: { set: number; c: number }[] = [];
    cellsList.forEach((_, set) => {
      for (let c = 0; c * DEEP_CHUNK < P.refineRestarts; c++) p1.push({ set, c });
    });
    const p1Out = await runQueue(p1.map((j) => () => dispatchConstruction<EvalOut>(0, "cc_eval", {
      template, base, sets: [cellsList[j.set]], n_iter: P.refineIter,
      restarts: Math.min(DEEP_CHUNK, P.refineRestarts - j.c * DEEP_CHUNK),
      seed: seed + j.set * 104729 + j.c * 7919, keep_above: KEEP_ALL,
    })), pool);
    const ends: { score: number; layout: Start | null }[][] = cellsList.map(() => []);
    p1Out.forEach((r, i) => ends[p1[i].set].push({ score: r.scores[0], layout: r.layouts[0] ? startOf(r.layouts[0]!) : null }));
    const p2: { set: number; start: Start; j: number }[] = [];
    ends.forEach((list, set) => [...list].sort((a, b) => b.score - a.score).slice(0, P.refineP2Top)
      .forEach((e, j) => { if (e.layout) p2.push({ set, start: e.layout, j }); }));
    extra.forEach((s, set) => { if (s) p2.push({ set, start: s, j: P.refineP2Top }); });
    const p2Out = await runQueue(p2.map((j) => () => dispatchConstruction<EvalOut>(0, "cc_eval", {
      template, base, sets: [cellsList[j.set]], n_iter: P.refineP2Iter, restarts: 1,
      seed: seed + 0x51ED + j.set * 104729 + j.j * 7919,
      starts: [j.start], warm: warmBudget(P.refineP2Iter, SCHEDULE_PHASE2), keep_above: KEEP_ALL,
    })), pool);
    p2Out.forEach((r, i) => ends[p2[i].set].push({ score: r.scores[0], layout: r.layouts[0] ? startOf(r.layouts[0]!) : null }));
    return ends.map((list) => list.reduce((a, b) => (b.score > a.score ? b : a)));
  };
  const noteSet = (cells: number[], v: number) => {
    const key = keyOf(cells);
    cache.sets.set(key, Math.max(cache.sets.get(key) ?? -Infinity, v));
  };

  // 2. the player's own score (deep) — the rarity threshold
  onProgress("Scoring your layout deeply…");
  const [userDeepRes] = await deepScores([userCells], 11);
  const userDeep = userDeepRes.score;
  noteSet(userCells, Math.max(userDeep, inp.userScore));
  const T = cache.sets.get(userKey)!;
  if (T >= cache.best[userLoc.user_class] && userDeepRes.layout && userDeep >= T) {
    cache.layouts.set(userLoc.user_class, remapStart(base.length, userCells.slice().sort((a, b) => a - b),
      userDeepRes.layout, cache.classes[userLoc.user_class].rep) ?? userDeepRes.layout);
  }
  cache.best[userLoc.user_class] = Math.max(cache.best[userLoc.user_class], T);

  // 3. screen every class once (cached per budget)
  const screenBudget = P.screenIter * P.screenRestarts;
  if (cache.screened < screenBudget) {
    const order = cache.classes.map((_, i) => i);
    if (P.warmScreen) order.sort((a, b) => {
      const x = cache.classes[a].rep, y = cache.classes[b].rep;
      if (x.length !== y.length) return x.length - y.length;
      for (let j = 0; j < x.length; j++) if (x[j] !== y[j]) return x[j] - y[j];
      return 0;
    });
    onProgress(`Screening ${order.length.toLocaleString()} layouts…`);
    const sc = await evalSets(order.map((i) => cache.classes[i].rep), P.screenIter, P.screenRestarts,
      101 + screenBudget, P.warmScreen
        ? { keepAbove: T * (1 - BAND_MARGIN), chainWarm: true, warm: warmBudget(P.screenIter, SCHEDULE_PHASE1) }
        : { keepAbove: T * (1 - BAND_MARGIN) });
    sc.scores.forEach((v, j) => {
      const i = order[j];
      if (v > cache.best[i]) {
        cache.best[i] = v;
        const l = sc.layouts[j];
        if (l) cache.layouts.set(i, l);
      }
    });
    cache.screened = screenBudget;
  }

  /** Warm re-score of classes from their best kept layout. A class without one (its screen score fell below
   *  the kept-layout threshold, e.g. when the player's layout beats every class by a wide margin, or an earlier
   *  pass of this cache used a higher threshold) falls back to a cold re-score at twice the warm steps. */
  const rescore = async (ids: number[], nIter: number, restarts: number, seed: number, what: string) => {
    if (!ids.length) return;
    const starts = ids.map((i) => cache.layouts.get(i) ?? null);
    const cold = starts.filter((s) => s === null).length;
    if (cold) console.warn(`[structural] ${what}: ${cold}/${ids.length} re-scores have no kept layout, cold fallback`);
    const sc = await evalSets(ids.map((i) => cache.classes[i].rep), nIter * 2, restarts, seed, {
      starts, warm: warmBudget(nIter, SCHEDULE_PHASE1), keepAbove: KEEP_ALL,
    });
    sc.scores.forEach((v, j) => {
      const i = ids[j];
      if (v > cache.best[i]) {
        cache.best[i] = v;
        const l = sc.layouts[j];
        if (l) cache.layouts.set(i, l);
      }
    });
  };
  /** The best class with a kept layout: its cards seed other classes (a good card arrangement carries over
   *  between layouts that share most cells, which is what the old outer chains exploited). */
  const bestSource = (): { cells: number[]; layout: Start } | null => {
    let bi = -1;
    for (const i of cache.layouts.keys()) if (bi < 0 || cache.best[i] > cache.best[bi]) bi = i;
    return bi < 0 ? null : { cells: cache.classes[bi].rep, layout: cache.layouts.get(bi)! };
  };
  const ranked = () => cache.classes.map((_, i) => i).sort((a, b) => cache.best[b] - cache.best[a] || a - b);

  // 4. outer layout-space SA + deep refine of the finalists (cached per effort)
  const searched = cache.searchedAt === "deep" || cache.searchedAt === effort;
  if (!searched || !cache.idealResult) {
    onProgress(`Searching layouts (${P.chains} chains)…`);
    const seeds = ranked().filter((i) => tier.ks.includes(cache.classes[i].k));
    const chainRes = await runQueue(Array.from({ length: P.chains }, (_, c) => () => {
      if (c < P.hotChains) {
        return dispatchConstruction<{ set: number[]; score: number }[]>(c, "cc_chain", {
          template: legacyTemplate, base, k: tier.ks[c % tier.ks.length], steps: P.chainSteps,
          n_iter: P.chainIter, seed: 7000 + c * 31, keep: 8,
        });
      }
      const w = c - P.hotChains;
      const from = P.seeded && w % 2 === 0 ? seeds[w / 2] : undefined;
      const k = from !== undefined ? cache.classes[from].k : tier.ks[c % tier.ks.length];
      return dispatchConstruction<{ set: number[]; score: number }[]>(c, "cc_chain", {
        template, base, k, steps: P.chainSteps, n_iter: P.chainIter, seed: 7000 + c * 31, keep: 8,
        warm: warmBudget(P.chainWarmIter, SCHEDULE_PHASE2),
        ...(from !== undefined ? { start: cache.classes[from].rep } : {}),
      });
    }), pool);
    for (const list of chainRes) for (const v of list) noteSet(v.set, v.score);

    const byClass = ranked().slice(0, P.refineTop).map((i) => ({ set: cache.classes[i].rep }));
    const byChain = [...cache.sets.entries()].map(([key, score]) => ({
      set: key ? key.split(",").map(Number) : [], score,
    })).sort((a, b) => b.score - a.score).slice(0, P.refineTop);
    const finalists = new Map<string, number[]>();
    for (const f of [...byClass, ...byChain]) finalists.set(keyOf(f.set), f.set);
    finalists.set(userKey, userCells);
    onProgress(`Refining ${finalists.size} finalists…`);
    const toRefine = [...finalists].filter(([key]) => key !== userKey).map(([, cells]) => cells);
    const src = P.seeded ? bestSource() : null;
    const refined = await deepScores(toRefine, 3,
      toRefine.map((cells) => (src ? remapStart(base.length, src.cells, src.layout, cells) : null)));
    toRefine.forEach((cells, i) => noteSet(cells, refined[i].score));
    let bestKey = userKey;
    for (const key of finalists.keys()) {
      if ((cache.sets.get(key) ?? -Infinity) > (cache.sets.get(bestKey) ?? -Infinity)) bestKey = key;
    }
    cache.idealKey = bestKey;
    cache.idealResult = null;
    cache.searchedAt = effort;
  }

  // A hand-placed layout can beat the cached search's pick.
  if (cache.idealKey !== userKey && T > (cache.sets.get(cache.idealKey ?? "") ?? -Infinity)) {
    cache.idealKey = userKey;
    cache.idealResult = null;
  }

  // Render the ideal layout (full assignment + breakdown) once per search.
  const idealCells = cache.idealKey ? cache.idealKey.split(",").map(Number) : userCells;
  const idealAdded = idealCells.map(pos);
  if (!cache.idealResult) {
    onProgress("Rendering the ideal layout…");
    const deck = inp.deckFor(idealAdded);
    const run = { ...inp.run, deck };
    type Out = { assignment: (RawTaggedPlaced & { stack: number })[]; score: number };
    const nChunk = Math.ceil(P.refineRestarts / DEEP_CHUNK);
    const p1 = await runQueue(Array.from({ length: nChunk }, (_, w) => () => dispatchTagged<Out>(w, {
      ...buildTaggedPayload(run, inp.combo, Math.min(DEEP_CHUNK, P.refineRestarts - w * DEEP_CHUNK),
        { nIter: P.refineIter, schedule: SCHEDULE_PHASE1 }),
      seed: 900 + w,
    })), pool);
    const tops = [...p1].sort((a, b) => b.score - a.score).slice(0, P.refineP2Top);
    const p2 = await runQueue(tops.map((e, j) => () => dispatchTagged<Out>(j, {
      ...buildTaggedPayload(run, inp.combo, 1, { nIter: P.refineP2Iter, schedule: SCHEDULE_PHASE2 }),
      seed: 950 + j,
      starts: [e.assignment.map((x) => [x.t, x.color, x.scale_color, x.groups, x.stack])],
    })), pool);
    const top = [...p1, ...p2].reduce((a, b) => (b.score > a.score ? b : a));
    noteSet(idealCells, top.score);
    cache.idealResult = finalizeTaggedResult(run, inp.combo, top.assignment, top.score);
  }
  const idealScore = Math.max(cache.sets.get(keyOf(idealCells)) ?? -Infinity, cache.idealResult.wasmScore);

  // 5. rarity: P(one roll beats T). Classes already proven better count
  // exactly; the band just below T is resolved by refining its closest
  // classes outright and estimating the rest by stratified sampling.
  onProgress("Estimating roll rarity…");
  const better = (v: number) => v > T * (1 + 1e-9);
  const uc = userLoc.user_class;
  const refine = async (ids: number[], seed: number) => {
    const todo = ids.filter((i) => !cache.bandDone[i]);
    if (!todo.length) return;
    await rescore(todo, P.bandIter, P.bandRestarts, seed, "band");
    todo.forEach((i) => { cache.bandDone[i] = 1; });
  };
  const band: number[] = [];
  cache.classes.forEach((_, i) => {
    if (i !== uc && !better(cache.best[i]) && cache.best[i] >= T * (1 - BAND_MARGIN)) band.push(i);
  });
  band.sort((a, b) => cache.best[b] - cache.best[a]);
  const det = band.slice(0, P.bandDet);
  await refine(det, 303);
  const strata: number[][] = Array.from({ length: BAND_STRATA }, () => []);
  for (const i of band.slice(P.bandDet)) {
    const x = (1 - cache.best[i] / T) / BAND_MARGIN;
    strata[Math.min(BAND_STRATA - 1, Math.max(0, Math.floor(x * BAND_STRATA)))].push(i);
  }
  const perStratum = Math.floor(P.bandSamples / BAND_STRATA);
  let seed = 0x2545F491;
  const rnd = () => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) / 4294967296; };
  const sampled = new Set<number>();
  const draws: { mass: number; picks: number[]; exact: boolean; members: number[] }[] = [];
  for (const st of strata) {
    if (!st.length) continue;
    const mass = st.reduce((a, i) => a + cache.classes[i].prob, 0);
    if (st.length <= perStratum) {
      st.forEach((i) => sampled.add(i));
      draws.push({ mass, picks: [], exact: true, members: st });
      continue;
    }
    const cum: number[] = [];
    let acc = 0;
    for (const i of st) { acc += cache.classes[i].prob; cum.push(acc); }
    const picks: number[] = [];
    for (let s = 0; s < perStratum; s++) {
      const x = rnd() * mass;
      let lo = 0, hi = cum.length - 1;
      while (lo < hi) { const m = (lo + hi) >> 1; if (cum[m] < x) lo = m + 1; else hi = m; }
      picks.push(st[lo]);
      sampled.add(st[lo]);
    }
    draws.push({ mass, picks, exact: false, members: st });
  }
  await refine([...sampled], 505);

  // Resolve pass: evaluated classes that landed just below T get a deeper
  // re-score, and afterwards a tighter undecided margin.
  const firstMargin = (v: number) => !better(v) && v >= T * (1 - UNDECIDED_MARGIN);
  const evaluated = [...new Set([...det, ...sampled])];
  const toResolve = evaluated.filter((i) => firstMargin(cache.best[i]) && cache.resolved[i] < P.resolveIter);
  if (toResolve.length) {
    onProgress(`Resolving ${toResolve.length} close calls…`);
    await rescore(toResolve, P.resolveIter, P.bandRestarts, 707, "resolve");
    toResolve.forEach((i) => { cache.resolved[i] = P.resolveIter; });
  }

  let pCert = 0;
  cache.classes.forEach((c, i) => { if (i !== uc && better(cache.best[i])) pCert += c.prob; });
  const inStrata = new Set(draws.flatMap((d) => d.members));
  // Refined scores still undershoot a little, so classes landing just below
  // T are undecided: their mass widens the range's upper end only. (After
  // the resolve pass they are almost never truly better — on the runic+cake
  // mystery test the point estimate without them was within 1 % / 7 % of
  // exhaustive truth.)
  const undecidedAt = (i: number) => !better(cache.best[i]) && cache.best[i] >= T * (1 - (cache.resolved[i] > 0 ? P.resolveMargin : UNDECIDED_MARGIN));
  let pEst = 0, pUnd = 0, pVar = 0;
  cache.classes.forEach((c, i) => {
    if (i === uc || inStrata.has(i)) return;
    if (better(cache.best[i])) pEst += c.prob;
    else if (undecidedAt(i)) pUnd += c.prob;
  });
  for (const d of draws) {
    if (d.exact) {
      for (const i of d.members) {
        if (better(cache.best[i])) pEst += cache.classes[i].prob;
        else if (undecidedAt(i)) pUnd += cache.classes[i].prob;
      }
      continue;
    }
    const n = d.picks.length;
    const f = d.picks.filter((i) => better(cache.best[i])).length / n;
    const g = d.picks.filter((i) => undecidedAt(i)).length / n;
    pEst += d.mass * f;
    pUnd += d.mass * g;
    pVar += d.mass * d.mass * f * (1 - f) / n;
  }
  const pCertOnly = pCert;
  const pSe = Math.sqrt(pVar);
  const pBetter = Math.max(pCert, pEst);
  const pBetterLo = Math.max(pCert, pEst - 2 * pSe);
  const pBetterHi = pEst + pUnd + 2 * pSe;
  const pRest = draws.reduce((a, d) => a + (d.exact ? 0 : d.mass), 0);
  const userClass = cache.classes[uc];
  let pTie = userClass.prob;
  cache.classes.forEach((c, i) => {
    if (i !== uc && !better(cache.best[i]) && cache.best[i] >= T * (1 - 1e-9)) pTie += c.prob;
  });

  const idealLoc = await locate(idealAdded);
  const report: StructReport = {
    tier: tier.name,
    ks: tier.ks,
    ideal: {
      added: idealAdded, score: idealScore, rollProb: idealLoc.user_prob,
      classProb: idealLoc.user_class !== null ? cache.classes[idealLoc.user_class].prob : idealLoc.user_prob,
    },
    idealResult: cache.idealResult,
    user: { added: inp.userAdded, score: T, rollProb: userLoc.user_prob, classProb: userClass.prob },
    ofMax: T / Math.max(idealScore, T),
    pBetter, pBetterSe: pSe, pBetterCert: pCertOnly, pBetterLo, pBetterHi,
    pAtLeast: Math.min(1, pBetter + pTie),
    classes: cache.classes.length,
    sets: cache.setCounts.reduce((a, b) => a + b, 0),
    elapsedMs: performance.now() - t0,
    fromCache,
  };
  console.log(
    `[structural] ${effort} ${report.elapsedMs.toFixed(0)}ms  classes=${report.classes} ` +
    `T=${T.toFixed(3)} ideal=${idealScore.toFixed(3)} pBetter=${pBetter.toExponential(3)} ` +
    `[${pBetterLo.toExponential(2)}, ${pBetterHi.toExponential(2)}] (cert ${pCertOnly.toExponential(3)}, undecided ${pUnd.toExponential(2)}, ` +
    `band ${band.length}, sampled-rest mass ${pRest.toExponential(2)})`,
  );
  return report;
}
