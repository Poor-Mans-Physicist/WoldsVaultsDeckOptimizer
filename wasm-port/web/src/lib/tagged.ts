// Optimizer 2.0 run assembly: mode → supply/constraint configuration for the
// tag-aware kernel (`runSaTagged`). One kernel, three configurations
// (spec §1): Max / Targeted / Exact differ only in what this module builds.

import {
  CardClass, CardType, Color, ALL_COLORS, OptimizerMode, REAL_GREEDS,
  type CoreSpec, type ExactStack, type TagRuleRow, type TaggedPlaced,
  type GroupTag,
} from "./types";
import type { Deck } from "./deck";
import type { ResolvedConfig } from "./config";
import {
  preferredMonoColor, resolveImplicits, splitBlanketAssignable,
  type ImplicitDef, type ImplicitPayload,
} from "./implicits";

export interface TaggedRunInput {
  deck:        Deck;
  cardClass:   CardClass;
  mode:        OptimizerMode;
  appMode:     string;             // "wolds" | "vanilla" — foil rules + implicit gate
  targetedRules: TagRuleRow[];     // Targeted only
  exactStacks:   ExactStack[];     // Exact only
  mysteryPicks:  [string, string] | null;
  /** The Deck-card implicit toggle (default ON). OFF ⇒ the run scores the
   *  bare layout — for comparing base vs implicit-boosted NDM. */
  implicitsEnabled: boolean;
  implicitCatalog: Record<string, ImplicitDef>;
  /** Category combos that exist on real cards (deck.ts::legalTagCombos). */
  legalCombos: string[][];
  complexCards: boolean;
  minStatPlaced: number;
  autoPlaceArcane: boolean;
  cores:       CoreSpec[];         // the user's core inventory (pre-enumeration)
  nIter:       number;
  restarts:    number;
  cfg:         ResolvedConfig;
}

/** One worker task: a candidate core combo × a restart chunk. */
export interface TaggedTask {
  combo:    CoreSpec[];
  restarts: number;
}

export interface KernelStack {
  t: string; color: string; scale_color: string;
  groups: string[]; count: number | null; min_place: number;
}

// ── Mode-derived flags ────────────────────────────────────────────────────────

export function colorsRealFor(input: TaggedRunInput): boolean {
  if (input.mode === OptimizerMode.EXACT) return true;
  if (input.complexCards) return true;
  // color_mismatch (puzzle) scores MISMATCHED neighbor colors. Under blanket
  // mono colors the kernel would assume max mismatch while the grid displays
  // a single-color deck — score and display disagree. Optimize real colors
  // instead so the shown layout is the one being scored.
  if (activeImplicits(input).some((i) => i.kind === "color_mismatch")) return true;
  if (input.mode === OptimizerMode.TARGETED) {
    return input.targetedRules.some(
      (r) => r.axis === "color" && (r.min !== null || r.max !== null),
    );
  }
  return false;
}

export function activeImplicits(input: TaggedRunInput): ImplicitPayload[] {
  if (input.appMode === "vanilla") return [];   // implicits are Wold's-only (§8)
  if (!input.implicitsEnabled) return [];       // Deck-card toggle (base-layout runs)
  return resolveImplicits(
    input.deck.implicit, input.mysteryPicks, input.implicitCatalog,
  );
}

// ── Supply construction ───────────────────────────────────────────────────────

function scorableTypesFor(input: TaggedRunInput): CardType[] {
  const { cardClass, cfg } = input;
  const types: CardType[] = [];
  if (cardClass === CardClass.SHINY && !cfg.shiny.positional) {
    types.push(CardType.TYPELESS);
  } else {
    types.push(CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
               CardType.TYPELESS);
  }
  if (cfg.deluxe.allow) types.push(CardType.DELUXE);
  return types;
}

/** Unlimited supply for Max / Targeted. Mono-color when colors aren't real
 *  (the mono color follows a color-keyed implicit so the displayed deck
 *  matches the build guidance — scoring is color-blind regardless);
 *  full color (× scale-color under Complex) otherwise. */
