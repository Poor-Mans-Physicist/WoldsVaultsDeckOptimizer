// Per-slot NDM breakdown — re-score on a final assignment so the UI can
// show exactly what contributed to each slot.
//
// Port of `simulate_inventory_breakdown` in src/inventory_optimize.py. Uses
// the same classify-once / fold-into-one-multiplier approach.

import {
  CardClass, CardType, Color, CoreType,
  GREED_TYPES, POSITIONAL_TYPES,
  type CoreSpec, type Placed, type Position,
} from "./types";
import type { Deck } from "./deck";
import type { ResolvedConfig } from "./config";
import {
  classifyCores,
  type CoreComponent, type ExcludedCore,
} from "./cores";

export interface GreedSource {
  fromPosition: Position;
  greedType:    CardType;
  multiplier:   number;
}

export interface SlotBreakdown {
  cardType:            CardType;
  color:               Color | null;
  baseValue:           number;
  baseExplain:         string;
  appliedCores:        CoreComponent[];
  excludedCores:       ExcludedCore[];
  coreMult:            number;
  coreMultFormula:     string;
  boost:               number;
  boostSources:        GreedSource[];
  // Archive core (applied OUTSIDE core_mult). = 1.0 when archive isn't picked
  // or doesn't apply to this card class.
  archiveMult:         number;
  archiveArcaneCount:  number;
  archiveBase:         number;       // per-arcane base used (default or override)
  finalNdm:            number;
}

export interface BreakdownResult {
  total:        number;
  perSlot:      Map<string, SlotBreakdown>;   // key = `${r},${c}`
}

const posKey = (p: Position) => `${p[0]},${p[1]}`;

/** Helper — apply greed (additive or multiplicative) into a Map<key,number>. */
function applyGreed(boost: Map<string, number>, key: string, amount: number, additive: boolean) {
  // Additive: boost is a raw sum of greed multipliers pointing at this slot
  // (use-site floors at 1 via Math.max). Multiplicative: each greed scales
  // the running boost.
  if (!boost.has(key)) return;
  const v = boost.get(key)!;
  boost.set(key, additive ? v + amount : v * amount);
}

