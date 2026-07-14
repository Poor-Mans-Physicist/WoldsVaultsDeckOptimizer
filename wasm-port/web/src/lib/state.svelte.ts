// App-wide reactive state (Svelte 5 runes). One module-level store; the
// components read/mutate fields on `app`. Mirrors the `_AppState` dataclass
// in src/gui.py plus the parts we keep purely client-side (no shutdown).

import {
  CardClass, Depth, OptimizerMode,
  type CoreSpec, type ExactStack, type GroupTag, type Position,
  type TagRuleRow, type TaggedPlaced,
} from "./types";
import type { Deck } from "./deck";
import { implicitCatalog } from "./deck";
import type { ResolvedConfig, ConfigBundle } from "./config";
import type { TaggedOptimizeResult } from "./taggedClient";
import { defaultTargetedRules } from "./tagged";
import { toPayload, isScoringImplicit, type ImplicitPayload } from "./implicits";
import {
  simulateTaggedBreakdown,
} from "./taggedBreakdown";
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

  // ── Optimizer 2.0 inputs ──────────────────────────────────────────────
  // Mode slider: Max (theoretical ceiling) / Targeted (tag limits) / Exact
  // (real built cards). Each swaps the side panel + kernel supply model.
  optMode: OptimizerMode;
  // Depth slider → fixed SA params (DEPTH_PARAMS). Replaces raw inputs.
  depth: Depth;
  // Complex Cards (§7): scale_color may differ from card_color. Slows runs.
  complexCards: boolean;
  // Targeted per-tag Min/Max rows (canonical order; null = unbounded).
  targetedRules: TagRuleRow[];
  // Exact-mode inventory: stacked identical cards + per-stack must-place.
  exactStacks: ExactStack[];
  // Mystery deck: the two implicits the player's crafted deck rolled.
  mysteryPicks: [string, string] | null;
  // Deck-implicit toggle (default ON; reset on deck/mode change). OFF runs
  // the bare layout so players can compare base vs implicit-boosted NDM.
  implicitsEnabled: boolean;
  // Ephemeral what-if tag edits on the placed result (click popup, §9.6).
  // Keyed by `${r},${c}` → replacement group list. Discarded on re-run.
  whatIf: Map<string, GroupTag[]>;
  // Minimum number of stat-giving cards the SA must place. 0 disables.
  minRegularPlaced: number;
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

  // Run
  running: boolean;
  result: TaggedOptimizeResult | null;
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

  optMode: OptimizerMode.MAX,
  depth: Depth.DEFAULT,
  complexCards: false,
  targetedRules: defaultTargetedRules(),
  exactStacks: [],
  mysteryPicks: null,
  implicitsEnabled: true,
  whatIf: new Map(),
  minRegularPlaced: 0,
  autoPlaceArcane: true,   // default; overridden from cfg.arcane.auto_place on boot
  coreState: initialCoreState(),
  bonusCores: 0,           // seeded from cfg.deckmod on boot + mode change

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

/** Toggle every core checkbox at once (Enable-all / Disable-all). */
export function setAllCores(enabled: boolean): void {
  for (const s of app.coreState) s.enabled = enabled;
}

/** Deck-implicit toggle. Clears the run result: a stale result would have
 *  been scored under the other setting and every re-score (badge, what-if)
 *  would silently disagree with it. */
export function setImplicitsEnabled(on: boolean): void {
  if (app.implicitsEnabled === on) return;
  app.implicitsEnabled = on;
  clearRunResult();
}

/** Reset run-derived state when the deck / mode / class changes. */
export function clearRunResult(): void {
  app.result    = null;
  app.elapsedMs = null;
  app.runError  = null;
  app.whatIf    = new Map();   // what-if edits die with the run (§9.6)
}

// ─── Optimizer 2.0 helpers ───────────────────────────────────────────────────

/** Reset every Targeted Min/Max to unbounded. */
export function clearTargetedRules(): void {
  app.targetedRules = defaultTargetedRules();
}

/** The active implicit payloads for the CURRENT app state (deck implicit or
 *  the user's Mystery pair; empty in vanilla). Mirrors tagged.ts logic but
 *  reads the reactive state directly. */
export function currentImplicits(): ImplicitPayload[] {
  if (app.mode === "vanilla" || !app.deck) return [];
  if (!app.implicitsEnabled) return [];   // Deck-card toggle (base-layout runs)
  const def = app.deck.implicit;
  if (!def) return [];
  if (def.kind === "mystery") {
    if (!app.mysteryPicks) return [];
    const cat = implicitCatalog();
    const out: ImplicitPayload[] = [];
    for (const key of app.mysteryPicks) {
      const d = cat[key];
      if (isScoringImplicit(d)) out.push(toPayload(d));
    }
    return out;
  }
  return isScoringImplicit(def) ? [toPayload(def)] : [];
}

/** Result cards with the ephemeral what-if tag edits applied. Slot lookups
 *  go through the RESULT's deck (post-structural shape), not app.deck. */
export function whatIfCards(): TaggedPlaced[] | null {
  if (!app.result) return null;
  if (app.whatIf.size === 0) return app.result.cards;
  const deck = app.result.deck;
  return app.result.cards.map((c, i) => {
    const [r, cc] = deck.slots[i];
    const edited = app.whatIf.get(`${r},${cc}`);
    return edited ? { ...c, groups: [...edited] } : c;
  });
}

/** Re-score the current result with what-if edits applied (score-only sim
 *  pass, not a re-anneal — §9.6). Returns null when no result is live. */
/** colors_real flag for the CURRENT app state — must mirror
 *  tagged.ts::colorsRealFor exactly or the what-if re-score drifts. */
