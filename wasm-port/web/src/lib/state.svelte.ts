// App-wide reactive state (Svelte 5 runes). One module-level store; the
// components read/mutate fields on `app`. Mirrors the `_AppState` dataclass
// in src/gui.py plus the parts we keep purely client-side (no shutdown).

import { CardClass, type CoreSpec, type Position } from "./types";
import type { Deck } from "./deck";
import type { ResolvedConfig, ConfigBundle } from "./config";
import type { OptimizeResult } from "./optimize";
import { CORE_OPTIONS } from "./coreOptions";
import type { CardEntry } from "./modifiers";
import type { AssignmentKey, AssignmentVal } from "./preview";
import {
  emptyStructural, pruneConvertedSlots, canRemoveConstructionTile,
  MAX_CONSTRUCTION, MAX_ARCANE_CONVERT,
  type StructuralCores,
} from "./structural";

export type Tab = "optimize" | "preview";

export interface CoreRowState {
  enabled:  boolean;
  override: number | null;
}

interface AppState {
  // Boot
  bundle: ConfigBundle | null;
  mode: string;
  cfg: ResolvedConfig | null;
  decks: Deck[];
  bootError: string | null;

  // Selection
  deck: Deck | null;
  cardClass: CardClass;

  // Inputs
  inventoryCounts: Record<string, number>;            // stackKey -> count (regular pool)
  // Forced inventory — per-(type, color) lower bound the SA must satisfy.
  // Cap = inventoryCounts + forcedCounts. Empty by default.
  forcedCounts:    Record<string, number>;
  // Which inventory pool the table is currently editing.
  inventoryView:   "regular" | "forced";
  // Arcane auto-place toggle. true = SA must keep arcane slots as ARCANE
  // (color swaps still allowed within them). false = SA may swap arcane slots
  // to DEAD for void-core trade-offs. Initialised from cfg.arcane.auto_place.
  autoPlaceArcane: boolean;
  coreState: CoreRowState[];                          // index aligned with CORE_OPTIONS
  // User-adjustable "Bonus Cores" delta. Defaults to the active mode's
  // `deckmod` (1 in wolds, 0 in vanilla) and is re-seeded on every mode flip.
  // The optimizer uses `max(0, deck.base_core_slots + bonusCores)`, so the
  // value is unbounded — typing a large negative number just clamps the
  // effective core count to 0.
  bonusCores: number;
  nIter: number;
  restarts: number;

  // Run
  running: boolean;
  result: OptimizeResult | null;
  elapsedMs: number | null;
  runError: string | null;

  // Tabs / Preview
  tab: Tab;
  modifiers: Map<string, CardEntry> | null;
  modifiersError: string | null;
  previewAssignments: Map<AssignmentKey, AssignmentVal>;

  // Structural cores (Construction + Arcane Core). Web-only, mode-gated by
  // cfg.cores.{construction_allow, arcane_core_allow}. Their state mutates the
  // deck layout before SA runs — see lib/structural.ts.
  structural: StructuralCores;
}

function initialCoreState(): CoreRowState[] {
  return CORE_OPTIONS.map(() => ({ enabled: false, override: null }));
}

export const app = $state<AppState>({
  bundle: null,
  mode: "wolds",
  cfg: null,
  decks: [],
  bootError: null,

  deck: null,
  cardClass: CardClass.SHINY,

  inventoryCounts: {},
  forcedCounts:    {},
  inventoryView:   "regular",
  autoPlaceArcane: true,   // default; overridden from cfg.arcane.auto_place on boot
  coreState: initialCoreState(),
  bonusCores: 0,           // seeded from cfg.deckmod on boot + mode change
  nIter: 60_000,
  restarts: 12,

  running: false,
  result: null,
  elapsedMs: null,
  runError: null,

  tab: "optimize",
  modifiers: null,
  modifiersError: null,
  previewAssignments: new Map(),

  structural: emptyStructural(),
});

/** Build the user's CoreSpec[] from the picker state. */
export function selectedCores(): CoreSpec[] {
  const out: CoreSpec[] = [];
  for (let i = 0; i < CORE_OPTIONS.length; i++) {
    const s = app.coreState[i];
    if (!s.enabled) continue;
    out.push({
      core_type: CORE_OPTIONS[i].coreType,
      color:     CORE_OPTIONS[i].color,
      override:  s.override,
    });
  }
  return out;
}

/** Apply a flat preset to every inventory cell (Unlimited / Clear buttons).
 *  Operates on whichever pool the table is currently editing. */
export function setAllInventory(value: number, allKeys: string[]): void {
  const target = app.inventoryView === "forced" ? app.forcedCounts : app.inventoryCounts;
  for (const k of allKeys) target[k] = value;
}

