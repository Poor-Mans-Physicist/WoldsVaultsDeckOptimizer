// "Build your own deck" — types, key derivation, layout↔grid helpers, and the
// modpack JSON serializer.
//
// The Builder produces a `Deck` object that flows through the same SA pipeline
// as the JSON-imported decks. There is no new scoring math anywhere — this is
// purely a UX surface for designing layouts on a 9×6 canvas and exporting an
// entry that can be dropped into Vault Hunters' modpack data.

import type { Position } from "./types";
import type { Deck, RawDeck } from "./deck";
import { buildDeck } from "./deck";
import { MAX_GRID_WIDTH, MAX_GRID_HEIGHT } from "./structural";

export const BUILDER_GRID_WIDTH  = MAX_GRID_WIDTH;
export const BUILDER_GRID_HEIGHT = MAX_GRID_HEIGHT;

/** Defaults baked into every exported deck. Confirmed safe values by the user
 *  — these are what the modpack expects for a typical hand-curated deck. */
export const DEFAULT_ESSENCE = 5;
export const DEFAULT_WEIGHT  = 1.0;

export type BuilderTool = "regular" | "arcane" | "erase";

/** Builder runtime state. Lives under `app.builder` (state.svelte.ts). The
 *  canvas uses (row, col) positions; `regularSlots` and `arcaneSlots` are
 *  disjoint sets — flipping a tile from regular to arcane removes it from the
 *  first list and inserts it into the second. */
export interface BuilderState {
  name:        string;
  coreCount:   number;
  regularSlots: Position[];   // `O` cells
  arcaneSlots:  Position[];   // `A` cells
  tool:        BuilderTool;
  /** dirty = true means there are unsaved changes since the last save/load. */
  dirty:       boolean;
  /** Storage key of the saved deck currently loaded for editing (null = new). */
  loadedKey:   string | null;
}

export function emptyBuilder(): BuilderState {
  return {
    name:        "",
    coreCount:   3,
    regularSlots: [],
    arcaneSlots:  [],
    tool:        "regular",
    dirty:       false,
    loadedKey:   null,
  };
}

const samePos = (a: Position, b: Position) => a[0] === b[0] && a[1] === b[1];

/** Derive the modpack deck key from a display name. Matches the dedup rule in
 *  decks/README.md (lowercase, non-alphanumerics → underscores, strip leading
 *  digits / underscores). Empty input → `"untitled"` so JSON / storage always
 *  have a usable key. */
export function deriveKey(name: string): string {
  const k = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .replace(/^[0-9]+_?/, "");
  return k.length === 0 ? "untitled" : k;
}

/** All placed positions (regular ∪ arcane). */
export function allBuilderSlots(b: BuilderState): Position[] {
  return [...b.regularSlots, ...b.arcaneSlots];
}

/** Builder slots → a Deck the SA can score. Re-uses the same `buildDeck` peer
 *  precomputation as JSON/YAML-loaded decks so there's no special-casing
 *  downstream. */
export function builderToDeck(b: BuilderState, deckmod: number): Deck {
  const raw: RawDeck = {
    key:             "builder",
    name:            b.name || "Untitled",
    slots:           allBuilderSlots(b).map(([r, c]) => [r, c] as [number, number]),
    arcane_slots:    b.arcaneSlots.map(([r, c]) => [r, c] as [number, number]),
    base_core_slots: b.coreCount,
    min_regular:     -1,
    max_greed:       -1,
  };
  return buildDeck(raw, deckmod);
}

/** Render the builder's slot set as the modpack's layout strings. The output
 *  rectangle is the slot bbox — never larger than 9×6 (enforced upstream by
 *  the canvas geometry). */
export function renderLayoutStrings(b: BuilderState): string[] {
  const slots = allBuilderSlots(b);
  if (slots.length === 0) return [];
  let minR = Infinity, maxR = -Infinity, minC = Infinity, maxC = -Infinity;
  for (const [r, c] of slots) {
    if (r < minR) minR = r;
    if (r > maxR) maxR = r;
    if (c < minC) minC = c;
    if (c > maxC) maxC = c;
  }
  const arcaneSet = new Set(b.arcaneSlots.map(([r, c]) => `${r},${c}`));
  const regSet    = new Set(b.regularSlots.map(([r, c]) => `${r},${c}`));
  const rows: string[] = [];
  for (let r = minR; r <= maxR; r++) {
    let row = "";
    for (let c = minC; c <= maxC; c++) {
      const k = `${r},${c}`;
      if      (arcaneSet.has(k)) row += "A";
      else if (regSet.has(k))    row += "O";
      else                       row += "X";
    }
    rows.push(row);
  }
  return rows;
}

/** Build the modpack JSON entry for the current builder state. Returns the
 *  `{<key>: <value>}` pair — caller decides whether to embed in `values:` or
 *  emit as a single chunk. */
export interface ModpackEntry {
  model:       string;
  name:        string;
  essence:     { min: number; max: number };
  layout:      Array<{ value: string[]; weight: number }>;
  socketCount: { min: number; max: number };
}

export function buildModpackEntry(b: BuilderState): { key: string; entry: ModpackEntry } {
  const key = deriveKey(b.name);
  return {
    key,
    entry: {
      model:   `woldsvaults:deck/${key}#inventory`,
      name:    b.name || "Untitled",
      essence: { min: DEFAULT_ESSENCE, max: DEFAULT_ESSENCE },
      layout:  [{ value: renderLayoutStrings(b), weight: DEFAULT_WEIGHT }],
      socketCount: { min: b.coreCount, max: b.coreCount },
    },
  };
}

/** Render the modpack entry as a pretty-printed JSON snippet ready to drop
 *  into the `values:` object of a Wold's deck-data file. Trailing newline so
 *  the snippet round-trips cleanly through a textarea. */
export function buildModpackJson(b: BuilderState): string {
  const { key, entry } = buildModpackEntry(b);
  // Two-space indent matches the existing wolds_decks.json style.
  const payload = { [key]: entry };
  // JSON.stringify produces compact arrays — fine for our shape (the layout
  // string rows show one per line via the array bracket itself).
  return JSON.stringify(payload, null, 2) + "\n";
}

/** Position helpers used by the canvas click handlers. */
export function isRegular(b: BuilderState, p: Position): boolean {
  return b.regularSlots.some((q) => samePos(q, p));
}
export function isArcane(b: BuilderState, p: Position): boolean {
  return b.arcaneSlots.some((q) => samePos(q, p));
}
