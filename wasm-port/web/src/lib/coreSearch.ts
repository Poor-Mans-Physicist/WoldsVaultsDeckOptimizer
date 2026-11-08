// Core-set search for the tagged kernel: which owned cores go in the deck's core slots.
//
// 1. Pool: owned cores that can affect this run (class, arcane slots, legal tag combos, battery cards), one
//    color core when colors are interchangeable.
// 2. Beam: grow sets one core at a time; every visited set gets a cheap screen anneal and the best `beam` per
//    size are extended. Adding a core never lowers the score (additive cores), so the search ends on full sets.
// 3. Halving: the best full sets get a second, longer screen; those near the leader go on.
// 4. Full runs: phase-1 restarts split across the worker lanes, then the best ends are re-annealed (phase 2),
//    one start per job.
//
// The runner is injected so the same code drives the browser worker pool and the Node bench.

import { CardClass, CoreType, OptimizerMode, type CoreSpec } from "./types";
import type { RawTaggedPlaced } from "./optimize";
import { buildTaggedPayload, activeImplicits, colorsRealFor, type TaggedRunInput } from "./tagged";
import { TAG_CORE_GROUP } from "./coreOptions";
import { coreSpecKey } from "./cores";
import {
  SCHEDULE_PHASE1, SCHEDULE_PHASE2, type SearchBudget, type SearchDepth,
} from "./searchConfig";

export interface KernelOut {
  assignment: (RawTaggedPlaced & { stack: number })[];
  score: number;
}

export type Runner = (payload: Record<string, unknown>) => Promise<KernelOut>;

export interface CoreSearchOptions {
  runner: Runner;
  lanes: number;
  depth: SearchDepth;
  seed: number;
  onProgress?: (phase: string, done: number, total: number) => void;
}

export interface RankedSet {
  combo: CoreSpec[];
  screen: number;     // best screen score (halving screen when it had one)
  full: number | null;
}

export interface CoreSearchResult {
  combo: CoreSpec[];
  assignment: KernelOut["assignment"];
  score: number;
  ranked: RankedSet[];
  stats: { screened: number; halved: number; full: number; jobs: number; pool: string[] };
}

// ── Pool ──────────────────────────────────────────────────────────────────────

const LEGAL_CLASS = { evo: 0, shiny: 1, stat: 2, deluxe: 3, nonstat: 4 } as const;

function groupInCombos(group: string, combos: readonly string[][] | undefined, fallback: readonly string[][]): boolean {
  const list = combos && combos.length > 0 ? combos : fallback;
  return list.length === 0 || list.some((c) => c.includes(group));
}

/** Can a scoring card of this run carry `group`? */
function scoringCanCarry(input: TaggedRunInput, group: string): boolean {
  if (input.mode === OptimizerMode.EXACT) {
    return input.exactStacks.some((s) => s.count > 0 && s.groups.includes(group as never));
  }
  const by = input.legalByClass;
  const pos = input.cardClass === CardClass.SHINY ? LEGAL_CLASS.shiny : LEGAL_CLASS.evo;
  return [pos, LEGAL_CLASS.stat, LEGAL_CLASS.deluxe].some((k) => groupInCombos(group, by[k], input.legalCombos));
}

/** Can a battery (non-stat) card of this run carry `group`? */
function batteryCanCarry(input: TaggedRunInput, group: string): boolean {
  if (input.mode === OptimizerMode.EXACT) {
    return input.exactStacks.some((s) => s.count > 0 && s.groups.includes(group as never));
  }
  return groupInCombos(group, input.legalByClass[LEGAL_CLASS.nonstat], input.legalCombos);
}

