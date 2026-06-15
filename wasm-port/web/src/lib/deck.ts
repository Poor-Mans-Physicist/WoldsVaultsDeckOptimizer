// Deck geometry — port of src/config.py's Deck class.
// Loads from the build-time-generated `public/decks.json` and precomputes
// the row / col / surr / diag peer sets (same shape as the Rust kernel's
// DeckGeom).
//
// Schema change vs the original PR:
//   * Per-mode roster: decks.json is shaped `{wolds: [Deck], vanilla: [Deck]}`
//     (the build script reads each mode's `decks.json_file` and loads only
//     that JSON file plus any mode-agnostic YAML decks).
//   * Arcane: deck carries `arcane_slots: Position[]` (list of positions),
//     not `n_arcane: int`. The web grid uses these to render purple borders,
//     and `arcaneSlotIndices` is derived for the SA payload.

import type { Position } from "./types";

export interface RawDeck {
  key:             string;
  name:            string;
  slots:           [number, number][];   // JSON: array of [row, col] pairs (regular ∪ arcane)
  arcane_slots:    [number, number][];   // subset of `slots` that hold the ARCANE/DEAD-only rule
  base_core_slots: number;               // add ``modes.<mode>.deckmod`` for final
  min_regular:     number;
  max_greed:       number;
}

export interface Deck {
  key:        string;
  name:       string;
  slots:      Position[];                // ordering is canonical for the Rust call
  base_core_slots: number;               // raw deck-card value, before deckmod / Bonus Cores
  core_slots: number;                    // base + mode's deckmod (initial display value;
                                         // overridden at run-time by app.bonusCores)
  arcaneSlots:         Position[];        // for rendering the purple border
  arcaneSlotIndices:   number[];          // indices into `slots`, for the SA payload
  min_regular: number;
  max_greed:   number;

  // Peer-index arrays (indices into `slots`), parallel to Rust `DeckGeom`.
  rowPeers:  number[][];
  colPeers:  number[][];
  surrPeers: number[][];
  diagPeers: number[][];
}

function posKey(r: number, c: number): string {
  return `${r},${c}`;
}

/** Build a Deck (with precomputed peers) from a RawDeck and the active mode's deckmod. */
export function buildDeck(raw: RawDeck, deckmod: number): Deck {
  const slots: Position[] = raw.slots.map(([r, c]) => [r, c] as const);
  const n = slots.length;
  const index = new Map<string, number>();
  for (let i = 0; i < n; i++) {
    index.set(posKey(slots[i][0], slots[i][1]), i);
  }

  // Arcane subset → typed positions + indices into `slots` for the SA payload.
  const arcaneSlots: Position[] = (raw.arcane_slots ?? []).map(
    ([r, c]) => [r, c] as const,
  );
  const arcaneSlotIndices: number[] = [];
  for (const [r, c] of arcaneSlots) {
    const idx = index.get(posKey(r, c));
    if (idx !== undefined) arcaneSlotIndices.push(idx);
  }

  const rowPeers:  number[][] = [];
  const colPeers:  number[][] = [];
  const surrPeers: number[][] = [];
  const diagPeers: number[][] = [];

  for (let i = 0; i < n; i++) {
    const [r, c] = slots[i];
    const row: number[] = [], col: number[] = [], sur: number[] = [], dia: number[] = [];
    for (let j = 0; j < n; j++) {
      if (j === i) continue;
      const [qr, qc] = slots[j];
      if (qr === r) row.push(j);
      if (qc === c) col.push(j);
      if (Math.max(Math.abs(qr - r), Math.abs(qc - c)) <= 1) sur.push(j);
      // Same diagonal as Python: NW-SE  (q.r - q.c == r - c)  OR  NE-SW (q.r + q.c == r + c)
      if (qr - qc === r - c || qr + qc === r + c) dia.push(j);
    }
    rowPeers.push(row);
    colPeers.push(col);
    surrPeers.push(sur);
    diagPeers.push(dia);
  }

  return {
    key:               raw.key,
    name:              raw.name,
    slots,
    base_core_slots:   raw.base_core_slots,
    core_slots:        raw.base_core_slots + deckmod,
    arcaneSlots,
    arcaneSlotIndices,
    min_regular:       raw.min_regular,
    max_greed:         raw.max_greed,
    rowPeers, colPeers, surrPeers, diagPeers,
  };
}

/**
 * Fetch the build-time deck bundle and build Deck objects for the active mode.
 *
 * Shape change vs the original PR: `decks.json` is now per-mode-keyed
 * (`{wolds: [...], vanilla: [...]}`). We fetch once and index by `mode`.
 * Caches the parsed JSON so mode switches don't re-fetch.
 */
let _decksCache: Record<string, RawDeck[]> | null = null;

export async function loadDecks(
  baseUrl: string,
  deckmod: number,
  mode:    string,
): Promise<Deck[]> {
  if (!_decksCache) {
    // `no-cache` for the same reason as config.json — revalidate against
    // the server so new decks / arcane-slot changes aren't served stale.
    const res = await fetch(`${baseUrl}decks.json`, { cache: "no-cache" });
    if (!res.ok) throw new Error(`Failed to load decks.json: ${res.status}`);
    _decksCache = await res.json();
  }
  const list = _decksCache?.[mode];
  if (!list) {
    throw new Error(
      `Mode '${mode}' missing from decks.json (have: ${Object.keys(_decksCache ?? {}).join(", ")})`,
    );
  }
  // Alphabetize by display name (case-insensitive). The build-time JSON
  // preserves insertion order — sorting here means the dropdown order is
  // independent of how the YAML/JSON sources happen to be laid out.
  return list
    .map((r) => buildDeck(r, deckmod))
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}
