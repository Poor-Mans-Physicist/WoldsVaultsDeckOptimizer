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
  maxConstruction, maxArcaneConvert,
  type StructuralCores,
} from "./structural";
import {
  emptyBuilder, deriveKey, type BuilderState, type BuilderTool,
} from "./builder";
import {
  loadAllSaved, loadByKey, saveDeck as storageSaveDeck, deleteDeck as storageDeleteDeck,
  type SavedDeck,
} from "./savedDecks";
import {
  loadAllSnapshots, persistSnapshot, deleteSnapshot as storageDeleteSnapshot,
  makeSnapshotId,
  type Snapshot,
} from "./snapshots";
import { buildDeck } from "./deck";
import type { RawDeck } from "./deck";
import { CardType, type Placed } from "./types";
import { simulateInventoryBreakdown } from "./breakdown";

export type Tab = "optimize" | "preview" | "build" | "snapshots";

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

  // "Build your own deck" tab state. The canvas + name + core count + tool
  // mode all live here; saved decks live in localStorage via lib/savedDecks.ts
  // and are loaded on demand into `builder`. See lib/builder.ts.
  builder: BuilderState;
  /** Cached list of saved decks for the Builder sidebar — refreshed on
   *  load / save / delete via reloadSavedDecks(). */
  savedDecks: SavedDeck[];

  /** Cached list of optimization snapshots — refreshed on save/delete. The
   *  Snapshots tab reads this; lib/snapshots.ts owns the storage. */
  snapshots: Snapshot[];
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

  builder:    emptyBuilder(),
  savedDecks: [],
  snapshots:  [],
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
 *  constructionCandidates(); we still cap at the effective max here. */
