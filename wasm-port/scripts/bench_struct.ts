// Structural (Construction Core) optimizer bench: drives web/src/lib/structuralOpt.ts on Node worker threads
// through struct_shim.ts, so two commits can be compared on identical problems.
//
//   prep:  bench_struct.mjs <repo> prep <problems.json> [--decks a,b] [--ks 3,5] [--truth-k 3]
//          picks each deck's core combo (core search, fast depth, every core owned, one slot left for the
//          Construction Core), a seeded random roll per k as the player's layout, and for k in --truth-k an
//          exhaustive per-set reference score (every reachable set, no mirror merging, cold 6 x 100k target
//          walks + phase 2 top 2 x 200k).
//   run:   bench_struct.mjs <repo> run <problems.json> <rows.json> [--efforts standard,deep] [--label x]
//          [--only deck:k,deck:k] [--salt n]
//          one cold runStructural per (problem, effort); appends rows to rows.json. --salt n (or STRUCT_SALT)
//          remaps every job seed in the shim for repeat runs on fresh random numbers.
//
// Build (from the commit under test; struct_shim.ts replaces every workerPool import):
//   node wasm-port/scripts/build_bench_struct.mjs <repo> <out.mjs>
// Env: STRUCT_WASM=<repo>/wasm-port/web/src/wasm-node/ndm_core.js STRUCT_WORKER=<struct_worker.mjs> STRUCT_LANES=24
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { buildDeck, type RawDeck, type Deck } from "../web/src/lib/deck";
import type { ImplicitDef } from "../web/src/lib/implicits";
import { buildTaggedPayload, type TaggedRunInput } from "../web/src/lib/tagged";
import { CORE_OPTIONS } from "../web/src/lib/coreOptions";
import { searchCores, type KernelOut } from "../web/src/lib/coreSearch";
import { SEARCH_DEPTHS } from "../web/src/lib/searchConfig";
import { CardClass, Depth, DEPTH_PARAMS, OptimizerMode, type CoreSpec, type Position } from "../web/src/lib/types";
import { effectiveDeck, emptyStructural } from "../web/src/lib/structural";
import { runStructural, clearStructCache, type StructEffort } from "../web/src/lib/structuralOpt";
import { dispatchConstruction, dispatchTagged, workerPoolSize, closePool, cpuMs } from "./struct_shim";

const args = process.argv.slice(2);
const repo = args[0];
const mode = args[1];
const opt = (k: string, d: string) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
const pub = `${repo}/wasm-port/web/public`;
const cfg = JSON.parse(readFileSync(`${pub}/config.json`, "utf8")).modes.wolds;
const decksJson = JSON.parse(readFileSync(`${pub}/decks.json`, "utf8"));
const catalog: Record<string, ImplicitDef> = decksJson._implicits;
const raws: RawDeck[] = decksJson.wolds;

interface Problem {
  deck: string; k: number; combo: CoreSpec[]; userAdded: Position[];
  /** Exhaustive reference: every reachable set with its roll probability and reference score. */
  truth?: { sets: number[][]; prob: number[]; score: number[] };
}

function runInput(deck: Deck, cores: CoreSpec[]): TaggedRunInput {
  const d = DEPTH_PARAMS[Depth.DEFAULT];
  return {
    deck, cardClass: CardClass.EVO, mode: OptimizerMode.MAX, appMode: "wolds", targetedRules: [], exactStacks: [],
    mysteryPicks: null, implicitsEnabled: true, implicitCatalog: catalog,
    legalCombos: decksJson._tagCombos.wolds, legalByClass: decksJson._tagCombosByClass.wolds,
    complexCards: false, minStatPlaced: 0, autoPlaceArcane: true, cores,
    nIter: d.nIter, restarts: d.restarts, depth: Depth.DEFAULT, seed: 1, cfg,
  };
}

function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => { s ^= s << 13; s ^= s >>> 17; s ^= s << 5; return (s >>> 0) / 4294967296; };
}

/** One roll of k picks as the game draws it (uniform over the orthogonal frontier inside the 9 x 6 box). */
function sampleRoll(deck: Deck, k: number, seed: number): Position[] {
  const r = rng(seed);
  const occ = new Set(deck.slots.map(([a, b]) => `${a},${b}`));
  const added: Position[] = [];
  for (let n = 0; n < k; n++) {
    const fr: Position[] = [];
    const seen = new Set<string>();
    for (const key of occ) {
      const [a, b] = key.split(",").map(Number);
      for (const [da, db] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
        const p: Position = [a + da, b + db];
        const pk = `${p[0]},${p[1]}`;
        if (p[0] < 0 || p[0] > 5 || p[1] < 0 || p[1] > 8 || occ.has(pk) || seen.has(pk)) continue;
        seen.add(pk);
        fr.push(p);
      }
    }
    fr.sort((x, y) => x[0] - y[0] || x[1] - y[1]);
    const pick = fr[Math.floor(r() * fr.length)];
    added.push(pick);
    occ.add(`${pick[0]},${pick[1]}`);
  }
  return added;
}

