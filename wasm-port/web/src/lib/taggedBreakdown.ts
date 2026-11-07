// Per-slot NDM breakdown for Optimizer 2.0 — a 1:1 TypeScript mirror of
// tagsim.rs::simulate() over a FIXED assignment. The result total must match
// the wasm kernel's score within 1e-6 (the verify badge depends on it), and
// the what-if tag-edit popup re-scores through this same function.

import {
  CardClass, CardType, Color, CoreType,
  type CoreSpec, type GroupTag, type TaggedPlaced,
} from "./types";
import type { Deck } from "./deck";
import { greedAdd } from "./breakdown";
import type { ResolvedConfig } from "./config";
import type { ImplicitPayload } from "./implicits";
import {
  classifyCores, type CoreComponent, type ExcludedCore,
} from "./cores";
import { ADDON_CORES, TAG_CORE_GROUP, coreLabel, kernelCoreValue } from "./coreOptions";

export interface ImplicitPart {
  label:  string;
  addend: number;      // additive contribution folded into core_mult
}

export interface TaggedSlotBreakdown {
  cardType:   CardType;
  color:      Color | null;
  scaleColor: Color | null;
  groups:     GroupTag[];
  baseValue:  number;
  baseExplain: string;
  appliedCores:  CoreComponent[];
  excludedCores: ExcludedCore[];
  implicitParts: ImplicitPart[];
  coreMult:   number;
  coreMultFormula: string;
  boost:      number;
  boostSources: { fromPosition: [number, number]; greedType: CardType; multiplier: number }[];
  // Archive core (live semantics: additive term INSIDE coreMult). archiveMult
  // is the stack value base^n — kept for the popup's exponent explainer.
  archiveMult: number;
  archiveArcaneCount: number;
  archiveBase: number;
  mirrorFactor: number;      // runic (1.0 when absent / not passing)
  finalNdm:   number;
}

export interface TaggedBreakdownResult {
  total:   number;
  perSlot: Map<string, TaggedSlotBreakdown>;
  nNs:     number;
  nDead:   number;
}

export interface TaggedScoreFlags {
  colorsReal: boolean;
  complex:    boolean;
  wvFoilRules: boolean;
}

const POSITIONAL = new Set<CardType>([CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG]);
const GREEDS = new Set<CardType>([
  CardType.DIR_GREED_UP, CardType.DIR_GREED_DOWN,
  CardType.DIR_GREED_LEFT, CardType.DIR_GREED_RIGHT,
]);
const SCORABLE = new Set<CardType>([
  CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
  CardType.DELUXE, CardType.TYPELESS,
]);

const key = (r: number, c: number) => `${r},${c}`;

const NONSTAT = ["Resource", "Temporal", "Knack"];
/** The Wild ("dummy") card's groups in the 0.34.1 card dump. */
const WILD_GROUPS = ["Wild", "Arcane", "Evolution", "Stat", "Defensive", "Knack", "Magical", "Physical", "Resource", "Utility"];

/** The game's group set of a placed card (kernel xgroups): what Jupiter/Pluto count and the Stat/Shiny gates read. */
export function cardGameGroups(c: TaggedPlaced, isShiny: boolean): Set<string> {
  if (c.t === CardType.DEAD) return new Set();
  const g = new Set<string>(c.groups);
  if (c.t === CardType.WILD) { for (const x of WILD_GROUPS) g.add(x); return g; }
  const nonstat = c.groups.some((x) => NONSTAT.includes(x));
  if (POSITIONAL.has(c.t)) {
    if (isShiny) { g.add("Shiny"); g.add("Stat"); } else g.add("Evolution");
  } else if (c.t === CardType.DELUXE) {
    g.add("Deluxe");
    if (!nonstat) g.add("Stat");
  } else if (c.t === CardType.TYPELESS) {
    if (!nonstat) g.add("Stat");
  } else if (GREEDS.has(c.t)) {
    g.add("Greed");
  } else if (c.t === CardType.ARCANE) {
    g.add("Arcane");
  }
  return g;
}

