// Web-channel search bench: the site's own TypeScript search + WASM kernel on Node worker threads.
//
// Suites (settings in bench_web.json):
//   panel    — whole optimizer runs (core-set search + anneals) per deck x class, for each strategy:
//              "old"     = the pre-port site: structured core enumeration, every candidate at the full depth,
//                          legacy geometric schedule, restarts chunked across the pool;
//              "new"     = coreSearch.ts at a depth preset (optionally with schedule overrides).
//              Each strategy runs with an owned-core set ("old9" = the pre-port core list, "all" = every core).
//              Reports gap to the persistent best-known per (deck, class, owned set) and wall time per run.
//   schedule — single kernel calls on fixed core sets (the panel's best-known sets): schedule variants at equal
//              steps, gap to the best-known layout score of that set.
//
// Every random choice comes from seeds derived from (problem, config, rep). Best-known scores only ever rise
// (bench_web_best.json); compare numbers from one run only.
//
//   npx esbuild wasm-port/scripts/bench_web.ts --bundle --platform=node --format=esm --outfile=<dir>/bw.mjs
//   node <dir>/bw.mjs <repo> --suite panel|schedule [--lanes 12] [--reps 2] [--decks a,b] [--configs x,y]
import { Worker } from "node:worker_threads";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { buildDeck, type RawDeck, type Deck } from "../web/src/lib/deck";
import type { ImplicitDef } from "../web/src/lib/implicits";
import { buildTaggedPayload, buildTasks, type TaggedRunInput } from "../web/src/lib/tagged";
import { candidateCoresInventory, coreSpecKey } from "../web/src/lib/cores";
import { CORE_OPTIONS } from "../web/src/lib/coreOptions";
import { searchCores, jobSeed, type KernelOut } from "../web/src/lib/coreSearch";
import { SEARCH_DEPTHS, type SearchDepth, type Schedule } from "../web/src/lib/searchConfig";
import { CardClass, CoreType, Depth, DEPTH_PARAMS, OptimizerMode, type CoreSpec } from "../web/src/lib/types";

const args = process.argv.slice(2);
const repo = args[0];
const opt = (k: string, d: string) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
const SUITE = opt("--suite", "panel");
const LANES = Number(opt("--lanes", "12"));
const pub = `${repo}/wasm-port/web/public`;
const settings = JSON.parse(readFileSync(`${repo}/wasm-port/scripts/bench_web.json`, "utf8"));
const REPS = Number(opt("--reps", String(settings.reps ?? 2)));
const bestPath = `${repo}/wasm-port/scripts/bench_web_best.json`;
const best: Record<string, { score: number; combo: string[] }> = existsSync(bestPath)
  ? JSON.parse(readFileSync(bestPath, "utf8")) : {};

const cfg = JSON.parse(readFileSync(`${pub}/config.json`, "utf8")).modes.wolds;
const decksJson = JSON.parse(readFileSync(`${pub}/decks.json`, "utf8"));
const catalog: Record<string, ImplicitDef> = decksJson._implicits;
const raws: RawDeck[] = decksJson.wolds;

// ── Worker pool ───────────────────────────────────────────────────────────────

const wasmPath = `${repo}/wasm-port/web/src/wasm-node/ndm_core.js`;
const workers = Array.from({ length: LANES }, () =>
  new Worker(new URL("./bench_worker.mjs", import.meta.url), { workerData: { wasmPath } }));
const idle: Worker[] = [...workers];
const waiting: ((w: Worker) => void)[] = [];
let cpuMs = 0;
let nextId = 1;
const pending = new Map<number, { res: (r: KernelOut) => void; rej: (e: Error) => void }>();
for (const w of workers) {
  w.on("message", (m) => {
    const p = pending.get(m.id);
    pending.delete(m.id);
    release(w);
    if (!p) return;
    if (m.ok) { cpuMs += m.ms; p.res(m.r); } else p.rej(new Error(m.error));
  });
}
function release(w: Worker) { const f = waiting.shift(); if (f) f(w); else idle.push(w); }
function acquire(): Promise<Worker> {
  const w = idle.pop();
  return w ? Promise.resolve(w) : new Promise((r) => waiting.push(r));
}
async function runner(payload: Record<string, unknown>): Promise<KernelOut> {
  const w = await acquire();
  const id = nextId++;
  return new Promise((res, rej) => { pending.set(id, { res, rej }); w.postMessage({ id, payload }); });
}