/** Owned cores that can change this run's score, with interchangeable color cores collapsed to one. */
export function corePool(input: TaggedRunInput, owned: readonly CoreSpec[]): CoreSpec[] {
  const hasArcane = input.deck.arcaneSlots.length > 0;
  const colorKeyed = activeImplicits(input).some((i) => i.colors.length > 0);
  const colorsMatter = input.mode === OptimizerMode.EXACT || colorKeyed;
  const out: CoreSpec[] = [];
  let colorTaken = false;
  for (const s of owned) {
    switch (s.core_type) {
      case CoreType.SPARKLING:
        if (input.cardClass !== CardClass.SHINY) continue;
        break;
      case CoreType.ARCHIVE_CORE:
      case CoreType.NITWIT:
        if (!hasArcane) continue;
        break;
      case CoreType.COLOR:
        if (!colorsMatter) {
          if (colorTaken) continue;
          colorTaken = true;
        }
        break;
      case CoreType.BAZAAR:
        if (!batteryCanCarry(input, "Resource")) continue;
        break;
      case CoreType.TALENT:
        if (!batteryCanCarry(input, "Knack")) continue;
        break;
      case CoreType.TEMPORAL:
        if (!batteryCanCarry(input, "Temporal")) continue;
        break;
      case CoreType.HARVEST:
      case CoreType.FORTUNE:
      case CoreType.BOUNTY:
      case CoreType.ADEPT:
        continue;
      default: {
        const g = TAG_CORE_GROUP[s.core_type];
        if (g && !scoringCanCarry(input, g)) continue;
      }
    }
    out.push(s);
  }
  return out;
}

/** Sets the game allows: one color core, and Nitwit (which disables arcane cards) never with Archive. */
function compatible(set: readonly CoreSpec[], add: CoreSpec): boolean {
  for (const s of set) {
    if (s.core_type === add.core_type && s.color === add.color) return false;
    if (s.core_type === CoreType.COLOR && add.core_type === CoreType.COLOR) return false;
    const pair = new Set([s.core_type, add.core_type]);
    if (pair.has(CoreType.NITWIT) && pair.has(CoreType.ARCHIVE_CORE)) return false;
  }
  return true;
}

/** Cores whose payoff needs a coordinated layout (group counts, battery cards, slot conversion): a cheap
 *  screen rarely finds it, so the screened rank undervalues every set holding them. */
const COORDINATION_CORES: ReadonlySet<string> = new Set([
  CoreType.PLUTO, CoreType.JUPITER, CoreType.NITWIT, CoreType.TEMPORAL, CoreType.BAZAAR, CoreType.TALENT,
]);

/** The first `take` sets of `ranked`, plus (when `diverse`) the best-ranked set holding each coordination core. */
function keepDiverse(ranked: CoreSpec[][], take: number, diverse: boolean): CoreSpec[][] {
  const out = ranked.slice(0, take);
  if (!diverse) return out;
  const have = new Set(out.flatMap((s) => s.map((c) => c.core_type)));
  for (const set of ranked.slice(take)) {
    const extra = set.filter((c) => COORDINATION_CORES.has(c.core_type) && !have.has(c.core_type));
    if (extra.length === 0) continue;
    out.push(set);
    for (const c of set) have.add(c.core_type);
  }
  return out;
}

function setKey(set: readonly CoreSpec[]): string {
  return set.map(coreSpecKey).sort().join(";");
}

// ── Job plumbing ──────────────────────────────────────────────────────────────