/** First group (alphabetical) with the highest (`most`) or lowest positive count, Stat excluded. */
function dominantGroup(counts: Map<string, number>, most: boolean): string | null {
  let best: string | null = null;
  let bn = 0;
  for (const g of [...counts.keys()].sort()) {
    if (g === "Stat") continue;
    const n = counts.get(g) ?? 0;
    if (n === 0) continue;
    if (best === null || (most ? n > bn : n < bn)) { best = g; bn = n; }
  }
  return best;
}

export function simulateTaggedBreakdown(
  deck:      Deck,
  cards:     readonly TaggedPlaced[],     // parallel to deck.slots
  cardClass: CardClass,
  cores:     readonly CoreSpec[],
  implicits: readonly ImplicitPayload[],
  flags:     TaggedScoreFlags,
  cfg:       ResolvedConfig,
): TaggedBreakdownResult {
  const n = deck.slots.length;
  const slotIndex = new Map<string, number>();
  deck.slots.forEach((p, i) => slotIndex.set(key(p[0], p[1]), i));

  // Orthogonal neighbors (up/down/left/right slot indices).
  const orth: (number | undefined)[][] = deck.slots.map(([r, c]) => [
    slotIndex.get(key(r - 1, c)), slotIndex.get(key(r + 1, c)),
    slotIndex.get(key(r, c - 1)), slotIndex.get(key(r, c + 1)),
  ]);

  const rows = deck.slots.map(([r]) => r);
  const cols = deck.slots.map(([, c]) => c);
  const rowMax = Math.max(...rows);
  const colMin = Math.min(...cols);
  const colMax = Math.max(...cols);

  // Runic horizontal mirror per slot (bbox mirror, same row).
  const mirrorOf: (number | undefined)[] = deck.slots.map(([r, c]) => {
    const mc = colMax - (c - colMin);
    return mc === c ? undefined : slotIndex.get(key(r, mc));
  });
  const mirrorSelf: boolean[] = deck.slots.map(([, c]) => colMax - (c - colMin) === c);

  // Star-cross cells (AdjacencyBonus STARCROSS) and Nitwit slots (the deck's arcane slots under a Nitwit core).
  const STAR: [number, number][] = [[-1, 0], [1, 0], [0, -1], [0, 1], [-2, 0], [2, 0], [0, -2], [0, 2],
                                    [-1, -1], [-1, 1], [1, -1], [1, 1]];
  const starcross: number[][] = deck.slots.map(([r, c]) =>
    STAR.map(([dr, dc]) => slotIndex.get(key(r + dr, c + dc))).filter((q): q is number => q !== undefined));
  const isShiny = cardClass === CardClass.SHINY;
  const nitwitOn = cores.some((s) => s.core_type === CoreType.NITWIT);
  const nitwitSlot = new Set<number>(nitwitOn ? deck.arcaneSlotIndices : []);
  const gameGroupCount = new Map<string, number>();

  // ── Pass 1: classify + counts ─────────────────────────────────────────
  const rowColor = new Map<string, number>();   // `${r}|${color}`
  const colColor = new Map<string, number>();
  const rowFill = new Map<number, number>();
  const colFill = new Map<number, number>();
  let nDeluxe = 0, nArcane = 0, nGreed = 0, nDead = 0, nNonFoil = 0;
  let groupsUnion = new Set<GroupTag>();
  const colorsPresent = new Set<string>();
  let anyWild = false;

  for (let i = 0; i < n; i++) {
    const c = cards[i];
    if (c.t === CardType.DEAD) { nDead++; continue; }
    const [r, cc] = deck.slots[i];
    rowFill.set(r, (rowFill.get(r) ?? 0) + 1);
    colFill.set(cc, (colFill.get(cc) ?? 0) + 1);
    if (c.t === CardType.WILD) {
      for (const col of ["red", "green", "blue", "yellow"]) {
        rowColor.set(`${r}|${col}`, (rowColor.get(`${r}|${col}`) ?? 0) + 1);
        colColor.set(`${cc}|${col}`, (colColor.get(`${cc}|${col}`) ?? 0) + 1);
        colorsPresent.add(col);
      }
      anyWild = true;
      for (const g of ["Offensive","Defensive","Physical","Magical","Utility","Resource","Knack","Temporal","Essence","Stat"] as GroupTag[]) {
        groupsUnion.add(g);
      }
    } else if (c.color !== null) {
      rowColor.set(`${r}|${c.color}`, (rowColor.get(`${r}|${c.color}`) ?? 0) + 1);
      colColor.set(`${cc}|${c.color}`, (colColor.get(`${cc}|${c.color}`) ?? 0) + 1);
      colorsPresent.add(c.color);
    }
    for (const g of c.groups) groupsUnion.add(g);
    for (const g of cardGameGroups(c, isShiny)) gameGroupCount.set(g, (gameGroupCount.get(g) ?? 0) + 1);
    if (!c.groups.includes("Foil")) nNonFoil++;
    if (c.t === CardType.DELUXE) nDeluxe++;
    else if (c.t === CardType.ARCANE) nArcane++;
    else if (GREEDS.has(c.t)) nGreed++;
  }

  // n_ns (game NonFoilEfficiency, audited 2026-08-01): EVERY placed card
  // without the Foil group — typeless/deluxe/wild included.
  const nNs = nNonFoil;
  // EQUILIBRIUM's unique-color count (wild counts as every color).
  const nColors = colorsPresent.size;

  // ── Cores (identical math to classifyCores / kernel) ──────────────────
  const { baseline, statComps, sparkComp, colorComp, deluxeComp, voidComp, archiveComp, classExcluded } =
    classifyCores(cores, cardClass, nNs, nDeluxe, nDead, nArcane, nColors, cfg);
  const jupiterGroup = dominantGroup(gameGroupCount, true);
  const plutoGroup = dominantGroup(gameGroupCount, false);
  const addonCores = cores.filter((s) => ADDON_CORES.has(s.core_type))
    .map((s) => ({ spec: s, value: kernelCoreValue(s, cfg), label: coreLabel({ coreType: s.core_type, color: null }) }));
  const archiveMult = archiveComp !== null ? archiveComp.value : 1.0;
  const archiveSpec = cores.find((s) => s.core_type === CoreType.ARCHIVE_CORE) ?? null;
  const archiveBase = archiveSpec === null
    ? 1.0 : (archiveSpec.override !== null ? archiveSpec.override : cfg.cores.archive_core);

  // ── Implicit precompute ────────────────────────────────────────────────
  const freqMult: Record<string, number> = {};
  let chainValue = 0, haveChain = false;
  let emptyAddend = 0;
  let uniqueValue = 0, haveUnique = false;
  let mirrorValue = 1.0, haveMirror = false;
  for (const imp of implicits) {
    switch (imp.kind) {
      case "freq":
        freqMult[imp.extra] = (freqMult[imp.extra] ?? 1) * Math.max(1, Math.round(imp.value));
        break;
      case "chain": chainValue = imp.value; haveChain = true; break;
      case "empty_slots": emptyAddend += imp.value * nDead; break;
      case "unique_groups": uniqueValue = imp.value; haveUnique = true; break;
      case "mirror": mirrorValue = imp.value; haveMirror = true; break;
    }
  }

  let uniqueGroupsCount = 0;
  if (haveUnique) {
    uniqueGroupsCount = groupsUnion.size + 1;            // + Shiny/Evolution marker
    if (nDeluxe > 0) uniqueGroupsCount++;
    if (nArcane > 0) uniqueGroupsCount++;
    if (nGreed > 0) uniqueGroupsCount++;
    if (anyWild) uniqueGroupsCount++;
  }

  // Snake chain labeling (flood fill over non-dead; same-color when real).
  const chainId = new Array<number>(n).fill(0);
  const chainSize: number[] = [0];
  if (haveChain) {
    let nextId = 0;
    for (let start = 0; start < n; start++) {
      if (cards[start].t === CardType.DEAD || chainId[start] !== 0) continue;
      nextId++;
      let size = 0;
      const stack = [start];
      chainId[start] = nextId;
      while (stack.length) {
        const i = stack.pop()!;
        size++;
        for (const j of orth[i]) {
          if (j === undefined) continue;
          if (cards[j].t === CardType.DEAD || chainId[j] !== 0) continue;
          const a = cards[i], b = cards[j];
          const same = !flags.colorsReal
            || a.t === CardType.WILD || b.t === CardType.WILD
            || (a.color !== null && a.color === b.color);
          if (same) { chainId[j] = nextId; stack.push(j); }
        }
      }
      chainSize[nextId] = size;
    }
  }

  // ── Greed boosts (orthogonal only) ─────────────────────────────────────
  const boost = new Array<number>(n).fill(1.0);
  const boostSources: TaggedSlotBreakdown["boostSources"][] = Array.from({ length: n }, () => []);
  for (let i = 0; i < n; i++) {
    const g = cards[i];
    if (!GREEDS.has(g.t)) continue;
    let target: number | undefined, amount = 0;
    if (g.t === CardType.DIR_GREED_UP) { target = orth[i][0]; amount = cfg.greed.dir_vert; }
    else if (g.t === CardType.DIR_GREED_DOWN) { target = orth[i][1]; amount = cfg.greed.dir_vert; }
    else if (g.t === CardType.DIR_GREED_LEFT) { target = orth[i][2]; amount = cfg.greed.dir_horiz; }
    else if (g.t === CardType.DIR_GREED_RIGHT) { target = orth[i][3]; amount = cfg.greed.dir_horiz; }
    if (target === undefined) continue;
    if (!SCORABLE.has(cards[target].t)) continue;
    const colorOk = !(flags.colorsReal && flags.complex)
      || cards[target].t === CardType.WILD
      || cards[target].color === g.scaleColor;
    if (!colorOk) continue;
    if (cfg.stacking.greed_additive) boost[target] = greedAdd(boost[target], amount);
    else boost[target] *= amount;
    boostSources[target].push({
      fromPosition: deck.slots[i] as [number, number],
      greedType: g.t, multiplier: amount,
    });
  }

  // ── Accumulate ─────────────────────────────────────────────────────────
  const perSlot = new Map<string, TaggedSlotBreakdown>();
  let total = 0;

  const zero = (i: number, explain: string): TaggedSlotBreakdown => ({
    cardType: cards[i]?.t ?? CardType.EMPTY,
    color: cards[i]?.color ?? null,
    scaleColor: cards[i]?.scaleColor ?? null,
    groups: cards[i]?.groups ?? [],
    baseValue: 0, baseExplain: explain,
    appliedCores: [], excludedCores: [], implicitParts: [],
    coreMult: 1.0, coreMultFormula: "(not scored)",
    boost: 1.0, boostSources: [],
    archiveMult: 1.0, archiveArcaneCount: nArcane, archiveBase,
    mirrorFactor: 1.0, finalNdm: 0,
  });

  for (let i = 0; i < n; i++) {
    const c = cards[i];
    const k = key(deck.slots[i][0], deck.slots[i][1]);

    if (!SCORABLE.has(c.t)) {
      const explain =
        c.t === CardType.DEAD ? "dead card — empty slot, contributes nothing"
        : c.t === CardType.ARCANE ? "arcane card — 0 NDM by design; counts in n_ns and neighbors' peer counts"
        : c.t === CardType.WILD ? "Wild — 0 NDM itself; counts as any group/color for every neighbor's scaling"
        : GREEDS.has(c.t) ? "greed card — boosts its orthogonal target, no own NDM"
        : "empty slot";
      perSlot.set(k, zero(i, explain));
      continue;
    }
    // Non-stat card (Resource/Temporal/Knack): provides no player stats → 0 NDM
    // itself; still fills its slot and feeds implicits that read it.
    if (c.groups.some((g) => NONSTAT.includes(g))) {
      perSlot.set(k, zero(i,
        "non-stat card (Resource/Temporal/Knack) — gives no stats, 0 NDM; still " +
        "counts as a neighbor, in n_ns, and for implicits and cores that read it"));
      continue;
    }

    // Positional base with freq implicit.
    let base: number, baseExplain: string;
    const scanColor = flags.complex && flags.colorsReal ? c.scaleColor : c.color;
    if (POSITIONAL.has(c.t)) {
      const [r, cc] = deck.slots[i];
      let raw = 0;
      if (c.t === CardType.ROW) {
        raw = flags.colorsReal
          ? (scanColor !== null ? rowColor.get(`${r}|${scanColor}`) ?? 0 : 0)
          : rowFill.get(r) ?? 0;
        baseExplain = `row ${r}${flags.colorsReal ? ` (${scanColor})` : ""} count = ${raw}`;
      } else if (c.t === CardType.COL) {
        raw = flags.colorsReal
          ? (scanColor !== null ? colColor.get(`${cc}|${scanColor}`) ?? 0 : 0)
          : colFill.get(cc) ?? 0;
        baseExplain = `col ${cc}${flags.colorsReal ? ` (${scanColor})` : ""} count = ${raw}`;
      } else {
        const peers = c.t === CardType.SURR ? deck.surrPeers[i] : deck.diagPeers[i];
        for (const q of peers) {
          const qc = cards[q];
          if (qc.t === CardType.DEAD) continue;
          const m = !flags.colorsReal || qc.t === CardType.WILD || qc.color === scanColor;
          if (m) raw++;
        }
        baseExplain = `${c.t === CardType.SURR ? "surrounding" : "diagonal"} peers = ${raw}`;
      }
      const fm = freqMult[c.t] ?? 1;
      const scaled = raw * fm;
      if (fm !== 1) baseExplain += ` × ${fm} (deck implicit)`;
      base = scaled;
    } else if (c.t === CardType.DELUXE) {
      base = cfg.deluxe.flat;
      baseExplain = `deluxe flat value = ${cfg.deluxe.flat}`;
    } else {
      base = 1.0;
      baseExplain = "typeless flat value = 1.0";
    }

    // Cores.
    const applied: CoreComponent[] = [...baseline];
    const excluded: ExcludedCore[] = [...classExcluded];
    const gg = cardGameGroups(c, isShiny);
    for (const sc of statComps) {
      if (gg.has("Stat")) applied.push(sc);
      else excluded.push({ core_type: sc.core_type, color: null, reason: "boosts Stat cards only (evolution cards carry no Stat)" });
    }
    if (sparkComp !== null) {
      if (gg.has("Shiny")) applied.push(sparkComp);
      else excluded.push({ core_type: CoreType.SPARKLING, color: null, reason: "boosts Shiny cards only (deluxe and plain stat cards are not Shiny)" });
    }
    if (colorComp !== null) {
      const applies = !flags.colorsReal
        || (colorComp.color !== null
            && (c.t === (CardType.WILD as CardType) || c.color === colorComp.color));
      if (applies) applied.push(colorComp);
      else excluded.push({ core_type: CoreType.COLOR, color: colorComp.color,
        reason: `card color is ${c.color ?? "—"} (color core is ${colorComp.color ?? "?"})` });
    }
    // Premium boosts every card, Deluxe included: in 0.34.1 its boostGroups list is empty, so the game's
    // GroupSynergyDeckModifier does not enforce the "non-Deluxe" tooltip.
    if (deluxeComp !== null) applied.push(deluxeComp);
    if (voidComp !== null) applied.push(voidComp);
    // Archive (live semantics): additive stack term like every other core;
    // its value is base^n_arcane (see cores.archiveCoreMult). No per-card gate.
    if (archiveComp !== null) applied.push(archiveComp);

    // Additive implicit addends for this card.
    const implicitParts: ImplicitPart[] = [];
    for (const imp of implicits) {
      switch (imp.kind) {
        case "global": {
          const groupOk = imp.groups.every((g) => c.groups.includes(g as GroupTag));
          const colorOk = imp.colors.length === 0 || !flags.colorsReal
            || (c.color !== null && imp.colors.includes(c.color));
          if (groupOk && colorOk) {
            implicitParts.push({ label: `implicit +${imp.value} (${imp.groups.join("+") || imp.colors.join("/") || "all"})`, addend: imp.value });
          }
          break;
        }
        case "adjacency": {
          const peers = imp.extra === "surrounding" ? deck.surrPeers[i] : deck.colPeers[i];
          let matches = 0;
          for (const q of peers) {
            const qc = cards[q];
            if (qc.t === CardType.DEAD || GREEDS.has(qc.t)) continue;
            if (qc.t === CardType.WILD || qc.groups.includes(imp.groups[0] as GroupTag)) matches++;
          }
          if (matches > 0) {
            implicitParts.push({ label: `implicit +${imp.value} × ${matches} ${imp.groups[0]} in ${imp.extra === "surrounding" ? "range" : "column"}`, addend: imp.value * matches });
          }
          break;
        }
        case "color_mismatch": {
          let mism = 0;
          for (const j of orth[i]) {
            if (j === undefined) continue;
            const qc = cards[j];
            if (qc.t === CardType.DEAD) continue;
            if (!flags.colorsReal) mism++;
            else if (qc.t === CardType.WILD || (qc.color !== null && qc.color !== c.color)) mism++;
          }
          if (mism > 0) implicitParts.push({ label: `implicit +${imp.value} × ${mism} mismatched neighbors`, addend: imp.value * mism });
          break;
        }
        case "row_pos": {
          const dist = rowMax - deck.slots[i][0] + 1;
          implicitParts.push({ label: `implicit +${imp.value} × row ${dist} from bottom`, addend: imp.value * dist });
          break;
        }
        case "chain": {
          const id = chainId[i];
          if (id !== 0 && chainSize[id] > 1) {
            implicitParts.push({ label: `implicit +${chainValue} × ${chainSize[id] - 1} chain links`, addend: chainValue * (chainSize[id] - 1) });
          }
          break;
        }
        case "unique_groups": {
          if (c.groups.includes("Stat")) {
            implicitParts.push({ label: `implicit +${uniqueValue} × ${uniqueGroupsCount} unique groups`, addend: uniqueValue * uniqueGroupsCount });
          }
          break;
        }
      }
    }
    // 0.34.1 addon cores (kernel core_implicits, same order as the cores).
    const hasPeer = (peers: number[], group: string) => peers.some((q) => {
      const qc = cards[q];
      return qc.t !== CardType.DEAD && !GREEDS.has(qc.t)
        && (qc.t === CardType.WILD || qc.groups.includes(group as GroupTag));
    });
    for (const { spec, value, label } of addonCores) {
      const tag = TAG_CORE_GROUP[spec.core_type];
      if (tag !== undefined) {
        if (c.groups.includes(tag)) implicitParts.push({ label: `${label} +${(value - 1).toFixed(2)} (${tag})`, addend: value - 1 });
        continue;
      }
      switch (spec.core_type) {
        case CoreType.BAZAAR:
          if (hasPeer(deck.surrPeers[i], "Resource")) implicitParts.push({ label: `${label} +${(value - 1).toFixed(2)} (Resource neighbour)`, addend: value - 1 });
          break;
        case CoreType.TALENT: {
          let m = 0;
          for (const q of deck.surrPeers[i]) {
            const qc = cards[q];
            if (qc.t === CardType.DEAD || GREEDS.has(qc.t)) continue;
            if (qc.t === CardType.WILD || qc.groups.includes("Knack")) m++;
          }
          if (m > 0) implicitParts.push({ label: `${label} +${value} × ${m} Knack neighbours`, addend: value * m });
          break;
        }
        case CoreType.TEMPORAL:
          if (hasPeer(starcross[i], "Temporal")) implicitParts.push({ label: `${label} +${(value - 1).toFixed(2)} (Temporal in star-cross)`, addend: value - 1 });
          break;
        case CoreType.JUPITER:
          if (jupiterGroup !== null && gg.has(jupiterGroup)) implicitParts.push({ label: `${label} +${(value - 1).toFixed(2)} (most common: ${jupiterGroup})`, addend: value - 1 });
          break;
        case CoreType.PLUTO:
          if (plutoGroup !== null && gg.has(plutoGroup)) implicitParts.push({ label: `${label} +${(value - 1).toFixed(2)} (least common: ${plutoGroup})`, addend: value - 1 });
          break;
        case CoreType.NITWIT:
          if (nitwitSlot.has(i)) implicitParts.push({ label: `${label} +${(value - 1).toFixed(2)} (arcane slot)`, addend: value - 1 });
          break;
      }
    }
    if (emptyAddend > 0) {
      implicitParts.push({ label: `implicit +${emptyAddend.toFixed(3)} (empty slots)`, addend: emptyAddend });
    }
    const impAddend = implicitParts.reduce((a, p) => a + p.addend, 0);

    // Core-mult composition.
    const vals = applied.map((x) => x.value);
    let coreMult: number, formula: string;
    if (cfg.stacking.additive_cores) {
      coreMult = 1.0 + vals.reduce((a, v) => a + (v - 1), 0) + impAddend;
      formula = "1 + " + vals.map((v) => `(${v.toFixed(3)}-1)`).join(" + ")
        + (impAddend !== 0 ? ` + ${impAddend.toFixed(3)} (implicit)` : "");
      if (vals.length === 0 && impAddend === 0) formula = "1.0 (no cores apply)";
    } else {
      coreMult = vals.length ? vals.reduce((a, b) => a * b, 1.0) : 1.0;
      formula = vals.length ? vals.map((v) => v.toFixed(3)).join(" × ") : "1.0 (no cores apply)";
    }

    // Runic mirror (multiplicative).
    let mirrorFactor = 1.0;
    if (haveMirror) {
      let pass: boolean;
      if (mirrorSelf[i]) pass = true;
      else {
        const mi = mirrorOf[i];
        if (mi === undefined) pass = false;
        else {
          const mc = cards[mi];
          if (mc.t === CardType.DEAD) pass = false;
          else if (!flags.colorsReal) pass = true;
          else pass = mc.t === CardType.WILD || (mc.color !== null && mc.color === c.color);
        }
      }
      if (pass) mirrorFactor = mirrorValue;
    }

    const b = cfg.stacking.greed_additive ? Math.max(boost[i], 1.0) : boost[i];
    const v = base * coreMult * b * mirrorFactor;
    perSlot.set(k, {
      cardType: c.t, color: c.color, scaleColor: c.scaleColor, groups: c.groups,
      baseValue: base, baseExplain,
      appliedCores: applied, excludedCores: excluded, implicitParts,
      coreMult, coreMultFormula: formula,
      boost: b, boostSources: boostSources[i],
      archiveMult, archiveArcaneCount: nArcane, archiveBase,
      mirrorFactor, finalNdm: v,
    });
    total += v;
  }

  return { total, perSlot, nNs, nDead };
}
