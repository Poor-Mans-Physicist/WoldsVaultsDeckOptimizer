// Optimization snapshots — full self-contained captures of a Run's inputs +
// output. Persists to localStorage; nothing leaves the browser.
//
// Each snapshot embeds the deck layout (rather than referencing a roster key)
// so renames or removals of modpack decks never orphan an old snapshot. The
// snapshot also locks to the mode it was taken in — cross-mode loads are
// refused at the call site (loadSnapshotInto() takes a same-mode check).
//
// Storage layout: a single key (STORAGE_KEY) holds a JSON array of Snapshot
// records. Same pattern as savedDecks.ts; the array stays small (a few dozen
// records at most) so we rewrite the whole thing on every CRUD call.

import type { Position } from "./types";
import type { CardClass, CoreSpec } from "./types";
import type { StructuralCores } from "./structural";
import { emptyStructural } from "./structural";

const STORAGE_KEY = "wvdo.snapshots.v1";

/** Embedded deck shape — everything `buildDeck()` needs to rebuild peers etc.
 *  Stored as `[number, number]` tuples rather than `Position` (readonly) so
 *  JSON round-trips cleanly. */
export interface SnapshotDeck {
  isBuiltDeck:     boolean;
  key:             string;
  name:            string;
  slots:           [number, number][];
  arcaneSlots:     [number, number][];
  base_core_slots: number;
  min_regular:     number;
  max_greed:       number;
}

export interface Snapshot {
  id:        string;
  label:     string;
  createdAt: number;
  mode:      string;          // mode-locked — loading a wolds snapshot in
                              // vanilla (or vice versa) is refused upstream.

  deck:      SnapshotDeck;

  // — Inputs the user had set when Run fired —
  cardClass:       CardClass;
  bonusCores:      number;
  autoPlaceArcane: boolean;
  inventoryCounts: Record<string, number>;
  forcedCounts:    Record<string, number>;
  cores:           CoreSpec[];
  structural:      StructuralCores;

  // — Output the SA produced —
  assignment: [string, string][];   // parallel to deck.slots; same shape as
                                    // SliceResult.assignment
  wasmScore:  number;
}

// ─── localStorage CRUD ───────────────────────────────────────────────────────

export function loadAllSnapshots(): Snapshot[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return [];
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) {
      console.warn("[snapshots] storage payload is not an array, ignoring");
      return [];
    }
    return arr.map(_normalize);
  } catch (e) {
    console.error("[snapshots] failed to parse storage payload — using empty list", e);
    return [];
  }
}

/** Pre-empt the case where an older record is missing fields added later — pad
 *  with safe defaults rather than crashing the whole list on load. */
function _normalize(rec: any): Snapshot {
  return {
    id:        String(rec?.id ?? `snap_${Date.now()}_${Math.floor(Math.random() * 1e6)}`),
    label:     String(rec?.label ?? "(unnamed)"),
    createdAt: Number(rec?.createdAt ?? Date.now()),
    mode:      String(rec?.mode ?? "wolds"),
    deck:      rec?.deck ?? {
      isBuiltDeck: false, key: "", name: "",
      slots: [], arcaneSlots: [],
      base_core_slots: 0, min_regular: -1, max_greed: -1,
    },
    cardClass:       rec?.cardClass ?? "shiny",
    bonusCores:      Number(rec?.bonusCores ?? 0),
    autoPlaceArcane: Boolean(rec?.autoPlaceArcane ?? true),
    inventoryCounts: rec?.inventoryCounts ?? {},
    forcedCounts:    rec?.forcedCounts ?? {},
    cores:           Array.isArray(rec?.cores) ? rec.cores : [],
    structural:      rec?.structural ?? emptyStructural(),
    assignment:      Array.isArray(rec?.assignment) ? rec.assignment : [],
    wasmScore:       Number(rec?.wasmScore ?? 0),
  };
}

function writeAll(list: Snapshot[]): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(list));
  } catch (e) {
    console.error("[snapshots] failed to write to localStorage", e);
    throw e;
  }
}

export function persistSnapshot(snap: Snapshot): void {
  const list = loadAllSnapshots();
  const idx  = list.findIndex((s) => s.id === snap.id);
  if (idx >= 0) list[idx] = snap;
  else          list.unshift(snap);   // newest first
  writeAll(list);
}

export function deleteSnapshot(id: string): void {
  writeAll(loadAllSnapshots().filter((s) => s.id !== id));
}

export function makeSnapshotId(): string {
  return `snap_${Date.now()}_${Math.floor(Math.random() * 1e6)}`;
}

/** Human-readable auto label: "2026-06-15 14:23 · The Queen Deck (SHINY)". */
export function defaultLabel(deckName: string, cardClass: CardClass): string {
  const d  = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const stamp = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
                `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  return `${stamp} · ${deckName} (${cardClass.toUpperCase()})`;
}

/** Position[]'s helper (used by call sites that want to walk slots in canonical
 *  order — same convention as deck.slots). */
export function snapshotSlotPositions(s: Snapshot): Position[] {
  return s.deck.slots.map(([r, c]) => [r, c] as Position);
}