/** FNV-1a over a string, folded with a base seed; stays below 2^53 so it crosses into WASM as a u64. */
export function jobSeed(base: number, key: string): number {
  let h = 0x811c9dc5 ^ (base >>> 0);
  for (let i = 0; i < key.length; i++) {
    h ^= key.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  const hi = Math.floor(Math.abs(base) / 4294967296) % 2097152;
  return hi * 4294967296 + h;
}

/** Phase-1 restarts per full-run job: each job returns its best end, and phase 2 re-anneals the best `topK` ends. */
const FULL_CHUNK_RESTARTS = 6;

/** Run jobs on `lanes` concurrent lanes pulling from one queue; results in job order. */
async function runQueue<T>(
  jobs: (() => Promise<T>)[], lanes: number, onDone?: (done: number) => void,
): Promise<T[]> {
  const out: T[] = new Array(jobs.length);
  let next = 0;
  let done = 0;
  const lane = async () => {
    while (next < jobs.length) {
      const i = next++;
      out[i] = await jobs[i]();
      done++;
      onDone?.(done);
    }
  };
  await Promise.all(Array.from({ length: Math.max(1, Math.min(lanes, jobs.length)) }, lane));
  return out;
}

function budget(nIter: number, schedule = SCHEDULE_PHASE1): SearchBudget {
  return { nIter, schedule };
}

function startOf(a: KernelOut["assignment"]): unknown[] {
  return a.map((p) => [p.t, p.color, p.scale_color, p.groups, p.stack]);
}

// ── Search ────────────────────────────────────────────────────────────────────

export async function searchCores(
  input: TaggedRunInput,
  owned: readonly CoreSpec[],
  slots: number,
  opt: CoreSearchOptions,
): Promise<CoreSearchResult> {
  const { runner, lanes, depth, seed } = opt;
  const pool = corePool(input, owned);
  const k = Math.max(0, Math.min(slots, pool.length));
  let jobs = 0;

  // Warm screens start every walk of a set from its parent set's best layout (a layout keeps or gains score
  // when an additive core joins), unless the new core changes the stacks (colors turn real) or the slots (Nitwit).
  const layouts = new Map<string, KernelOut["assignment"]>();
  const warmFrom = (combo: CoreSpec[]): KernelOut["assignment"] | null => {
    if (!depth.screen.warm || combo.length < 2) return null;
    const added = combo[combo.length - 1];
    const parent = combo.slice(0, -1);
    if (added.core_type === CoreType.NITWIT) return null;
    if (colorsRealFor(input, parent) !== colorsRealFor(input, combo)) return null;
    return layouts.get(setKey(parent)) ?? null;
  };
  const screen = async (sets: CoreSpec[][], restarts: number, nIter: number, phase: string, salt: string,
                        warm = false) => {
    const tasks = sets.map((combo) => () => {
      jobs++;
      const p = buildTaggedPayload(input, combo, restarts, budget(nIter));
      p.seed = jobSeed(seed, `${salt}|${setKey(combo)}`);
      const start = warm ? warmFrom(combo) : null;
      if (start) p.starts = Array.from({ length: restarts }, () => startOf(start));
      return runner(p);
    });
    opt.onProgress?.(phase, 0, tasks.length);
    const res = await runQueue(tasks, lanes, (d) => opt.onProgress?.(phase, d, tasks.length));
    sets.forEach((combo, i) => {
      const key = setKey(combo);
      const prev = layouts.get(key);
      if (!prev || res[i].score >= (screenedScore.get(key) ?? -Infinity)) {
        layouts.set(key, res[i].assignment);
        screenedScore.set(key, res[i].score);
      }
    });
    return res.map((r) => r.score);
  };
  const screenedScore = new Map<string, number>();

  // Beam over set sizes.
  const screened = new Map<string, { combo: CoreSpec[]; score: number }>();
  let beam: CoreSpec[][] = [[]];
  let finals: CoreSpec[][] = [[]];
  for (let size = 1; size <= k; size++) {
    const seen = new Set<string>();
    const cand: CoreSpec[][] = [];
    for (const set of beam) {
      for (const c of pool) {
        if (!compatible(set, c)) continue;
        const next = [...set, c];
        const key = setKey(next);
        if (seen.has(key)) continue;
        seen.add(key);
        cand.push(next);
      }
    }
    if (cand.length === 0) break;
    const scores = await screen(cand, depth.screen.restarts, depth.screen.nIter, `screen ${size}/${k}`, "s", true);
    cand.forEach((combo, i) => screened.set(setKey(combo), { combo, score: scores[i] }));
    const order = cand.map((_, i) => i).sort((a, b) => scores[b] - scores[a]);
    finals = order.map((i) => cand[i]);
    beam = keepDiverse(finals, depth.beam, depth.diverse ?? false);
  }

  // Halving on the best full sets.
  const pick = keepDiverse(finals, depth.halve.pool, depth.diverse ?? false);
  let survivors = pick;
  let halveScores: number[] = pick.map((c) => screened.get(setKey(c))?.score ?? 0);
  if (pick.length > 1) {
    halveScores = await screen(pick, depth.halve.restarts, depth.halve.nIter, "halving", "h");
    const order = pick.map((_, i) => i).sort((a, b) => halveScores[b] - halveScores[a]);
    const lead = halveScores[order[0]];
    const keep = order.filter((i, rank) =>
      rank < depth.halve.keepMin || halveScores[i] >= lead - Math.abs(lead) * depth.halve.margin);
    survivors = keep.slice(0, depth.halve.keepMax).map((i) => pick[i]);
  }

  // Full runs: phase-1 chunks, then phase-2 starts. The chunking depends only on the preset (never on the lane
  // count), so a seed gives the same result on any number of workers; more workers only finish sooner.
  const f = depth.full;
  const perSet = Math.max(f.topK, Math.ceil(f.restarts / FULL_CHUNK_RESTARTS));
  const chunk = Math.max(1, Math.ceil(f.restarts / perSet));
  const p1: { set: number; run: () => Promise<KernelOut> }[] = [];
  survivors.forEach((combo, si) => {
    for (let left = f.restarts, c = 0; left > 0; left -= chunk, c++) {
      const r = Math.min(chunk, left);
      p1.push({ set: si, run: () => {
        jobs++;
        const p = buildTaggedPayload(input, combo, r, budget(f.nIter));
        p.seed = jobSeed(seed, `f1|${setKey(combo)}|${c}`);
        return runner(p);
      } });
    }
  });
  const p1Out = await runQueue(p1.map((j) => j.run), lanes,
    (d) => opt.onProgress?.("full runs", d, p1.length));
  const ends: KernelOut[][] = survivors.map(() => []);
  p1Out.forEach((r, i) => ends[p1[i].set].push(r));

  const p2: { set: number; run: () => Promise<KernelOut> }[] = [];
  survivors.forEach((combo, si) => {
    const top = [...ends[si]].sort((a, b) => b.score - a.score).slice(0, f.topK);
    top.forEach((e, j) => p2.push({ set: si, run: () => {
      jobs++;
      const p = buildTaggedPayload(input, combo, 1, budget(f.p2Iter, SCHEDULE_PHASE2));
      p.seed = jobSeed(seed, `f2|${setKey(combo)}|${j}`);
      p.starts = [startOf(e.assignment)];
      return runner(p);
    } }));
  });
  const p2Out = await runQueue(p2.map((j) => j.run), lanes,
    (d) => opt.onProgress?.("refining", d, p2.length));
  p2Out.forEach((r, i) => ends[p2[i].set].push(r));

  const bestPer = ends.map((es) => es.reduce((a, b) => (b.score > a.score ? b : a)));
  let bi = 0;
  for (let i = 1; i < bestPer.length; i++) if (bestPer[i].score > bestPer[bi].score) bi = i;

  const fullScore = new Map(survivors.map((c, i) => [setKey(c), bestPer[i].score]));
  const halved = new Map(pick.map((c, i) => [setKey(c), halveScores[i]]));
  const ranked: RankedSet[] = finals.slice(0, Math.max(depth.halve.pool, 8)).map((combo) => {
    const key = setKey(combo);
    return { combo, screen: halved.get(key) ?? screened.get(key)?.score ?? 0, full: fullScore.get(key) ?? null };
  }).sort((a, b) => (b.full ?? -Infinity) - (a.full ?? -Infinity) || b.screen - a.screen);

  return {
    combo: survivors[bi],
    assignment: bestPer[bi].assignment,
    score: bestPer[bi].score,
    ranked,
    stats: {
      screened: screened.size, halved: pick.length > 1 ? pick.length : 0, full: survivors.length, jobs,
      pool: pool.map(coreSpecKey),
    },
  };
}