export function currentColorsReal(): boolean {
  if (app.optMode === OptimizerMode.EXACT) return true;
  if (app.complexCards) return true;
  // Mirrors the puzzle rule in colorsRealFor: a color_mismatch implicit
  // forces real colors (currentImplicits() already respects the toggle).
  if (currentImplicits().some((i) => i.kind === "color_mismatch")) return true;
  if (app.optMode === OptimizerMode.TARGETED) {
    return app.targetedRules.some(
      (r) => r.axis === "color" && (r.min !== null || r.max !== null),
    );
  }
  return false;
}

export function whatIfBreakdown() {
  if (!app.result || !app.cfg) return null;
  const cards = whatIfCards();
  if (cards === null) return null;
  const colorsReal = currentColorsReal();
  return simulateTaggedBreakdown(
    app.result.deck, cards, app.cardClass, app.result.coresUsed, currentImplicits(),
    { colorsReal, complex: app.complexCards, wvFoilRules: app.mode !== "vanilla" },
    app.cfg,
  );
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
 *  conversions, and the counter clamps at 0 when the user is over-cap. */
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
      implicit:        deckSrc.implicit,
    },
    cardClass:       app.cardClass,
    bonusCores:      app.bonusCores,
    autoPlaceArcane: app.autoPlaceArcane,
    // v1 pools retired in 2.0 — kept as empty maps for schema compat.
    inventoryCounts: {},
    forcedCounts:    {},
    minRegularPlaced: app.minRegularPlaced,
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
    // — Optimizer 2.0 —
    optMode:      app.optMode,
    depth:        app.depth,
    complexCards: app.complexCards,
    targetedRules: app.targetedRules.map((r) => ({ ...r })),
    exactStacks:   app.exactStacks.map((s) => ({ ...s, groups: [...s.groups] })),
    mysteryPicks:  app.mysteryPicks ? [...app.mysteryPicks] : null,
    implicitsEnabled: app.implicitsEnabled,
    taggedAssignment: app.result.cards.map((c) => [
      c.t, c.color ?? "", c.scaleColor ?? "", [...c.groups],
    ]),
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
    implicit:        snap.deck.implicit ?? null,
  };
  const deck = buildDeck(raw, app.cfg.deckmod);

  // Inputs.
  app.deck            = deck;
  app.cardClass       = snap.cardClass;
  app.bonusCores      = snap.bonusCores;
  app.autoPlaceArcane = snap.autoPlaceArcane;
  app.minRegularPlaced = snap.minRegularPlaced ?? 0;
  // Optimizer 2.0 inputs — v1 snapshots restore as Max/default.
  app.optMode       = snap.optMode ?? OptimizerMode.MAX;
  app.depth         = snap.depth ?? Depth.DEFAULT;
  app.complexCards  = snap.complexCards ?? false;
  app.targetedRules = snap.targetedRules?.map((r) => ({ ...r })) ?? defaultTargetedRules();
  app.exactStacks   = snap.exactStacks?.map((s) => ({ ...s, groups: [...s.groups] })) ?? [];
  app.mysteryPicks  = snap.mysteryPicks ?? null;
  // Must land before the breakdown below — currentImplicits() reads it.
  // Pre-toggle snapshots default to ON (the only behavior that existed).
  app.implicitsEnabled = snap.implicitsEnabled ?? true;
  app.whatIf        = new Map();
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

  // Rebuild the result from the serialized assignment so the grid paints +
  // the breakdown popup works on click.
  //
  // v2 snapshots carry the tagged per-slot cards and re-score under the
  // captured mode's flags + the deck's implicits. v1 snapshots predate tags
  // AND implicits: synthesize the run-level foil bits (so n_ns matches) and
  // re-score with implicits OFF + real colors (the old color-aware model) —
  // showing the run as it was captured, not as 2.0 would score it today.
  const isV2 = Array.isArray(snap.taggedAssignment)
    && snap.taggedAssignment.length === deck.slots.length;
  const foilCore = snap.cores.some((c) => c.core_type === "foil");
  const v1Foil = (snap.mode !== "vanilla" && snap.cardClass === CardClass.SHINY)
    || (snap.cardClass === CardClass.EVO && foilCore);

  const cards: TaggedPlaced[] = [];
  const asgnMap = new Map<string, Placed>();
  for (let i = 0; i < deck.slots.length; i++) {
    let card: TaggedPlaced;
    if (isV2) {
      const [t, color, scale, groups] = snap.taggedAssignment![i];
      card = {
        t: t as any,
        color: (color || null) as any,
        scaleColor: (scale || null) as any,
        groups: groups as GroupTag[],
      };
    } else {
      const [tStr, cStr] = snap.assignment[i] ?? [CardType.EMPTY, ""];
      const t = tStr as any;
      const scorableOrArcane = ["row", "col", "surr", "diag", "deluxe", "typeless", "arcane"].includes(tStr);
      card = {
        t,
        color: (cStr ? cStr : null) as any,
        scaleColor: (cStr ? cStr : null) as any,
        groups: v1Foil && scorableOrArcane ? (["Foil"] as GroupTag[]) : [],
      };
    }
    cards.push(card);
    asgnMap.set(`${deck.slots[i][0]},${deck.slots[i][1]}`, [card.t, card.color]);
  }

  const breakdown = simulateTaggedBreakdown(
    deck, cards, snap.cardClass, snap.cores,
    isV2 ? currentImplicits() : [],
    {
      colorsReal: isV2 ? currentColorsReal() : true,
      complex: isV2 ? (snap.complexCards ?? false) : false,
      wvFoilRules: snap.mode !== "vanilla",
    },
    app.cfg,
  );
  app.result = {
    deck,
    cards,
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