// ── Problems ──────────────────────────────────────────────────────────────────

const OLD9 = new Set<string>([CoreType.PURE, CoreType.EQUILIBRIUM, CoreType.STEADFAST, CoreType.SPARKLING,
  CoreType.FOIL, CoreType.DELUXE_CORE, CoreType.VOID_CORE, CoreType.ARCHIVE_CORE, CoreType.COLOR]);
const ownedSets: Record<string, CoreSpec[]> = {
  old9: CORE_OPTIONS.filter((o) => OLD9.has(o.coreType)).map((o) => ({ core_type: o.coreType, color: o.color, override: null })),
  all: CORE_OPTIONS.map((o) => ({ core_type: o.coreType, color: o.color, override: null })),
};

function runInput(deck: Deck, cls: CardClass, owned: CoreSpec[], depth: Depth, seed: number): TaggedRunInput {
  const d = DEPTH_PARAMS[depth];
  return {
    deck, cardClass: cls, mode: OptimizerMode.MAX, appMode: "wolds", targetedRules: [], exactStacks: [],
    mysteryPicks: null, implicitsEnabled: true, implicitCatalog: catalog,
    legalCombos: decksJson._tagCombos.wolds, legalByClass: decksJson._tagCombosByClass.wolds,
    complexCards: false, minStatPlaced: 0, autoPlaceArcane: true, cores: owned,
    nIter: d.nIter, restarts: d.restarts, depth, seed, cfg,
  };
}

const deckFilter = opt("--decks", "");
const decks = raws
  .filter((r) => r.implicit?.kind !== "mystery")
  .filter((r) => !deckFilter || deckFilter.split(",").includes(r.key))
  .filter((r) => settings.decks === "all" || (settings.decks as string[]).includes(r.key));
const classes = [CardClass.EVO, CardClass.SHINY];

function raise(key: string, score: number, combo: CoreSpec[]) {
  if (!best[key] || score > best[key].score) best[key] = { score, combo: combo.map(coreSpecKey).sort() };
}

// ── Strategies ────────────────────────────────────────────────────────────────

async function runOld(inp: TaggedRunInput, seed: number): Promise<{ score: number; combo: CoreSpec[] }> {
  let cands = candidateCoresInventory(inp.cores, inp.cardClass, inp.deck.core_slots,
    inp.deck.slots.length, inp.deck.arcaneSlots.length, cfg);
  if (cands.length === 0) cands = [[]];
  const tasks = buildTasks(cands, inp.restarts, LANES);
  const res = await Promise.all(tasks.map((t, i) => {
    const p = buildTaggedPayload(inp, t.combo, t.restarts);
    p.seed = jobSeed(seed, `old|${i}`);
    return runner(p).then((r) => ({ score: r.score, combo: t.combo }));
  }));
  return res.reduce((a, b) => (b.score > a.score ? b : a));
}

function depthWith(base: SearchDepth, over: Partial<SearchDepth> | undefined): SearchDepth {
  if (!over) return base;
  return {
    screen: { ...base.screen, ...(over.screen ?? {}) },
    beam: over.beam ?? base.beam,
    diverse: over.diverse ?? base.diverse,
    halve: { ...base.halve, ...(over.halve ?? {}) },
    full: { ...base.full, ...(over.full ?? {}) },
  };
}

interface Row { cfgName: string; problem: string; score: number; wall: number; cpu: number; combo?: string[] }

