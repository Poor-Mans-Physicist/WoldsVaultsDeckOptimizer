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

/**
 * Favorable blanket groups for Max/Targeted (spec §3.1): exactly the
 * category tags the active implicits reward. Stat/Foil are run-derived in
 * the kernel and never blanket-assigned from here.
 */
export function blanketGroups(implicits: ImplicitPayload[]): GroupTag[] {
  const out: GroupTag[] = [];
  for (const imp of implicits) {
    for (const g of imp.groups) {
      if ((CATEGORY_GROUPS as readonly string[]).includes(g) && !out.includes(g as GroupTag)) {
        out.push(g as GroupTag);
      }
    }
  }
  return out;
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
