// Card / core / color enums — string values must stay in lockstep with
// src/types.py (Python) and the `from_str` matchers in ndm_core (Rust).

export const CardType = {
  ROW:             "row",
  COL:             "col",
  SURR:            "surr",
  DIAG:            "diag",
  DELUXE:          "deluxe",
  TYPELESS:        "typeless",
  DIR_GREED_UP:    "dir_greed_up",
  DIR_GREED_DOWN:  "dir_greed_down",
  DIR_GREED_LEFT:  "dir_greed_left",
  DIR_GREED_RIGHT: "dir_greed_right",
  DIR_GREED_NE:    "dir_greed_ne",
  DIR_GREED_NW:    "dir_greed_nw",
  DIR_GREED_SE:    "dir_greed_se",
  DIR_GREED_SW:    "dir_greed_sw",
  EVO_GREED:       "evo_greed",
  SURR_GREED:      "surr_greed",
  FILLER_GREED:    "filler_greed",
  EMPTY:           "empty",
  DEAD:            "dead",
  // Placeable only in arcane (`A`) slots. 0 NDM directly, no cores apply,
  // no greed boost. Counts in n_ns (EVO-no-FOIL only, "treat like regulars")
  // and participates in same-color row/col/peer counts for neighbors.
  ARCANE:          "arcane",
  // Optimizer 2.0: the Wild card — 0 NDM itself, counts as ANY group and
  // ANY color for neighbors' positional scaling (universal wildcard).
  WILD:            "wild",
} as const;
export type CardType = typeof CardType[keyof typeof CardType];

export const Color = {
  RED:    "red",
  GREEN:  "green",
  BLUE:   "blue",
  YELLOW: "yellow",
} as const;
export type Color = typeof Color[keyof typeof Color];
export const ALL_COLORS: readonly Color[] = [
  Color.RED, Color.GREEN, Color.BLUE, Color.YELLOW,
];

export const CardClass = { EVO: "evo", SHINY: "shiny" } as const;
export type CardClass = typeof CardClass[keyof typeof CardClass];

export const CoreType = {
  PURE:        "pure",
  EQUILIBRIUM: "equilibrium",
  STEADFAST:   "steadfast",
  // Sparkling — flat SHINY-only multiplier (Wold's-only). Same gating as
  // steadfast / equilibrium, just a different number.
  SPARKLING:   "sparkling",
  COLOR:       "color",
  FOIL:        "foil",
  DELUXE_CORE: "deluxe_core",
  // Void core: base + scale × n_dead. Applies to every non-DEAD scoring card.
  VOID_CORE:   "void_core",
  // Archive core: base ** n_arcane_placed, applied *outside* per-card core_mult.
  // Bypasses the additive_cores switch; only enumerated when the deck has any
  // arcane slot.
  ARCHIVE_CORE: "archive_core",
  // ── Structural cores (WEB ONLY) ───────────────────────────────────────────
  // These never reach the SA kernel. They mutate the deck layout in the UI
  // before optimize is dispatched and each costs one of the deck's core-slot
  // budget. Distinct from ARCHIVE_CORE (which IS a scoring multiplier).
  //
  //   CONSTRUCTION_CORE — player adds ≤3 new regular (`O`) slots to the grid,
  //                       each 8-adjacency-connected to the existing structure.
  //   ARCANE_CORE       — player converts ≤3 existing `O` slots to `A` (arcane).
  //                       Synergizes with ARCHIVE_CORE via raised n_arcane_placed.
  CONSTRUCTION_CORE: "construction_core",
  ARCANE_CORE:       "arcane_core",
} as const;
export type CoreType = typeof CoreType[keyof typeof CoreType];

// Category sets — mirror types.py and inventory_optimize.py.
export const GREED_TYPES: ReadonlySet<CardType> = new Set([
  CardType.DIR_GREED_UP,    CardType.DIR_GREED_DOWN,
  CardType.DIR_GREED_LEFT,  CardType.DIR_GREED_RIGHT,
  CardType.DIR_GREED_NE,    CardType.DIR_GREED_NW,
  CardType.DIR_GREED_SE,    CardType.DIR_GREED_SW,
  CardType.EVO_GREED,       CardType.SURR_GREED,
]);
export const POSITIONAL_TYPES: ReadonlySet<CardType> = new Set([
  CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
]);
export const DELUXE_TYPES:   ReadonlySet<CardType> = new Set([CardType.DELUXE]);
export const TYPELESS_TYPES: ReadonlySet<CardType> = new Set([CardType.TYPELESS]);

export type Position = readonly [number, number];

// A placed card on a slot — color is null only for DEAD (transparent filler).
export type Placed = readonly [CardType, Color | null];
export const DEAD_CARD: Placed = [CardType.DEAD, null];

// Core spec — `color` set only for CoreType.COLOR; `override` null means
// "use the config-bundled default multiplier".
export interface CoreSpec {
  core_type: CoreType;
  color:     Color | null;
  override:  number | null;
}

// ─── Optimizer 2.0 ───────────────────────────────────────────────────────────

export const OptimizerMode = {
  MAX:      "max",
  TARGETED: "targeted",
  EXACT:    "exact",
} as const;
export type OptimizerMode = typeof OptimizerMode[keyof typeof OptimizerMode];

export const Depth = { FAST: "fast", DEFAULT: "default", DEEP: "deep" } as const;
export type Depth = typeof Depth[keyof typeof Depth];

/** Depth slider → fixed SA params (spec §9.2). */
export const DEPTH_PARAMS: Record<Depth, { nIter: number; restarts: number }> = {
  fast:    { nIter:  50_000, restarts:  6 },
  default: { nIter:  75_000, restarts: 12 },
  deep:    { nIter: 125_000, restarts: 24 },
};

/** The 9 freeform category tags (spec §2.2 Bucket B), in canonical order. */
export const CATEGORY_GROUPS = [
  "Offensive", "Defensive", "Physical", "Magical", "Utility",
  "Resource", "Knack", "Temporal", "Essence",
] as const;
export type GroupTag = typeof CATEGORY_GROUPS[number] | "Foil" | "Stat";
export const ALL_GROUP_TAGS: readonly GroupTag[] =
  [...CATEGORY_GROUPS, "Foil", "Stat"];

/** A card placed by the 2.0 kernel: type + colors + carried tags. */
export interface TaggedPlaced {
  t:          CardType;
  color:      Color | null;
  scaleColor: Color | null;   // ≠ color only under Complex Cards
  groups:     GroupTag[];
}

/** One Targeted-mode constraint row. null = unbounded on that side. */
export interface TagRuleRow {
  axis: "color" | "type" | "group" | "greed";
  key:  string;                // color / card-type / group name; "" for greed
  min:  number | null;
  max:  number | null;
}

/** One Exact-mode inventory stack (identical cards ×count). */
export interface ExactStack {
  t:          CardType;
  color:      Color;
  scaleColor: Color;           // == color unless Complex Cards
  groups:     GroupTag[];      // real tags on the card (incl. Foil / Stat)
  count:      number;
  mustPlace:  boolean;         // per-stack lower bound = count when true
}

/** Real greed in 2.0 = the 4 orthogonal directions only (spec §2.3). */
export const REAL_GREEDS: readonly CardType[] = [
  CardType.DIR_GREED_UP, CardType.DIR_GREED_DOWN,
  CardType.DIR_GREED_LEFT, CardType.DIR_GREED_RIGHT,
];
