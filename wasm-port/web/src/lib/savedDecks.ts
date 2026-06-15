// localStorage CRUD for user-built decks.
//
// Saved decks live entirely in the browser; nothing is uploaded anywhere.
// The Build tab is the only surface that loads them — the Optimize tab's deck
// dropdown stays "real decks only" so users don't mix their experiments up
// with the modpack's roster.
//
// Storage shape: a single key (`STORAGE_KEY`) holds a JSON array of
// SavedDeck records. We rewrite the whole array on every CRUD call — total
// payload should stay tiny (a few dozen layouts at most), so the cost is
// trivial.

import type { Position } from "./types";
import type { BuilderState } from "./builder";

const STORAGE_KEY = "wvdo.builder.savedDecks.v1";

export interface SavedDeck {
  /** Stable id — derived from name at save time, but immutable per record.
   *  Renaming a saved deck keeps the same id; collisions get a `_2`, `_3`, … */
  key:          string;
  name:         string;
  coreCount:    number;
  regularSlots: Position[];
  arcaneSlots:  Position[];
  /** epoch-ms */
  lastModified: number;
}

/** Read every saved deck. Returns [] on any parse failure — we don't want a
 *  corrupted entry to brick the Build tab. Errors are logged for diagnosis. */
export function loadAllSaved(): SavedDeck[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return [];
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) {
      console.warn("[savedDecks] storage payload is not an array, ignoring");
      return [];
    }
    return arr;
  } catch (e) {
    console.error("[savedDecks] failed to parse storage payload — using empty list", e);
    return [];
  }
}

/** Overwrite the entire saved-decks list. */
function writeAll(list: SavedDeck[]): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(list));
  } catch (e) {
    // Quota errors land here. Log so the user can see what happened in devtools
    // — and surface to the caller so the UI can show an error toast.
    console.error("[savedDecks] failed to write to localStorage", e);
    throw e;
  }
}

/** Find by key, or null. */
export function loadByKey(key: string): SavedDeck | null {
  return loadAllSaved().find((d) => d.key === key) ?? null;
}

/** Insert-or-update by key. If `wantKey` collides with an existing entry that
 *  isn't the same record (caller hasn't provided `existingKey`), suffix `_2`,
 *  `_3` until unique. Returns the resolved key. */
export function saveDeck(b: BuilderState, wantKey: string, existingKey: string | null): string {
  const list = loadAllSaved();
  // If we're replacing a known record, allow keeping the same key even when
  // another record (somehow) collides with the same key — the existing record
  // is the one we're overwriting.
  let resolvedKey = wantKey;
  if (existingKey !== wantKey) {
    const taken = new Set(list.map((d) => d.key));
    let n = 2;
    while (taken.has(resolvedKey)) {
      resolvedKey = `${wantKey}_${n}`;
      n++;
    }
  }
  const record: SavedDeck = {
    key:          resolvedKey,
    name:         b.name || "Untitled",
    coreCount:    b.coreCount,
    regularSlots: b.regularSlots,
    arcaneSlots:  b.arcaneSlots,
    lastModified: Date.now(),
  };
  const idx = existingKey !== null
    ? list.findIndex((d) => d.key === existingKey)
    : -1;
  if (idx >= 0) list[idx] = record;
  else          list.push(record);
  writeAll(list);
  return resolvedKey;
}

export function deleteDeck(key: string): void {
  const list = loadAllSaved().filter((d) => d.key !== key);
  writeAll(list);
}
