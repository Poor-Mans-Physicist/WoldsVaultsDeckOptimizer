// Tag-notch palette (spec §9.5). Foil = bright white is mandated; the rest
// follow the spec's proposed palette (author may adjust freely).
//
// Notched tags = the 9 Bucket-B category tags + Foil (+ Stat, shown gold).
// Card COLOR (red/green/blue/yellow) is conveyed by the tile itself, never
// a notch.

import type { GroupTag } from "./types";

export const NOTCH_COLOR: Record<GroupTag, string> = {
  Offensive: "#D7263D",   // crimson
  Defensive: "#3A6EA5",   // steel blue
  Physical:  "#B07D3B",   // bronze
  Magical:   "#7B2FBE",   // violet
  Utility:   "#189A8A",   // teal
  Resource:  "#2FA84F",   // emerald
  Knack:     "#E8A020",   // amber
  Temporal:  "#5BC8E8",   // sky
  Essence:   "#D6469E",   // magenta
  Stat:      "#E8C33B",   // gold
  Foil:      "#FFFFFF",   // bright white (mandated)
};

/** Wild's chartreuse — used for the Wild tile accent, not a group notch. */
export const WILD_COLOR = "#9BCF3B";

/** Render order: category tags first (canonical order), then Stat, then
 *  Foil last so the white notch sits at the end of the strip. */
const ORDER: readonly GroupTag[] = [
  "Offensive", "Defensive", "Physical", "Magical", "Utility",
  "Resource", "Knack", "Temporal", "Essence", "Stat", "Foil",
];

export function sortTags(tags: readonly GroupTag[]): GroupTag[] {
  return [...tags].sort((a, b) => ORDER.indexOf(a) - ORDER.indexOf(b));
}