export function simulateInventoryBreakdown(
  deck:       Deck,
  assignment: Map<string, Placed>,             // key = posKey
  card_class: CardClass,
  cores:      readonly CoreSpec[],
  cfg:        ResolvedConfig,
): BreakdownResult {
  // ── Partition ────────────────────────────────────────────────────────────
  const positional = new Map<string, Placed>();
  const deluxe     = new Map<string, Placed>();
  const typeless   = new Map<string, Placed>();
  const greed      = new Map<string, Placed>();
  const arcane     = new Map<string, Placed>();
  const colored    = new Map<string, Color>();
  let n_dead = 0;

  for (const [k, [t, c]] of assignment) {
    if (t === CardType.DEAD) { n_dead++; continue; }
    if (POSITIONAL_TYPES.has(t))      positional.set(k, [t, c]);
    else if (t === CardType.DELUXE)   deluxe.set(k, [t, c]);
    else if (t === CardType.TYPELESS) typeless.set(k, [t, c]);
    else if (t === CardType.ARCANE)   arcane.set(k, [t, c]);
    else if (GREED_TYPES.has(t))      greed.set(k, [t, c]);
    // ARCANE participates in same-color counts so its neighbors see it.
    if (c !== null) colored.set(k, c);
  }

  // Same-color row/col counts.
  const rowCount = new Map<string, number>();   // `${r}|${color}`
  const colCount = new Map<string, number>();   // `${c}|${color}`
  for (const [k, color] of colored) {
    const [r, c] = k.split(",").map(Number);
    const rk = `${r}|${color}`;
    const ck = `${c}|${color}`;
    rowCount.set(rk, (rowCount.get(rk) ?? 0) + 1);
    colCount.set(ck, (colCount.get(ck) ?? 0) + 1);
  }

  // ── n_ns / n_deluxe ──────────────────────────────────────────────────────
  // Mirrors the classic CLI kernel (`src/simulate.py::simulate` /
  // `ndm_core/src/lib.rs::run_sa_optimize`). ARCANE always counts; on top:
  //   EVO-no-FOIL  → positional + greed (+ arcane)
  //   EVO+FOIL     → greed (+ arcane)
  //   SHINY        → greed (+ arcane)
  // TYPELESS / DELUXE are intentionally excluded — typeless is "always
  // shiny" by classic-kernel design and deluxe scores via DELUXE_CORE.
  const foilActive = cores.some((s) => s.core_type === CoreType.FOIL);
  const n_ns = (card_class === CardClass.EVO && !foilActive)
    ? positional.size + arcane.size + greed.size
    : greed.size + arcane.size;
  const n_deluxe = deluxe.size;

  const { baseline, colorComp, deluxeComp, voidComp, archiveComp, classExcluded } =
    classifyCores(cores, card_class, n_ns, n_deluxe, n_dead, arcane.size, cfg);
  // Archive multiplier — applied *outside* the per-card core_mult (bypasses
  // the additive_cores stacking switch). Value = base ^ n_arcane_placed.
  const archiveMult = archiveComp !== null ? archiveComp.value : 1.0;
  // Pull the picked spec so we can show "base ^ n = mult" in the popup.
  const archiveSpec = cores.find((s) => s.core_type === CoreType.ARCHIVE_CORE) ?? null;
  const archiveBase = archiveSpec === null
    ? 1.0
    : (archiveSpec.override !== null ? archiveSpec.override : cfg.cores.archive_core);
  const archiveArcaneCount = arcane.size;

  // ── Greed → boost (with provenance) ──────────────────────────────────────
  // Additive starts at 0 (use-site floors at 1); multiplicative starts at 1.
  const scorable = new Set<string>([...positional.keys(), ...deluxe.keys(), ...typeless.keys()]);
  const boost = new Map<string, number>();
  const initBoost = cfg.stacking.greed_additive ? 0.0 : 1.0;
  for (const k of scorable) boost.set(k, initBoost);
  const boostSources = new Map<string, GreedSource[]>();
  for (const k of scorable) boostSources.set(k, []);

  // Map (slot-index → posKey) for SURR/DIAG peer-set walks.
  const slotKey = (i: number) => posKey(deck.slots[i]);
  const slotIndex = new Map<string, number>();
  deck.slots.forEach((p, i) => slotIndex.set(posKey(p), i));

  const tryApply = (srcKey: string, srcType: CardType, targetKey: string, amount: number) => {
    if (!scorable.has(targetKey)) return;
    applyGreed(boost, targetKey, amount, cfg.stacking.greed_additive);
    boostSources.get(targetKey)!.push({
      fromPosition: posKeyToPos(srcKey), greedType: srcType, multiplier: amount,
    });
  };

  for (const [gk, [gt, _gc]] of greed) {
    const [gr, gcc] = posKeyToPos(gk);
    switch (gt) {
      case CardType.DIR_GREED_UP:    tryApply(gk, gt, posKey([gr - 1, gcc]),     cfg.greed.dir_vert); break;
      case CardType.DIR_GREED_DOWN:  tryApply(gk, gt, posKey([gr + 1, gcc]),     cfg.greed.dir_vert); break;
      case CardType.DIR_GREED_LEFT:  tryApply(gk, gt, posKey([gr, gcc - 1]),     cfg.greed.dir_horiz); break;
      case CardType.DIR_GREED_RIGHT: tryApply(gk, gt, posKey([gr, gcc + 1]),     cfg.greed.dir_horiz); break;
      case CardType.DIR_GREED_NE:    tryApply(gk, gt, posKey([gr - 1, gcc + 1]), cfg.greed.dir_diag_up); break;
      case CardType.DIR_GREED_NW:    tryApply(gk, gt, posKey([gr - 1, gcc - 1]), cfg.greed.dir_diag_up); break;
      case CardType.DIR_GREED_SE:    tryApply(gk, gt, posKey([gr + 1, gcc + 1]), cfg.greed.dir_diag_down); break;
      case CardType.DIR_GREED_SW:    tryApply(gk, gt, posKey([gr + 1, gcc - 1]), cfg.greed.dir_diag_down); break;
      case CardType.EVO_GREED: {
        if (card_class === CardClass.EVO) {
          const tk = posKey([gr + 1, gcc]);
          if (positional.has(tk)) tryApply(gk, gt, tk, cfg.greed.evo);
        }
        break;
      }
      case CardType.SURR_GREED: {
        const idx = slotIndex.get(gk);
        if (idx !== undefined) {
          for (const peer of deck.surrPeers[idx]) tryApply(gk, gt, slotKey(peer), cfg.greed.surr);
        }
        break;
      }
    }
  }

  // ── Per-card breakdown (which cores apply / are excluded). ───────────────
  const cardBreakdown = (cardType: CardType, cardColor: Color | null): {
    applied: CoreComponent[]; excluded: ExcludedCore[]; mult: number; formula: string;
  } => {
    const applied:  CoreComponent[] = [...baseline];
    const excluded: ExcludedCore[]  = [...classExcluded];

    if (colorComp !== null) {
      if (cardColor === null) {
        excluded.push({ core_type: CoreType.COLOR, color: colorComp.color,
          reason: `card has no color (color core is ${colorComp.color ?? "?"})` });
      } else if (cardColor === colorComp.color) {
        applied.push(colorComp);
      } else {
        excluded.push({ core_type: CoreType.COLOR, color: colorComp.color,
          reason: `card color is ${cardColor} (color core is ${colorComp.color ?? "?"})` });
      }
    }

    if (deluxeComp !== null) {
      if (cardType === CardType.DELUXE) {
        excluded.push({ core_type: CoreType.DELUXE_CORE, color: null,
          reason: "deluxe core never boosts deluxe cards (they fuel it instead)" });
      } else {
        applied.push(deluxeComp);
      }
    }

    // Void: applies to every non-DEAD scoring card. DEAD cards never reach
    // here (their final_ndm is 0 via the zero-breakdown path), but the
    // gating exclusion is symmetric with deluxe.
    if (voidComp !== null) {
      if (cardType === CardType.DEAD) {
        excluded.push({ core_type: CoreType.VOID_CORE, color: null,
          reason: "void core never boosts dead cards (they fuel it instead)" });
      } else {
        applied.push(voidComp);
      }
    }

    const vals = applied.map((c) => c.value);
    let mult: number, formula: string;
    if (cfg.stacking.additive_cores) {
      mult = 1.0 + vals.reduce((acc, v) => acc + (v - 1), 0);
      formula = vals.length
        ? "1 + " + vals.map((v) => `(${v.toFixed(3)}-1)`).join(" + ")
        : "1.0 (no cores apply)";
    } else {
      mult = vals.length ? vals.reduce((a, b) => a * b, 1.0) : 1.0;
      formula = vals.length ? vals.map((v) => v.toFixed(3)).join(" × ") : "1.0 (no cores apply)";
    }
    return { applied, excluded, mult, formula };
  };

  const perSlot = new Map<string, SlotBreakdown>();
  let total = 0;
  const additive = cfg.stacking.greed_additive;

  const zero = (t: CardType, c: Color | null, explain: string): SlotBreakdown => ({
    cardType: t, color: c,
    baseValue: 0.0, baseExplain: explain,
    appliedCores: [], excludedCores: [],
    coreMult: 1.0, coreMultFormula: "(not scored)",
    boost: 1.0, boostSources: [],
    archiveMult: 1.0, archiveArcaneCount, archiveBase,
    finalNdm: 0.0,
  });

  // Greed / arcane / dead / empty.
  for (const p of deck.slots) {
    const k = posKey(p);
    if (scorable.has(k)) continue;
    if (greed.has(k)) {
      const [gt, gc] = greed.get(k)!;
      perSlot.set(k, zero(gt, gc, "greed card — provides boost to neighbors, no own NDM"));
    } else if (arcane.has(k)) {
      const [at, ac] = arcane.get(k)!;
      const colorStr = ac !== null ? ac : "—";
      perSlot.set(k, zero(at, ac,
        `arcane card (color ${colorStr}) — 0 NDM by design, no cores apply; ` +
        `counts in n_ns for Pure and in same-color peer counts for neighbors`));
    } else if (assignment.has(k)) {
      const [t, c] = assignment.get(k)!;
      perSlot.set(k, zero(t, c, "dead card — transparent, contributes nothing"));
    } else {
      perSlot.set(k, zero(CardType.EMPTY, null, "empty slot"));
    }
  }

  // Positional.
  for (const [k, [t, c]] of positional) {
    const [r, cc] = posKeyToPos(k);
    let posVal = 0; let explain = "";
    if (t === CardType.ROW) {
      posVal = c !== null ? (rowCount.get(`${r}|${c}`) ?? 0) : 0;
      explain = `row ${r}, color ${c ?? "—"} → row_count = ${posVal}`;
    } else if (t === CardType.COL) {
      posVal = c !== null ? (colCount.get(`${cc}|${c}`) ?? 0) : 0;
      explain = `col ${cc}, color ${c ?? "—"} → col_count = ${posVal}`;
    } else if (t === CardType.DIAG) {
      const idx = slotIndex.get(k)!;
      posVal = 1 + deck.diagPeers[idx].filter((j) => colored.get(slotKey(j)) === c).length;
      explain = `diag (self + same-color peers, color ${c ?? "—"}) = ${posVal}`;
    } else {  // SURR
      const idx = slotIndex.get(k)!;
      posVal = deck.surrPeers[idx].filter((j) => colored.get(slotKey(j)) === c).length;
      explain = `surrounding same-color peers (color ${c ?? "—"}) = ${posVal}`;
    }
    const { applied, excluded, mult, formula } = cardBreakdown(t, c);
    const b = additive ? Math.max(boost.get(k)!, 1.0) : boost.get(k)!;
    const v = posVal * mult * b * archiveMult;
    perSlot.set(k, {
      cardType: t, color: c,
      baseValue: posVal, baseExplain: explain,
      appliedCores: applied, excludedCores: excluded,
      coreMult: mult, coreMultFormula: formula,
      boost: b, boostSources: boostSources.get(k) ?? [],
      archiveMult, archiveArcaneCount, archiveBase,
      finalNdm: v,
    });
    total += v;
  }

  // Deluxe.
  for (const [k, [t, c]] of deluxe) {
    const { applied, excluded, mult, formula } = cardBreakdown(CardType.DELUXE, c);
    const b = additive ? Math.max(boost.get(k)!, 1.0) : boost.get(k)!;
    const v = cfg.deluxe.flat * mult * b * archiveMult;
    perSlot.set(k, {
      cardType: t, color: c,
      baseValue: cfg.deluxe.flat, baseExplain: `deluxe flat value = ${cfg.deluxe.flat}`,
      appliedCores: applied, excludedCores: excluded,
      coreMult: mult, coreMultFormula: formula,
      boost: b, boostSources: boostSources.get(k) ?? [],
      archiveMult, archiveArcaneCount, archiveBase,
      finalNdm: v,
    });
    total += v;
  }

  // Typeless.
  for (const [k, [t, c]] of typeless) {
    const { applied, excluded, mult, formula } = cardBreakdown(CardType.TYPELESS, c);
    const b = additive ? Math.max(boost.get(k)!, 1.0) : boost.get(k)!;
    const v = 1.0 * mult * b * archiveMult;
    perSlot.set(k, {
      cardType: t, color: c,
      baseValue: 1.0, baseExplain: "typeless flat value = 1.0",
      appliedCores: applied, excludedCores: excluded,
      coreMult: mult, coreMultFormula: formula,
      boost: b, boostSources: boostSources.get(k) ?? [],
      archiveMult, archiveArcaneCount, archiveBase,
      finalNdm: v,
    });
    total += v;
  }

  return { total, perSlot };
}

function posKeyToPos(k: string): Position {
  const [r, c] = k.split(",").map(Number);
  return [r, c];
}