async function panel(): Promise<void> {
  const want = opt("--configs", "");
  const configs = (settings.panel.configs as { name: string; strategy: string; owned: string; depth: Depth;
    override?: Partial<SearchDepth> }[]).filter((c) => !want || want.split(",").includes(c.name));
  const rows: Row[] = [];
  const t0 = performance.now();
  let n = 0;
  const total = configs.length * decks.length * classes.length * REPS;
  for (const raw of decks) {
    const deck = buildDeck(raw, cfg.deckmod ?? 0);
    for (const cls of classes) {
      for (const c of configs) {
        for (let rep = 0; rep < REPS; rep++) {
          const problem = `${raw.key}|${cls}|${c.owned}`;
          const seed = jobSeed(0x5eed, `${problem}|${c.name}|${rep}`);
          const inp = runInput(deck, cls, ownedSets[c.owned], c.depth, seed);
          const cpu0 = cpuMs;
          const t = performance.now();
          let out: { score: number; combo: CoreSpec[] };
          if (c.strategy === "old") out = await runOld(inp, seed);
          else {
            const r = await searchCores(inp, inp.cores, deck.core_slots, {
              runner, lanes: LANES, depth: depthWith(SEARCH_DEPTHS[c.depth], c.override), seed,
            });
            out = { score: r.score, combo: r.combo };
          }
          rows.push({ cfgName: c.name, problem, score: out.score, wall: performance.now() - t, cpu: cpuMs - cpu0,
            combo: out.combo.map(coreSpecKey).sort() });
          raise(problem, out.score, out.combo);
          n++;
          if (n % 20 === 0) console.error(`[bench] ${n}/${total} runs, ${((performance.now() - t0) / 1000).toFixed(0)} s`);
        }
      }
    }
  }
  report(rows);
}

async function schedule(): Promise<void> {
  const variants = settings.schedule.variants as { name: string; nIter: number; restarts: number;
    schedule: Schedule | null; phase2?: { topK: number; nIter: number; schedule: Schedule } }[];
  const rows: Row[] = [];
  const jobs: Promise<void>[] = [];
  for (const raw of decks) {
    const deck = buildDeck(raw, cfg.deckmod ?? 0);
    for (const cls of classes) {
      const problem = `${raw.key}|${cls}|${settings.schedule.owned}`;
      const rec = best[problem];
      if (!rec) continue;
      const combo = ownedSets.all.filter((s) => rec.combo.includes(coreSpecKey(s)));
      for (const v of variants) {
        for (let rep = 0; rep < REPS; rep++) {
          const inp = runInput(deck, cls, combo, Depth.DEFAULT, 0);
          const p = buildTaggedPayload(inp, combo, v.restarts, { nIter: v.nIter, schedule: v.schedule, phase2: v.phase2 ?? null });
          p.seed = jobSeed(0x5c4ed, `${problem}|${v.name}|${rep}`);
          jobs.push((async () => {
            const t = performance.now();
            const r = await runner(p);
            rows.push({ cfgName: v.name, problem: `${problem}`, score: r.score, wall: performance.now() - t, cpu: 0 });
            raise(problem, r.score, combo);
          })());
        }
      }
    }
  }
  await Promise.all(jobs);
  report(rows);
}

function stats(xs: number[]) {
  const s = [...xs].sort((a, b) => a - b);
  const q = (f: number) => s[Math.min(s.length - 1, Math.floor(f * (s.length - 1)))];
  return { mean: s.reduce((a, b) => a + b, 0) / s.length, med: q(0.5), p90: q(0.9), max: s[s.length - 1] };
}

function report(rows: Row[]) {
  writeFileSync(bestPath, JSON.stringify(best, null, 1) + "\n");
  const dump = opt("--dump", "");
  if (dump) writeFileSync(dump, JSON.stringify(rows.map((r) => ({ ...r, best: best[r.problem] })), null, 1));
  const names = [...new Set(rows.map((r) => r.cfgName))];
  console.log(`\n${SUITE}: ${decks.length} decks x ${classes.length} classes x ${REPS} reps, lanes ${LANES}`);
  console.log("config".padEnd(22) + "gap% mean   med    p90    max   hits   wall s mean  cpu s mean");
  for (const nm of names) {
    const rs = rows.filter((r) => r.cfgName === nm);
    const gaps = rs.map((r) => 100 * (best[r.problem].score - r.score) / Math.max(1e-9, best[r.problem].score));
    const g = stats(gaps);
    const hits = gaps.filter((x) => x < 1e-7).length / gaps.length;
    const w = stats(rs.map((r) => r.wall / 1000));
    const c = stats(rs.map((r) => r.cpu / 1000));
    console.log(nm.padEnd(22) + [g.mean, g.med, g.p90, g.max].map((x) => x.toFixed(3).padStart(6)).join(" ")
      + `  ${(100 * hits).toFixed(0).padStart(4)}%  ${w.mean.toFixed(2).padStart(9)}  ${c.mean.toFixed(2).padStart(10)}`);
  }
}

(SUITE === "panel" ? panel() : schedule()).then(() => process.exit(0), (e) => { console.error(e); process.exit(1); });