async function prep(path: string) {
  const want = opt("--decks", "queen,wold,bishop,merchant,fairy,large").split(",");
  const ks = opt("--ks", "3,5").split(",").map(Number);
  const truthKs = new Set(opt("--truth-k", "3").split(",").map(Number));
  const owned = CORE_OPTIONS.map((o) => ({ core_type: o.coreType, color: o.color, override: null }));
  const runner = (p: Record<string, unknown>) => dispatchTagged<KernelOut>(0, p);
  const out: Problem[] = [];
  for (const key of want) {
    const raw = raws.find((r) => r.key === key);
    if (!raw) throw new Error(`no deck ${key}`);
    const deck = buildDeck(raw, cfg.deckmod ?? 0);
    const slots = Math.max(0, deck.core_slots - 1);
    const best = await searchCores(runInput(deck, owned), owned, slots, {
      runner, lanes: workerPoolSize(), depth: SEARCH_DEPTHS.fast, seed: 0x57A7,
    });
    for (const k of ks) {
      const p: Problem = { deck: key, k, combo: best.combo, userAdded: sampleRoll(deck, k, 0xC0DE + k * 31 + key.length) };
      if (truthKs.has(k)) {
        const t0 = performance.now();
        const base = deck.slots.map(([r, c]) => [r, c, deck.arcaneSlots.some(([a, b]) => a === r && b === c)]);
        const en = await dispatchConstruction<{ classes: { rep: number[]; prob: number }[] }>(0, "cc_enum", {
          base, ks: [k], k_probs: [1], allow_x: false, allow_y: false, user_added: [],
        });
        const run = runInput(deck, best.combo);
        const template = {
          ...buildTaggedPayload(run, best.combo, 1),
          schedule: { kind: "target", params: [0.5, 0.001, 0.02, 0.01] },
          phase2: { top_k: 2, n_iter: 200_000, schedule: { kind: "target", params: [0.1, 0.001, 0.005, 0.01] } },
        };
        const sets = en.classes.map((c) => c.rep);
        const parts: number[][] = Array.from({ length: Math.min(sets.length, 96) }, () => []);
        sets.forEach((_, i) => parts[i % parts.length].push(i));
        const score = new Array<number>(sets.length);
        await Promise.all(parts.map((idx, w) => dispatchConstruction<{ scores: number[] }>(0, "cc_eval", {
          template, base, sets: idx.map((i) => sets[i]), n_iter: 100_000, restarts: 6, seed: 0x7A07 + w * 7919,
        }).then((r) => idx.forEach((i, j) => { score[i] = r.scores[j]; }))));
        p.truth = { sets, prob: en.classes.map((c) => c.prob), score };
        console.error(`[prep] ${key} k=${k}: ${sets.length} sets scored in ${((performance.now() - t0) / 1000).toFixed(1)} s`);
      }
      out.push(p);
    }
    console.error(`[prep] ${key}: combo ${best.combo.map((c) => c.core_type + (c.color ? `:${c.color}` : "")).join(" ")}`);
  }
  writeFileSync(path, JSON.stringify(out));
}

async function run(path: string, rowsPath: string) {
  const problems: Problem[] = JSON.parse(readFileSync(path, "utf8"));
  const efforts = opt("--efforts", "standard,deep").split(",") as StructEffort[];
  const label = opt("--label", "run");
  const only = opt("--only", "");
  const salt = opt("--salt", process.env.STRUCT_SALT ?? "0");
  process.env.STRUCT_SALT = salt;
  const rows: unknown[] = existsSync(rowsPath) ? JSON.parse(readFileSync(rowsPath, "utf8")) : [];
  for (const p of problems) {
    if (only && !only.split(",").includes(`${p.deck}:${p.k}`)) continue;
    const raw = raws.find((r) => r.key === p.deck)!;
    const base = buildDeck(raw, cfg.deckmod ?? 0);
    const structural = { ...emptyStructural(), constructionEnabled: true, greaterStructural: p.k > 3 };
    for (const effort of efforts) {
      clearStructCache();
      const cpu0 = cpuMs;
      const t = performance.now();
      const rep = await runStructural({
        run: runInput(base, p.combo), combo: p.combo, baseDeck: base,
        deckFor: (added) => effectiveDeck(base, { ...structural, addedSlots: [...added] }, cfg.deckmod ?? 0),
        userAdded: p.userAdded, userScore: 0, fingerprint: `${p.deck}|${p.k}`,
      }, effort, () => {});
      const row = {
        label, salt: Number(salt), deck: p.deck, k: p.k, effort, wall: performance.now() - t, cpu: cpuMs - cpu0,
        T: rep.user.score, ideal: rep.ideal.score, idealAdded: rep.ideal.added,
        pBetter: rep.pBetter, lo: rep.pBetterLo, hi: rep.pBetterHi, classes: rep.classes,
      };
      rows.push(row);
      console.error(`[run] ${label} ${p.deck} k=${p.k} ${effort}: ${(row.wall / 1000).toFixed(1)} s ideal ${row.ideal.toFixed(2)} T ${row.T.toFixed(2)} p ${row.pBetter.toExponential(2)}`);
      writeFileSync(rowsPath, JSON.stringify(rows));
    }
  }
}

(async () => {
  if (mode === "prep") await prep(args[2]);
  else if (mode === "run") await run(args[2], args[3]);
  else throw new Error(`mode ${mode}: want prep or run`);
  closePool();
})().catch((e) => { console.error(e); process.exit(1); });
