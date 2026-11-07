// Scoring parity gate for the tagged kernel (web channel): the WASM kernel and the TypeScript mirror
// (taggedBreakdown.ts) are two independent implementations of MODELING_CHOICES.md; they must agree on random
// layouts across every core, implicit and flag combination. Part B anneals a few runs and checks that the
// kernel's reported score equals both a fresh kernel score and the mirror of the returned layout.
//
//   wasm-pack build --target nodejs --release --out-dir ../web/src/wasm-node -- --no-default-features --features wasm
//   npx esbuild wasm-port/scripts/tagged_parity.ts --bundle --platform=node --format=esm --outfile=<dir>/tp.mjs
//   node <dir>/tp.mjs <repo>/wasm-port/web/public <repo>/wasm-port/web/src/wasm-node/ndm_core.js [--cases 3000]
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { buildDeck, type RawDeck } from "../web/src/lib/deck";
import type { ImplicitDef } from "../web/src/lib/implicits";
import { buildTaggedPayload, activeImplicits, colorsRealFor, type TaggedRunInput } from "../web/src/lib/tagged";
import { simulateTaggedBreakdown } from "../web/src/lib/taggedBreakdown";
import { CORE_OPTIONS } from "../web/src/lib/coreOptions";
import {
  CardClass, CardType, CATEGORY_GROUPS, Depth, OptimizerMode, REAL_GREEDS,
  type CoreSpec, type GroupTag, type TaggedPlaced, type Color,
} from "../web/src/lib/types";

const args = process.argv.slice(2);
const [pub, wasmPath] = args;
const opt = (k: string, d: string) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
const CASES = Number(opt("--cases", "3000"));
const require = createRequire(import.meta.url);
const wasm = require(wasmPath);
const cfgAll = JSON.parse(readFileSync(`${pub}/config.json`, "utf8"));
const cfg = cfgAll.modes.wolds;
const decksJson = JSON.parse(readFileSync(`${pub}/decks.json`, "utf8"));
const catalog: Record<string, ImplicitDef> = decksJson._implicits;
const raws: RawDeck[] = decksJson.wolds;

let state = 0x5eed1234;
const rnd = () => { state ^= state << 13; state >>>= 0; state ^= state >>> 17; state ^= state << 5; state >>>= 0; return state / 4294967296; };
const pick = <T>(xs: readonly T[]): T => xs[Math.floor(rnd() * xs.length)];
const COLORS: Color[] = ["red", "green", "blue", "yellow"];
const CORE_KINDS = CORE_OPTIONS.filter((o, i, a) => a.findIndex((x) => x.coreType === o.coreType) === i);

function randomCores(): CoreSpec[] {
  const out: CoreSpec[] = [];
  for (const o of CORE_KINDS) {
    if (rnd() > 0.3) continue;
    out.push({
      core_type: o.coreType,
      color: o.color === null ? null : pick(COLORS),
      override: rnd() < 0.25 ? 1 + Math.floor(rnd() * 20) / 10 : null,
    });
  }
  return out;
}

function randomCard(arcaneSlot: boolean, scorable: CardType[]): TaggedPlaced {
  const roll = rnd();
  let t: CardType;
  if (arcaneSlot && roll < 0.5) t = CardType.ARCANE;
  else if (roll < 0.1) t = CardType.DEAD;
  else if (roll < 0.25) t = pick(REAL_GREEDS);
  else if (roll < 0.28) t = CardType.WILD;
  else t = pick(scorable);
  if (t === CardType.DEAD) return { t, color: null, scaleColor: null, groups: [] };
  const color = pick(COLORS);
  const groups: GroupTag[] = [];
  for (const g of [...CATEGORY_GROUPS, "Foil", "Stat"] as GroupTag[]) if (rnd() < 0.15) groups.push(g);
  return { t, color, scaleColor: rnd() < 0.2 ? pick(COLORS) : color, groups };
}

function input(raw: RawDeck, cardClass: CardClass, cores: CoreSpec[], mystery: [string, string] | null): TaggedRunInput {
  return {
    deck: buildDeck(raw, cfg.deckmod ?? 0), cardClass, mode: OptimizerMode.MAX, appMode: "wolds",
    targetedRules: [], exactStacks: [], mysteryPicks: mystery, implicitsEnabled: true, implicitCatalog: catalog,
    legalCombos: decksJson._tagCombos.wolds, legalByClass: decksJson._tagCombosByClass.wolds,
    complexCards: rnd() < 0.2, minStatPlaced: 0, autoPlaceArcane: true, cores,
    nIter: 2000, restarts: 1, depth: Depth.DEFAULT, seed: 1, cfg,
  };
}