function unlimitedStacks(
  input: TaggedRunInput,
  colorsReal: boolean,
  implicits: ImplicitPayload[],
): KernelStack[] {
  const stacks: KernelStack[] = [];
  const scorable = scorableTypesFor(input);
  const mono = (preferredMonoColor(implicits) as Color | null) ?? Color.RED;
  const colors: Color[] = colorsReal ? [...ALL_COLORS] : [mono];
  const push = (t: CardType, color: string, scale: string) =>
    stacks.push({ t, color, scale_color: scale, groups: [], count: null, min_place: 0 });

  for (const t of scorable) {
    for (const c of colors) {
      const isPositional = t === CardType.ROW || t === CardType.COL
        || t === CardType.SURR || t === CardType.DIAG;
      if (input.complexCards && isPositional) {
        // Complex: positional cards may scale off a different color (§7).
        for (const sc of ALL_COLORS) push(t, c, sc);
      } else {
        push(t, c, c);
      }
    }
  }
  for (const g of REAL_GREEDS) {
    for (const c of colors) {
      if (input.complexCards) {
        for (const sc of ALL_COLORS) push(g, c, sc);
      } else {
        push(g, c, c);
      }
    }
  }
  if (input.deck.arcaneSlots.length > 0) {
    for (const c of colors) push(CardType.ARCANE, c, c);
  }
  // Wild: a universal-match neighbor — only meaningful when colors are real
  // (under mono assumptions any card already matches everything). Targeted
  // only; Max never needs it, Exact supplies it explicitly.
  if (input.mode === OptimizerMode.TARGETED && colorsReal) {
    push(CardType.WILD, Color.RED, Color.RED);
  }
  return stacks;
}

function exactStacksToKernel(input: TaggedRunInput): KernelStack[] {
  return input.exactStacks
    // Complex OFF: mismatched-scale cards are greyed out and ignored (§7).
    .filter((s) => input.complexCards || s.scaleColor === s.color)
    .filter((s) => s.count > 0)
    .map((s) => ({
      t: s.t,
      color: s.color,
      scale_color: s.scaleColor,
      groups: s.groups,
      count: s.count,
      min_place: s.mustPlace ? s.count : 0,
    }));
}

// ── Rules ─────────────────────────────────────────────────────────────────────

function kernelRules(input: TaggedRunInput): { axis: string; key: string; min: number; max: number | null }[] {
  if (input.mode !== OptimizerMode.TARGETED) return [];
  const out: { axis: string; key: string; min: number; max: number | null }[] = [];
  for (const r of input.targetedRules) {
    if (r.min === null && r.max === null) continue;
    out.push({ axis: r.axis, key: r.key, min: r.min ?? 0, max: r.max });
  }
  return out;
}

/** Per-slot SA tag variables: non-stat implicit-relevant tags always
 *  (carrying one zeroes the card — the SA weighs each battery), plus, in
 *  Targeted, capped/min'd implicit-relevant tags (spec §1). Exact never
 *  toggles (real cards keep their real tags). */
function assignableGroups(input: TaggedRunInput, implicits: ImplicitPayload[]): GroupTag[] {
  if (input.mode === OptimizerMode.EXACT) return [];
  const { blanket, assignable } = splitBlanketAssignable(implicits, input.legalCombos);
  const out: GroupTag[] = [...assignable];
  if (input.mode === OptimizerMode.TARGETED) {
    const relevant = new Set(blanket);
    for (const r of input.targetedRules) {
      if (r.axis !== "group") continue;
      if (r.min === null && r.max === null) continue;
      if (relevant.has(r.key as GroupTag) && !out.includes(r.key as GroupTag)) {
        out.push(r.key as GroupTag);
      }
    }
  }
  return out;
}

// ── Payload assembly ──────────────────────────────────────────────────────────

