// Deck implicits (Optimizer 2.0, Wold's-only). Typed view of the entries
// build_data.py attaches to each wolds RawDeck (from decks/wolds_implicits.json)
// plus the full catalog for the Mystery deck's pair-picker.
//
// Evaluation semantics live in the Rust kernel (tagsim.rs) and are mirrored
// in taggedBreakdown.ts; this module is data + payload conversion + the
// favorable-blanket derivation for Max/Targeted.

import { CATEGORY_GROUPS, type GroupTag } from "./types";

export interface ImplicitDef {
  kind:   "global" | "freq" | "adjacency" | "color_mismatch" | "row_pos"
        | "chain" | "empty_slots" | "unique_groups" | "mirror"
        | "gameplay" | "mystery";
  value?:  number;
  groups?: string[];       // global: ALL required
  group?:  string;         // adjacency: the single group counted in range
  colors?: string[];       // global: card color must be one of these
  ptype?:  string;         // freq: "col" | "surr" | "diag"
  range?:  string;         // adjacency: "column" | "surrounding"
  name?:   string;
  desc?:   string;
}

/** Kernel payload shape (tagsim_wasm::ImplicitIn). */
export interface ImplicitPayload {
  kind:   string;
  value:  number;
  groups: string[];
  colors: string[];
  extra:  string;
}

/** Is this implicit NDM-relevant (i.e. reaches the kernel at all)? */
export function isScoringImplicit(def: ImplicitDef | null | undefined): boolean {
  return !!def && def.kind !== "gameplay" && def.kind !== "mystery";
}

export function toPayload(def: ImplicitDef): ImplicitPayload {
  return {
    kind:   def.kind,
    value:  def.value ?? 0,
    groups: def.group ? [def.group] : (def.groups ?? []),
    colors: def.colors ?? [],
    extra:  def.ptype ?? def.range ?? "",
  };
}

/**
 * Resolve the implicit payloads for a run: the deck's own implicit, or the
 * user-picked pair for the Mystery deck. Gameplay-only entries yield [].
 */
export function resolveImplicits(
  deckImplicit: ImplicitDef | null,
  mysteryPicks: [string, string] | null,
  catalog:      Record<string, ImplicitDef>,
): ImplicitPayload[] {
  if (!deckImplicit) return [];
  if (deckImplicit.kind === "mystery") {
    if (!mysteryPicks) return [];
    const out: ImplicitPayload[] = [];
    for (const key of mysteryPicks) {
      const def = catalog[key];
      if (isScoringImplicit(def)) out.push(toPayload(def));
    }
    return out;
  }
  if (!isScoringImplicit(deckImplicit)) return [];
  return [toPayload(deckImplicit)];
}

/** Non-stat categories: cards carrying these give no player stats → 0 NDM
 *  (kernel NONSTAT_GROUPS). Never blanket-assigned; always a per-slot SA
 *  decision ("battery" cards). */
export const NONSTAT_TAGS: readonly GroupTag[] = ["Resource", "Temporal"];

/**
 * Which category tags the active implicits actually read. `unique_groups`
 * (mutant) rewards diversity, so every category is relevant to it.
 */
export function relevantGroups(implicits: ImplicitPayload[]): GroupTag[] {
  const out: GroupTag[] = [];
  for (const imp of implicits) {
    if (imp.kind === "unique_groups") return [...CATEGORY_GROUPS];
    for (const g of imp.groups) {
      if ((CATEGORY_GROUPS as readonly string[]).includes(g) && !out.includes(g as GroupTag)) {
        out.push(g as GroupTag);
      }
    }
  }
  return out;
}

/**
 * Is this category set buildable as a REAL card? True iff it's a subset of
 * some real card's category set (Wild excepted — never checked here).
 * An empty combo catalog means "no data" → unconstrained.
 */
export function isLegalCategorySet(
  tags: readonly GroupTag[],
  combos: readonly string[][],
): boolean {
  if (combos.length === 0) return true;
  const cats = tags.filter((g) =>
    (CATEGORY_GROUPS as readonly string[]).includes(g));
  if (cats.length === 0) return true;
  return combos.some((combo) => cats.every((g) => combo.includes(g)));
}

/**
 * Split the implicit-relevant tags for Max/Targeted (spec §3.1, amended):
 *  - blanket    — stat-safe tags, assigned free to every non-greed card
 *                 (genuinely NDM-inert, so "assign to all" stays exact).
 *  - assignable — non-stat tags (Resource/Temporal): carrying one zeroes
 *                 the card, so the SA decides per slot whether a battery
 *                 is worth it (merchant's column feeders, mutant diversity).
 *
 * When the blanket UNION isn't buildable as one real card (possible with
 * Mystery pairs, e.g. champion+fairy wanting Physical+Magical), the whole
 * blanket demotes to assignable — the SA then picks a legal subset per
 * slot under the kernel's combo check.
 */
export function splitBlanketAssignable(
  implicits: ImplicitPayload[],
  combos: readonly string[][] = [],
): { blanket: GroupTag[]; assignable: GroupTag[] } {
  let blanket: GroupTag[] = [];
  const assignable: GroupTag[] = [];
  for (const g of relevantGroups(implicits)) {
    if (NONSTAT_TAGS.includes(g)) assignable.push(g);
    else blanket.push(g);
  }
  if (!isLegalCategorySet(blanket, combos)) {
    assignable.push(...blanket);
    blanket = [];
  }
  return { blanket, assignable };
}

/** Back-compat helper: the stat-safe blanket tags only. */
export function blanketGroups(implicits: ImplicitPayload[]): GroupTag[] {
  return splitBlanketAssignable(implicits).blanket;
}

/**
 * The color a color-keyed implicit wants (idona/velara/tenos/wendarr,
 * gilded/ornate/living). Max's mono-color supply uses it so the DISPLAYED
 * deck matches the build guidance — scoring is color-blind either way.
 */
export function preferredMonoColor(implicits: ImplicitPayload[]): string | null {
  for (const imp of implicits) {
    if (imp.colors.length > 0) return imp.colors[0];
  }
  return null;
}

/** Short human line for the deck card / Max readout. */
export function implicitSummary(def: ImplicitDef | null): string | null {
  if (!def) return null;
  return def.desc ?? def.name ?? def.kind;
}

/** Catalog keys eligible for the Mystery pair-picker (everything except
 *  mystery itself), sorted by display name. */
export function mysteryChoices(catalog: Record<string, ImplicitDef>): [string, ImplicitDef][] {
  return Object.entries(catalog)
    .filter(([k, d]) => k !== "mystery" && d.kind !== "mystery")
    .sort((a, b) => (a[1].name ?? a[0]).localeCompare(b[1].name ?? b[0]));
}