const scorable = [CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG, CardType.DELUXE, CardType.TYPELESS];
const mysteryKeys = Object.keys(catalog).filter((k) => catalog[k].kind !== "mystery");
let worst = 0, fails = 0;
const byCore = new Map<string, number>();
for (let n = 0; n < CASES; n++) {
  const raw = pick(raws);
  const cls = rnd() < 0.5 ? CardClass.SHINY : CardClass.EVO;
  const cores = randomCores();
  const mystery: [string, string] | null = raw.implicit?.kind === "mystery" ? [pick(mysteryKeys), pick(mysteryKeys)] : null;
  const inp = input(raw, cls, cores, mystery);
  const arc = new Set(inp.deck.arcaneSlotIndices);
  const cards = inp.deck.slots.map((_, i) => randomCard(arc.has(i), scorable));
  const payload = buildTaggedPayload(inp, cores, 1);
  payload.assignment = cards.map((c) => [c.t, c.color ?? "", c.scaleColor ?? "", c.groups]);
  const k = wasm.scoreTagged(payload) as number;
  const ts = simulateTaggedBreakdown(inp.deck, cards, cls, cores, activeImplicits(inp), {
    colorsReal: colorsRealFor(inp, cores), complex: inp.complexCards, wvFoilRules: true,
  }, cfg).total;
  const rel = Math.abs(k - ts) / Math.max(1, Math.abs(k));
  worst = Math.max(worst, rel);
  if (rel > 1e-9) {
    fails++;
    if (fails <= 5) console.log(`MISMATCH case ${n}: deck ${raw.key} ${cls} cores=${cores.map((c) => c.core_type).join(",")} kernel=${k} ts=${ts}`);
  }
  for (const c of cores) byCore.set(c.core_type, (byCore.get(c.core_type) ?? 0) + 1);
}
console.log(`Part A: ${CASES} random layouts, ${fails} mismatches, worst rel diff ${worst.toExponential(2)}`);
console.log(`  cases per core: ${[...byCore].map(([c, m]) => `${c}=${m}`).join(" ")}`);

let bFails = 0;
const B = 40;
for (let n = 0; n < B; n++) {
  const raw = pick(raws);
  const cls = rnd() < 0.5 ? CardClass.SHINY : CardClass.EVO;
  const cores = randomCores().slice(0, 4);
  const inp = input(raw, cls, cores, null);
  const payload = buildTaggedPayload(inp, cores, 2, { nIter: 4000, schedule: { kind: "target", params: [0.5, 0.001, 0.02, 0.01] } });
  payload.seed = 1000 + n;
  const out = wasm.runSaTagged(payload) as { assignment: { t: string; color: string; scale_color: string; groups: string[] }[]; score: number };
  const cards: TaggedPlaced[] = out.assignment.map((p) => ({
    t: p.t as CardType, color: (p.color || null) as Color | null, scaleColor: (p.scale_color || null) as Color | null,
    groups: p.groups as GroupTag[],
  }));
  const sp = buildTaggedPayload(inp, cores, 1);
  sp.assignment = out.assignment.map((p) => [p.t, p.color, p.scale_color, p.groups]);
  const k = wasm.scoreTagged(sp) as number;
  const ts = simulateTaggedBreakdown(inp.deck, cards, cls, cores, activeImplicits(inp), {
    colorsReal: colorsRealFor(inp, cores), complex: inp.complexCards, wvFoilRules: true,
  }, cfg).total;
  const ok = Math.abs(k - out.score) <= 1e-9 * Math.max(1, k) && Math.abs(ts - out.score) <= 1e-9 * Math.max(1, k);
  if (!ok) { bFails++; console.log(`B MISMATCH ${raw.key} ${cls} run=${out.score} score=${k} ts=${ts}`); }
}
console.log(`Part B: ${B} anneals, ${bFails} score mismatches`);
process.exit(fails + bFails > 0 ? 1 : 0);