export function buildTaggedPayload(
  input: TaggedRunInput,
  combo: CoreSpec[],
  restarts: number,
): Record<string, unknown> {
  const { deck, cfg } = input;
  const colorsReal = colorsRealFor(input);
  const implicits = activeImplicits(input);
  const exact = input.mode === OptimizerMode.EXACT;

  const stacks = exact ? exactStacksToKernel(input) : unlimitedStacks(input, colorsReal, implicits);
  const blanket = exact
    ? [] : splitBlanketAssignable(implicits, input.legalCombos).blanket;
  const assignable = assignableGroups(input, implicits);

  return {
    slots:      deck.slots.map(([r, c]) => [r, c] as [number, number]),
    row_peers:  deck.rowPeers,
    col_peers:  deck.colPeers,
    surr_peers: deck.surrPeers,
    diag_peers: deck.diagPeers,
    arcane_slot_indices: deck.arcaneSlotIndices,
    stacks,
    tag_rules: kernelRules(input),
    blanket_groups: blanket,
    assignable_groups: assignable,
    legal_combos: input.legalCombos,
    implicits,
    cores: combo.map((s) => [s.core_type, s.color ?? "", s.override ?? -1.0]),
    min_stat_placed: Math.max(0, input.minStatPlaced | 0),
    // §6 wasted-greed → non-foil-evo cleanup: Wold's-only model improvement;
    // the kernel itself gates on evo + FOIL-core.
    final_pass_nonfoil_evo: input.appMode !== "vanilla",
    exact_groups: exact,
    n_iter: input.nIter,
    restarts,
    mult_dir_vert:          cfg.greed.dir_vert,
    mult_dir_horiz:         cfg.greed.dir_horiz,
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
    is_shiny:               input.cardClass === CardClass.SHINY,
    auto_place_arcane:      input.autoPlaceArcane,
    colors_real:            colorsReal,
    complex_cards:          input.complexCards,
    wv_foil_rules:          input.appMode !== "vanilla",
    floor_counts_deluxe:    true,
  };
}

/** Kernel result → typed per-slot cards (parallel to deck.slots). */
export function parseKernelAssignment(
  raw: { t: string; color: string; scale_color: string; groups: string[] }[],
): TaggedPlaced[] {
  return raw.map((p) => ({
    t: p.t as CardType,
    color: (p.color || null) as Color | null,
    scaleColor: (p.scale_color || null) as Color | null,
    groups: p.groups as GroupTag[],
  }));
}

/**
 * Canonical Targeted panel rows (spec §9.3): colors and positional/card
 * types first, then the freeform tags. All min/max start unbounded.
 */
export function defaultTargetedRules(): TagRuleRow[] {
  const rows: TagRuleRow[] = [];
  for (const c of ALL_COLORS) rows.push({ axis: "color", key: c, min: null, max: null });
  for (const t of [CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
                   CardType.DELUXE, CardType.TYPELESS, CardType.ARCANE, CardType.WILD]) {
    rows.push({ axis: "type", key: t, min: null, max: null });
  }
  rows.push({ axis: "greed", key: "", min: null, max: null });
  for (const g of REAL_GREEDS) rows.push({ axis: "type", key: g, min: null, max: null });
  for (const g of ["Offensive", "Defensive", "Physical", "Magical", "Utility",
                   "Resource", "Knack", "Temporal", "Essence", "Foil"]) {
    rows.push({ axis: "group", key: g, min: null, max: null });
  }
  return rows;
}

/**
 * Split (candidates × restarts) into worker tasks so every pool worker gets
 * work even when there's a single candidate ("1 core per restart"): per
 * candidate, restarts are chunked so total tasks ≈ pool size.
 */
export function buildTasks(
  candidates: CoreSpec[][],
  restarts: number,
  poolSize: number,
): TaggedTask[] {
  const perCandidate = Math.max(1, Math.round(poolSize / Math.max(1, candidates.length)));
  const chunk = Math.max(1, Math.ceil(restarts / perCandidate));
  const tasks: TaggedTask[] = [];
  for (const combo of candidates) {
    let left = restarts;
    while (left > 0) {
      const n = Math.min(chunk, left);
      tasks.push({ combo, restarts: n });
      left -= n;
    }
  }
  return tasks;
}
