// Per-slot NDM breakdown for Optimizer 2.0 — a 1:1 TypeScript mirror of
// tagsim.rs::simulate() over a FIXED assignment. The result total must match
// the wasm kernel's score within 1e-6 (the verify badge depends on it), and
// the what-if tag-edit popup re-scores through this same function.

import {
  CardClass, CardType, Color, CoreType,
  type CoreSpec, type GroupTag, type TaggedPlaced,
} from "./types";
import type { Deck } from "./deck";
import type { ResolvedConfig } from "./config";
import type { ImplicitPayload } from "./implicits";
import {
  classifyCores, type CoreComponent, type ExcludedCore,
} from "./cores";

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

  // ── Pass 1: classify + counts ─────────────────────────────────────────
  const rowColor = new Map<string, number>();   // `${r}|${color}`
  const colColor = new Map<string, number>();
  const rowFill = new Map<number, number>();
  const colFill = new Map<number, number>();
  let nDeluxe = 0, nArcane = 0, nGreed = 0, nDead = 0, nNsPositional = 0;
  let groupsUnion = new Set<GroupTag>();
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
      }
      anyWild = true;
      for (const g of ["Offensive","Defensive","Physical","Magical","Utility","Resource","Knack","Temporal","Essence","Stat"] as GroupTag[]) {
        groupsUnion.add(g);
      }
    } else if (c.color !== null) {
      rowColor.set(`${r}|${c.color}`, (rowColor.get(`${r}|${c.color}`) ?? 0) + 1);
      colColor.set(`${cc}|${c.color}`, (colColor.get(`${cc}|${c.color}`) ?? 0) + 1);
    }
    for (const g of c.groups) groupsUnion.add(g);
    if (POSITIONAL.has(c.t)) {
      if (!c.groups.includes("Foil")) nNsPositional++;
    } else if (c.t === CardType.DELUXE) nDeluxe++;
    else if (c.t === CardType.ARCANE) nArcane++;
    else if (GREEDS.has(c.t)) nGreed++;
  }

  // n_ns (per-card foil, §5): greed + arcane + non-foil positionals.
  const nNs = nGreed + nArcane + nNsPositional;

  // ── Cores (identical math to classifyCores / kernel) ──────────────────
  const { baseline, colorComp, deluxeComp, voidComp, archiveComp, classExcluded } =
    classifyCores(cores, cardClass, nNs, nDeluxe, nDead, nArcane, cfg);
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
    if (cfg.stacking.greed_additive) boost[target] += amount;
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
    // Non-stat card (Resource/Temporal): provides no player stats → 0 NDM
    // itself; still fills its slot and feeds implicits that read it.
    if (c.groups.includes("Resource") || c.groups.includes("Temporal")) {
      perSlot.set(k, zero(i,
        "non-stat card (Resource/Temporal) — gives no stats, 0 NDM; still " +
        "counts as a neighbor, in n_ns, and for implicits that read it"));
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
      let scaled = raw * fm;
      if (fm !== 1) baseExplain += ` × ${fm} (deck implicit)`;
      if (c.t === CardType.DIAG) {
        const floored = Math.max(1, scaled);
        if (floored !== scaled) baseExplain += " (clamped to 1)";
        scaled = floored;
      }
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
    if (colorComp !== null) {
      const applies = !flags.colorsReal
        || (colorComp.color !== null
            && (c.t === (CardType.WILD as CardType) || c.color === colorComp.color));
      if (applies) applied.push(colorComp);
      else excluded.push({ core_type: CoreType.COLOR, color: colorComp.color,
        reason: `card color is ${c.color ?? "—"} (color core is ${colorComp.color ?? "?"})` });
    }
    if (deluxeComp !== null) {
      if (c.t === CardType.DELUXE) {
        excluded.push({ core_type: CoreType.DELUXE_CORE, color: null,
          reason: "deluxe core never boosts deluxe cards (they fuel it)" });
      } else applied.push(deluxeComp);
    }
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