export function addConstructionSlot(pos: Position): void {
  if (!app.structural.constructionEnabled) return;
  if (app.structural.addedSlots.length >= maxConstruction(app.structural)) return;
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
 *  slot and isn't already arcane (native or converted). Cap at the effective
 *  max (3 normally, 5 with Greater). */
export function convertSlotToArcane(pos: Position): void {
  if (!app.structural.arcaneCoreEnabled) return;
  if (app.structural.convertedSlots.length >= maxArcaneConvert(app.structural)) return;
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

/** Toggle the "Greater" structural-cores variant (experimental community
 *  cap, 5 instead of 3). Switching off doesn't auto-prune existing
 *  overflow tiles — the new cap takes effect only for further additions /
 *  conversions, and the counter floors at 0 when the user is over-cap. */
export function toggleGreaterStructural(on: boolean): void {
  if (app.structural.greaterStructural === on) return;
  app.structural.greaterStructural = on;
  // No card-clear needed: a tile count change doesn't invalidate a prior run.
}

// ─── Builder (Build your own deck) ───────────────────────────────────────────
//
// Every mutation flips `app.builder.dirty = true` and tears down the SA result
// (the previous run was for the old layout). Saving / loading / starting a new
// deck resets dirty back to false. The Build tab guards against tab-switch and
// browser-close with this flag.

const samePos = (a: Position, b: Position) => a[0] === b[0] && a[1] === b[1];

function _markBuilderDirty(): void {
  app.builder.dirty = true;
  clearRunResult();
}

/** Place a regular `O` tile at `pos`. If the tile is already arcane, this
 *  silently no-ops (the user must Erase first). Caller verifies the position
 *  is inside the 9×6 canvas. */
export function builderPlaceRegular(pos: Position): void {
  if (app.builder.regularSlots.some((p) => samePos(p, pos))) return;
  if (app.builder.arcaneSlots.some((p) => samePos(p, pos)))  return;
  app.builder.regularSlots = [...app.builder.regularSlots, pos];
  _markBuilderDirty();
}

/** Place an arcane `A` tile at `pos`. Same no-op rule as above. */
export function builderPlaceArcane(pos: Position): void {
  if (app.builder.regularSlots.some((p) => samePos(p, pos))) return;
  if (app.builder.arcaneSlots.some((p) => samePos(p, pos)))  return;
  app.builder.arcaneSlots = [...app.builder.arcaneSlots, pos];
  _markBuilderDirty();
}

/** Remove a tile of either type at `pos`. */
export function builderEraseTile(pos: Position): void {
  const before =
    app.builder.regularSlots.length + app.builder.arcaneSlots.length;
  app.builder.regularSlots = app.builder.regularSlots.filter((p) => !samePos(p, pos));
  app.builder.arcaneSlots  = app.builder.arcaneSlots .filter((p) => !samePos(p, pos));
  const after =
    app.builder.regularSlots.length + app.builder.arcaneSlots.length;
  if (after !== before) _markBuilderDirty();
}

/** Dispatch a canvas left-click to the currently selected tool. */
export function builderCanvasClick(pos: Position): void {
  switch (app.builder.tool) {
    case "regular": builderPlaceRegular(pos); break;
    case "arcane":  builderPlaceArcane(pos);  break;
    case "erase":   builderEraseTile(pos);    break;
  }
}

/** Right-click anywhere on the canvas erases (parallel with the structural
 *  cores' right-click-to-revert convention). */
export function builderCanvasContextClick(pos: Position): void {
  builderEraseTile(pos);
}

/** Switch the canvas tool. Pure UI state — no dirty flip. */
export function builderSetTool(tool: BuilderTool): void {
  app.builder.tool = tool;
}

/** Rename the in-progress deck. Marks dirty so the user gets the save prompt. */
export function builderSetName(name: string): void {
  if (app.builder.name === name) return;
  app.builder.name = name;
  _markBuilderDirty();
}

/** Adjust the core-slot count for the in-progress deck. */
export function builderSetCoreCount(n: number): void {
  const safe = Number.isFinite(n) ? Math.max(0, Math.floor(n)) : 0;
  if (app.builder.coreCount === safe) return;
  app.builder.coreCount = safe;
  _markBuilderDirty();
}

/** Start from scratch. Caller has confirmed unsaved-changes flow first. */
export function builderNew(): void {
  app.builder = emptyBuilder();
  clearRunResult();
}

/** Refresh the cached saved-decks list from localStorage. */
export function reloadSavedDecks(): void {
  app.savedDecks = loadAllSaved();
}

/** Replace the builder state with the saved record at `key`. No-op if the key
 *  is missing — caller should refresh the list first. */
export function loadSavedDeck(key: string): void {
  const rec = loadByKey(key);
  if (rec === null) return;
  app.builder = {
    name:        rec.name,
    coreCount:   rec.coreCount,
    regularSlots: rec.regularSlots,
    arcaneSlots:  rec.arcaneSlots,
    tool:        "regular",
    dirty:       false,
    loadedKey:   rec.key,
  };
  clearRunResult();
}

/** Persist the builder state. If `saveAs` is true the existing-key is dropped
 *  so a colliding name spawns a new record. Returns the resolved key. */
export function saveBuilderDeck(saveAs: boolean = false): string {
  const wantKey = deriveKey(app.builder.name);
  const existing = saveAs ? null : app.builder.loadedKey;
  const resolvedKey = storageSaveDeck(app.builder, wantKey, existing);
  app.builder.loadedKey = resolvedKey;
  app.builder.dirty = false;
  reloadSavedDecks();
  return resolvedKey;
}

/** Delete a saved deck. If the deleted entry is the one currently loaded in
 *  the builder, the in-progress state stays but `loadedKey` is cleared. */
export function deleteSavedDeck(key: string): void {
  storageDeleteDeck(key);
  if (app.builder.loadedKey === key) app.builder.loadedKey = null;
  reloadSavedDecks();
}

// ─── Snapshots ───────────────────────────────────────────────────────────────
//
// A snapshot is a self-contained capture of one Run: the deck layout, every
// input that fed the SA, and the SA's output (assignment + NDM). It embeds the
// deck so renames or removals in the modpack roster never orphan a record. It
// also locks to the mode it was taken in — cross-mode loads are refused
// (loadSnapshot() switches mode silently when needed; the call site is
// expected to confirm if doing so loses other work).

export function reloadSnapshots(): void {
  app.snapshots = loadAllSnapshots();
}

/** Capture the live `app` state (post-run) into a Snapshot record. Returns
 *  null when prerequisites aren't met (no result, no cfg, no deck). */
export function captureSnapshot(label: string): Snapshot | null {
  if (!app.cfg || !app.deck || !app.result) return null;
  // The deck we want to embed is whatever the SA actually scored — that's the
  // structural-cores-mutated deck (additions baked in). Recover it by
  // resolving deck.slots at the call site; we re-snapshot the current deck's
  // raw geometry (slots + arcaneSlots) which is the post-structural shape.
  const deckSrc = app.deck;
  const snap: Snapshot = {
    id:        makeSnapshotId(),
    label,
    createdAt: Date.now(),
    mode:      app.mode,
    deck: {
      isBuiltDeck:     app.tab === "build",
      key:             deckSrc.key,
      name:            deckSrc.name,
      slots:           deckSrc.slots.map(([r, c]) => [r, c]),
      arcaneSlots:     deckSrc.arcaneSlots.map(([r, c]) => [r, c]),
      base_core_slots: deckSrc.base_core_slots,
      min_regular:     deckSrc.min_regular,
      max_greed:       deckSrc.max_greed,
    },
    cardClass:       app.cardClass,
    bonusCores:      app.bonusCores,
    autoPlaceArcane: app.autoPlaceArcane,
    inventoryCounts: { ...app.inventoryCounts },
    forcedCounts:    { ...app.forcedCounts },
    cores:           app.result.coresUsed.map((c) => ({ ...c })),
    structural: {
      constructionEnabled: app.structural.constructionEnabled,
      arcaneCoreEnabled:   app.structural.arcaneCoreEnabled,
      addedSlots:          app.structural.addedSlots.map(([r, c]) => [r, c] as Position),
      convertedSlots:      app.structural.convertedSlots.map(([r, c]) => [r, c] as Position),
      greaterStructural:   app.structural.greaterStructural,
    },
    assignment: _serializeAssignment(deckSrc.slots, app.result.assignment),
    wasmScore:  app.result.wasmScore,
  };
  return snap;
}

/** Walk deck.slots in canonical order and pull (type, color) per slot from the
 *  result's Map. Matches SliceResult.assignment so we re-use the existing
 *  finalize path on restore. */
function _serializeAssignment(slots: readonly Position[], asgn: Map<string, Placed>): [string, string][] {
  const out: [string, string][] = [];
  for (const [r, c] of slots) {
    const p = asgn.get(`${r},${c}`);
    if (p === undefined) {
      // Should not happen — every slot is assigned by the SA. Fill with EMPTY
      // so the array stays parallel.
      out.push([CardType.EMPTY, ""]);
    } else {
      out.push([p[0], p[1] ?? ""]);
    }
  }
  return out;
}

/** Persist a captured snapshot. Updates the cached list. */
export function saveSnapshot(snap: Snapshot): void {
  persistSnapshot(snap);
  reloadSnapshots();
}

export function deleteSnapshotById(id: string): void {
  storageDeleteSnapshot(id);
  reloadSnapshots();
}

/** Reverse the capture — slot a snapshot back into `app.*` and reconstruct an
 *  `OptimizeResult` so the deck grid + breakdown popup paint exactly what was
 *  captured. Caller must have already switched `app.mode` + `app.cfg` to the
 *  snapshot's mode and resolved the new config bundle. */
export function restoreSnapshot(snap: Snapshot): void {
  if (!app.cfg) return;

  // Rebuild the Deck via the same constructor JSON/YAML loads use, so peer
  // sets are computed fresh.
  const raw: RawDeck = {
    key:             snap.deck.key,
    name:            snap.deck.name,
    slots:           snap.deck.slots,
    arcane_slots:    snap.deck.arcaneSlots,
    base_core_slots: snap.deck.base_core_slots,
    min_regular:     snap.deck.min_regular,
    max_greed:       snap.deck.max_greed,
  };
  const deck = buildDeck(raw, app.cfg.deckmod);

  // Inputs.
  app.deck            = deck;
  app.cardClass       = snap.cardClass;
  app.bonusCores      = snap.bonusCores;
  app.autoPlaceArcane = snap.autoPlaceArcane;
  app.inventoryCounts = { ...snap.inventoryCounts };
  app.forcedCounts    = { ...snap.forcedCounts };
  // Structural cores — same shape, just clone so reactive proxies don't share.
  // `greaterStructural` was added later; older snapshots default to false.
  app.structural = {
    constructionEnabled: snap.structural.constructionEnabled,
    arcaneCoreEnabled:   snap.structural.arcaneCoreEnabled,
    addedSlots:          snap.structural.addedSlots.map(([r, c]) => [r, c] as Position),
    convertedSlots:      snap.structural.convertedSlots.map(([r, c]) => [r, c] as Position),
    greaterStructural:   snap.structural.greaterStructural ?? false,
  };

  // Update the CorePicker checkboxes from `snap.cores` — match by (type, color)
  // and stash the override, ignoring static-vs-variable differences.
  const wanted = new Map<string, { override: number | null }>();
  for (const c of snap.cores) {
    const k = `${c.core_type}|${c.color ?? ""}`;
    wanted.set(k, { override: c.override });
  }
  for (let i = 0; i < CORE_OPTIONS.length; i++) {
    const opt = CORE_OPTIONS[i];
    const k = `${opt.coreType}|${opt.color ?? ""}`;
    const w = wanted.get(k);
    if (w) {
      app.coreState[i].enabled  = true;
      app.coreState[i].override = w.override;
    } else {
      app.coreState[i].enabled  = false;
      app.coreState[i].override = null;
    }
  }

  // Rebuild the OptimizeResult from the serialized assignment so the grid
  // paints + the breakdown popup works on click.
  const asgnMap = new Map<string, Placed>();
  for (let i = 0; i < deck.slots.length; i++) {
    const [tStr, cStr] = snap.assignment[i] ?? [CardType.EMPTY, ""];
    asgnMap.set(
      `${deck.slots[i][0]},${deck.slots[i][1]}`,
      [tStr as any, (cStr ? cStr : null) as any],
    );
  }
  const breakdown = simulateInventoryBreakdown(deck, asgnMap, snap.cardClass, snap.cores, app.cfg);
  app.result = {
    assignment: asgnMap,
    wasmScore:  snap.wasmScore,
    tsScore:    breakdown.total,
    coresUsed:  snap.cores.map((c) => ({ ...c })),
    breakdown,
  };
  app.elapsedMs = null;
  app.runError  = null;
  clearPreviewAssignments();
}
