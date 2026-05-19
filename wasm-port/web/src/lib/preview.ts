// Preview-mode helpers — port of src/preview.py.
//
// Pure logic only (family resolution, abbreviations, stat aggregation,
// assignment-reset after Run). The actual UI dialog lives in the Svelte
// components; this module keeps the math + classification testable.

import { CardClass, CardType, type Position } from "./types";
import type { OptimizeResult } from "./optimize";
import type { CardEntry } from "./modifiers";

export type CardFamily = "shiny" | "evo" | "deluxe" | "typeless";

const POSITIONAL = new Set<CardType>([
  CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
]);

/** Which family of stat cards may be placed in this slot. */
export function slotFamily(slotType: CardType, deckClass: CardClass): CardFamily | null {
  if (POSITIONAL.has(slotType)) return deckClass === CardClass.SHINY ? "shiny" : "evo";
  if (slotType === CardType.DELUXE)   return "deluxe";
  if (slotType === CardType.TYPELESS) return "typeless";
  return null;
}

export function isAssignableSlot(slotType: CardType, deckClass: CardClass): boolean {
  return slotFamily(slotType, deckClass) !== null;
}

// Short attribute badge — mirrors the _ATTR_ABBREV table in preview.py.
const ATTR_ABBREV: Record<string, string> = {
  attack_damage: "ATK", attack_speed_percent: "AS%",
  ability_power: "AP", ability_power_percent: "AP%",
  health: "HP", health_percentile: "HP%", armor_percentile: "AR%",
  damage_increase: "DMG%", cooldown_reduction: "CDR",
  critical_hit_mitigation: "CHM", healing_effectiveness: "HEAL",
  knockback_resistance: "KBR", lucky_hit_chance: "LHC",
  mana_additive: "MANA", mana_additive_percentile: "MANA%",
  mana_regen: "MREG", mining_speed: "MINE", movement_speed: "MOVE",
  item_quantity: "IQ", item_rarity: "IR", soul_chance: "SOUL",
  resistance: "RES", area_of_effect: "AOE", thorns_damage_flat: "THRN",
  trap_disarming: "TRAP", added_ability_level: "ABIL",
  added_talent_level: "TAL", copiously: "COPI", random_vault_modifier: "RVM",
};

export function attrAbbrev(attributeShort: string): string {
  return ATTR_ABBREV[attributeShort] ?? attributeShort.slice(0, 4).toUpperCase();
}

// ── Stats aggregation ────────────────────────────────────────────────────
// Position key in the assignments map is always `${r},${c}` (same as the
// breakdown perSlot map) so we can index directly into result.breakdown.

export type AssignmentKey = string;                 // `${r},${c}`
export type AssignmentVal = { cardKey: string; tier: number };

export interface PreviewAggregate {
  flat:        Map<string, number>;
  percent:     Map<string, number>;
  nAssigned:   number;
  nAssignable: number;
}

export function aggregatePreview(
  result:       OptimizeResult | null,
  assignments:  Map<AssignmentKey, AssignmentVal>,
  deckClass:    CardClass,
  modifiers:    Map<string, CardEntry>,
): PreviewAggregate {
  const flat    = new Map<string, number>();
  const percent = new Map<string, number>();
  let nAssignable = 0, nAssigned = 0;

  if (result) {
    for (const [, [t]] of result.assignment) {
      if (isAssignableSlot(t, deckClass)) nAssignable++;
    }
  }

  for (const [key, { cardKey, tier }] of assignments) {
    const card = modifiers.get(cardKey);
    if (!card) continue;
    const ct = card.tiers.find((t) => t.tier === tier);
    if (!ct) continue;
    const ndm = result?.breakdown.perSlot.get(key)?.finalNdm ?? 0;
    const contribution = ct.value * ndm;
    const bucket = card.isPercent ? percent : flat;
    bucket.set(card.attribute, (bucket.get(card.attribute) ?? 0) + contribution);
    nAssigned++;
  }

  return { flat, percent, nAssigned, nAssignable };
}

/**
 * Drop assignments whose slot no longer hosts a card of the same family
 * (i.e. the deck / class / layout changed). Returns the number of dropped
 * entries so the caller can flash a toast if it wants.
 */
export function resetAssignmentsOnRun(
  assignments: Map<AssignmentKey, AssignmentVal>,
  result:      OptimizeResult | null,
  deckClass:   CardClass,
  modifiers:   Map<string, CardEntry>,
): { kept: Map<AssignmentKey, AssignmentVal>; dropped: number } {
  const kept = new Map<AssignmentKey, AssignmentVal>();
  let dropped = 0;
  const placedMap = result?.assignment ?? new Map();

  for (const [key, v] of assignments) {
    const card = modifiers.get(v.cardKey);
    if (!card) { dropped++; continue; }
    const placed = placedMap.get(key);
    if (!placed) { dropped++; continue; }
    if (slotFamily(placed[0], deckClass) !== card.family) { dropped++; continue; }
    kept.set(key, v);
  }
  return { kept, dropped };
}

/** Human label for a stat row. Strips namespace prefix, capitalizes words. */
export function humanAttrLabel(attribute: string): string {
  const short = attribute.includes(":") ? attribute.split(":", 2)[1] : attribute;
  return short.split("_").filter((w) => w.length > 0)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

export function posKeyToPos(key: AssignmentKey): Position {
  const [r, c] = key.split(",").map(Number);
  return [r, c] as Position;
}