/** Fill every (type, `color`) cell with `value` in the active pool. */
export function fillColumn(value: number, columnKeys: string[]): void {
  const target = app.inventoryView === "forced" ? app.forcedCounts : app.inventoryCounts;
  for (const k of columnKeys) target[k] = value;
}

/** Fill every (`cardType`, color) cell with `value` in the active pool. */
export function fillRow(value: number, rowKeys: string[]): void {
  const target = app.inventoryView === "forced" ? app.forcedCounts : app.inventoryCounts;
  for (const k of rowKeys) target[k] = value;
}

/** Toggle every core checkbox at once (Enable-all / Disable-all). */
export function setAllCores(enabled: boolean): void {
  for (const s of app.coreState) s.enabled = enabled;
}

/** Reset run-derived state when the deck / mode / class changes. */
export function clearRunResult(): void {
  app.result    = null;
  app.elapsedMs = null;
  app.runError  = null;
}

/** Drop every preview-mode assignment (deck/class swap, manual clear). */
export function clearPreviewAssignments(): void {
  app.previewAssignments = new Map();
}

// ─── Structural cores (Construction + Arcane Core) ───────────────────────────
//
// Every mutation here clears card placements + run result so the player never
// looks at a stale layout. The cores cost one of the deck's `core_slots` each
// (see structuralCoreCost in lib/structural.ts).

function _resetForStructuralChange(): void {
  clearRunResult();
  clearPreviewAssignments();
}

/** Toggle the Construction Core on/off. Off → drop every placed tile (which in
 *  turn may invalidate arcane conversions, so prune those too). */
export function toggleConstructionCore(on: boolean): void {
  if (app.structural.constructionEnabled === on) return;
  app.structural.constructionEnabled = on;
  if (!on) {
    app.structural.addedSlots = [];
    if (app.deck) {
      app.structural.convertedSlots = pruneConvertedSlots(app.deck, app.structural);
    } else {
      app.structural.convertedSlots = [];
    }
  }
  _resetForStructuralChange();
}

/** Toggle the Arcane Core on/off. Off → drop every conversion. */
export function toggleArcaneCore(on: boolean): void {
  if (app.structural.arcaneCoreEnabled === on) return;
  app.structural.arcaneCoreEnabled = on;
  if (!on) app.structural.convertedSlots = [];
  _resetForStructuralChange();
}

/** Add a construction tile. Caller must have verified `pos` is in
 *  constructionCandidates(); we still cap at MAX_CONSTRUCTION here. */
export function addConstructionSlot(pos: Position): void {
  if (!app.structural.constructionEnabled) return;
  if (app.structural.addedSlots.length >= MAX_CONSTRUCTION) return;
  if (app.structural.addedSlots.some((p) => p[0] === pos[0] && p[1] === pos[1])) return;
  app.structural.addedSlots = [...app.structural.addedSlots, pos];
  _resetForStructuralChange();
}

/** Remove a construction tile if doing so keeps the addition graph connected
 *  (canRemoveConstructionTile). Returns true on success. */
export function removeConstructionSlot(pos: Position): boolean {
  if (!app.deck) return false;
  if (!canRemoveConstructionTile(pos, app.deck, app.structural)) return false;
  app.structural.addedSlots = app.structural.addedSlots.filter(
    (p) => !(p[0] === pos[0] && p[1] === pos[1]),
  );
  // The removed tile may have been converted; prune.
  app.structural.convertedSlots = pruneConvertedSlots(app.deck, app.structural);
  _resetForStructuralChange();
  return true;
}

/** Convert a regular slot to arcane. Caller verifies the position is a current
 *  slot and isn't already arcane (native or converted). Cap at MAX_ARCANE_CONVERT. */
export function convertSlotToArcane(pos: Position): void {
  if (!app.structural.arcaneCoreEnabled) return;
  if (app.structural.convertedSlots.length >= MAX_ARCANE_CONVERT) return;
  if (app.structural.convertedSlots.some((p) => p[0] === pos[0] && p[1] === pos[1])) return;
  app.structural.convertedSlots = [...app.structural.convertedSlots, pos];
  _resetForStructuralChange();
}

/** Revert an arcane conversion (right-click on a converted slot). */
export function unconvertArcaneSlot(pos: Position): void {
  const before = app.structural.convertedSlots.length;
  app.structural.convertedSlots = app.structural.convertedSlots.filter(
    (p) => !(p[0] === pos[0] && p[1] === pos[1]),
  );
  if (app.structural.convertedSlots.length !== before) _resetForStructuralChange();
}

/** Mode/deck flip: zero out the structural cores (allow flags may have changed
 *  and the previous deck's coords don't carry over). Caller handles the rest of
 *  the reset (run result + preview). */
export function resetStructural(): void {
  app.structural = emptyStructural();
}
