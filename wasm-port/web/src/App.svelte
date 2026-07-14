<script lang="ts">
  // Two-tab app: Optimize (run SA, view per-slot NDM, breakdown popup) and
  // Preview (assign concrete stat cards to scoring slots, view aggregate
  // player stats). Mirrors the workflow of src/gui.py.

  import { onMount } from "svelte";
  import {
    app, clearRunResult, clearPreviewAssignments, selectedCores,
    resetStructural, whatIfBreakdown,
    addConstructionSlot, removeConstructionSlot,
    convertSlotToArcane, unconvertArcaneSlot,
    builderCanvasClick, builderCanvasContextClick,
    saveBuilderDeck, reloadSavedDecks,
    captureSnapshot, saveSnapshot, restoreSnapshot, reloadSnapshots,
  } from "./lib/state.svelte";
  import { loadConfigBundle, getMode } from "./lib/config";
  import { loadDecks, implicitCatalog, legalTagCombos } from "./lib/deck";
  import { coreKeyDisplay } from "./lib/coreOptions";
  import {
    CardClass, OptimizerMode, DEPTH_PARAMS,
    type ExactStack, type GroupTag, type TagRuleRow,
  } from "./lib/types";
  import { optimizeTaggedAsync } from "./lib/taggedClient";
  import { loadModifiers } from "./lib/modifiers";
  import {
    isAssignableSlot, slotFamily, slotCategoryTags, resetAssignmentsOnRun,
  } from "./lib/preview";
  import { classSelectLabel } from "./lib/visibility";
  import {
    effectiveDeck, constructionCandidates, structuralCoreCost,
  } from "./lib/structural";
  import {
    builderToDeck, buildModpackJson, allBuilderSlots,
    BUILDER_GRID_WIDTH, BUILDER_GRID_HEIGHT,
  } from "./lib/builder";
  import { defaultLabel as snapshotDefaultLabel, type Snapshot } from "./lib/snapshots";
  import type { TaggedSlotBreakdown as SlotBreakdown } from "./lib/taggedBreakdown";
  import type { Position, CardType } from "./lib/types";

  import ModeToggle             from "./components/ModeToggle.svelte";
  import DeckGrid               from "./components/DeckGrid.svelte";
  import CorePicker             from "./components/CorePicker.svelte";
  import Legend                 from "./components/Legend.svelte";
  import BreakdownDialog        from "./components/BreakdownDialog.svelte";
  import AssignDialog           from "./components/AssignDialog.svelte";
  import StatsPanel             from "./components/StatsPanel.svelte";
  import BuilderPanel           from "./components/BuilderPanel.svelte";
  import ExportJsonDialog       from "./components/ExportJsonDialog.svelte";
  import UnsavedChangesDialog   from "./components/UnsavedChangesDialog.svelte";
  import SnapshotsTab           from "./components/SnapshotsTab.svelte";
  import SaveSnapshotDialog     from "./components/SaveSnapshotDialog.svelte";
  import OptimizerSettings      from "./components/OptimizerSettings.svelte";
  import TargetedPanel          from "./components/TargetedPanel.svelte";
  import ExactPanel             from "./components/ExactPanel.svelte";
  import CardBuilderDialog      from "./components/CardBuilderDialog.svelte";
  import MaxReadout             from "./components/MaxReadout.svelte";
  import TagEditDialog          from "./components/TagEditDialog.svelte";
  import ImplicitInfo           from "./components/ImplicitInfo.svelte";

  const baseUrl = (import.meta.env.BASE_URL ?? "/").endsWith("/")
    ? (import.meta.env.BASE_URL ?? "/")
    : (import.meta.env.BASE_URL ?? "/") + "/";

  // Dialog states — only one is open at a time (gated by `app.tab`).
  let breakdownOpen = $state(false);
  let breakdownPos:  Position | null      = $state(null);
  let breakdownBd:   SlotBreakdown | null = $state(null);

  let assignOpen = $state(false);
  let assignPos:  Position | null = $state(null);
  let assignSlotType: CardType | null = $state(null);
  let assignSlotTags: GroupTag[] = $state([]);
  let assignNdm = $state(0);

  // Live breakdown: the run result, re-scored through the what-if overlay
  // when tag edits are active (score-only pass, §9.6).
  const liveBreakdown = $derived.by(() => {
    if (!app.result) return null;
    if (app.whatIf.size === 0) return app.result.breakdown;
    return whatIfBreakdown() ?? app.result.breakdown;
  });

  // Per-slot NDM map for grid annotation.
  const perSlotNdm = $derived.by<Map<string, number> | null>(() => {
    if (!liveBreakdown) return null;
    const m = new Map<string, number>();
    for (const [k, b] of liveBreakdown.perSlot) m.set(k, b.finalNdm);
    return m;
  });

  // Per-slot carried tags (post what-if) → notches + hover popup. Slot
  // lookups go through the RESULT's deck (post-structural shape).
  const tagsBySlot = $derived.by<Map<string, GroupTag[]> | null>(() => {
    if (!app.result) return null;
    const m = new Map<string, GroupTag[]>();
    for (let i = 0; i < app.result.deck.slots.length; i++) {
      const [r, c] = app.result.deck.slots[i];
      const k = `${r},${c}`;
      m.set(k, app.whatIf.get(k) ?? app.result.cards[i]?.groups ?? []);
    }
    return m;
  });

  // The "base" deck for the active tab:
  //   • Optimize / Preview  → `app.deck` (selected from the dropdown)
  //   • Build               → a synthesized Deck built from the canvas state
  // The structural cores then layer on top of that to produce `fxDeck`, which
  // is what the grid shows AND what the SA scores.
  const baseDeckForTab = $derived(
    app.tab === "build"
      ? (app.cfg ? builderToDeck(app.builder, app.cfg.deckmod) : null)
      : app.deck,
  );

  // Effective deck — base mutated by structural cores (Construction Core
  // additions, Arcane Core conversions). This is what both the grid renders
  // and the optimizer scores. Returns the base reference when no structural
  // changes are active, so the existing fast path is preserved.
  const fxDeck = $derived(
    baseDeckForTab && app.cfg
      ? effectiveDeck(baseDeckForTab, app.structural, app.cfg.deckmod)
      : null,
  );

  // Live Construction-Core candidates (used by DeckGrid in placement mode).
  const placementCandidates = $derived(
    baseDeckForTab ? constructionCandidates(baseDeckForTab, app.structural) : [],
  );

  // Effective core-slot count for the SA. Three layers:
  //   raw            = deck.base_core_slots
  //   + bonusCores   = user-adjustable delta (defaults to mode's deckmod)
  //   − structural   = one slot per active structural core (Construction +
  //                    Arcane Core), since each occupies a real core slot
  // Clamped to ≥ 0 so a negative user delta doesn't break the SA — but if
  // the structural cores push the budget below zero, run() flags an error.
  const structuralCost = $derived(structuralCoreCost(app.structural));
  const rawCoreBudget = $derived(
    baseDeckForTab ? baseDeckForTab.base_core_slots + app.bonusCores - structuralCost : 0,
  );
  const effectiveCoreSlots = $derived(Math.max(0, rawCoreBudget));

  const VERIFY_TOL = 1e-6;

  onMount(async () => {
    // Apply dark theme on root so component CSS-var styles resolve correctly.
    document.documentElement.setAttribute("data-theme", "dark");
    try {
      const bundle = await loadConfigBundle(baseUrl);
      app.bundle = bundle;
      app.mode   = bundle.default_mode;
      app.cfg    = getMode(bundle, app.mode);
      // Seed the arcane auto-place toggle from the resolved config.
      app.autoPlaceArcane = app.cfg.arcane?.auto_place ?? true;
      // Bonus Cores defaults to the mode's configured deckmod (1 in wolds,
      // 0 in vanilla). User-set value is reset whenever the mode flips.
      app.bonusCores = app.cfg.deckmod ?? 0;
      app.decks  = await loadDecks(baseUrl, app.cfg.deckmod, app.mode);
      app.deck   = app.decks[0] ?? null;
    } catch (e) {
      app.bootError = e instanceof Error ? e.message : String(e);
    }
    // Modifiers load is best-effort; preview just degrades if missing.
    await reloadCardCatalog();
    // Populate the Builder's saved-decks list once at boot. Refreshed by the
    // builder helpers (save / delete) on demand thereafter.
    reloadSavedDecks();
    reloadSnapshots();

    // Browser-close guard: warn the user if they're about to lose unsaved
    // builder changes. The exact wording is browser-controlled — we just
    // signal that a confirmation is desired by setting `returnValue`.
    window.addEventListener("beforeunload", (e) => {
      if (app.tab === "build" && app.builder.dirty) {
        e.preventDefault();
        e.returnValue = "";
      }
    });
  });

  // ─── Tab switching with unsaved-changes guard ───────────────────────────
  //
  // Any navigation away from the Build tab that would drop unsaved canvas
  // state is gated through `requestNavigate`. The dialog returns one of:
  //   • Save    → persist, then run the deferred action
  //   • Discard → drop the dirty flag, then run the deferred action
  //   • Cancel  → drop the deferred action entirely
  // BuilderPanel also routes its New / Load-saved actions through the same
  // gate so they can't quietly lose work.

  let unsavedOpen = $state(false);
  let unsavedAction: (() => void) | null = $state(null);

  function requestNavigate(proceed: () => void) {
    if (app.tab === "build" && app.builder.dirty) {
      unsavedAction = proceed;
      unsavedOpen = true;
      return;
    }
    proceed();
  }

  function unsavedSave() {
    if (!app.builder.name.trim() || allBuilderSlots(app.builder).length === 0) {
      // Can't save in this state — abort the navigation and surface the issue.
      alert("Can't save: deck needs a name and at least one slot. Save manually or Discard.");
      return;
    }
    saveBuilderDeck(false);
    unsavedOpen = false;
    const a = unsavedAction; unsavedAction = null;
    a?.();
  }
  function unsavedDiscard() {
    app.builder.dirty = false;
    unsavedOpen = false;
    const a = unsavedAction; unsavedAction = null;
    a?.();
  }
  function unsavedCancel() {
    unsavedOpen = false;
    unsavedAction = null;
  }

  function switchTab(next: "optimize" | "preview" | "build" | "snapshots") {
    if (app.tab === next) return;
    requestNavigate(() => { app.tab = next; });
  }

  // ─── Snapshots ─────────────────────────────────────────────────────────
  // Save: small modal asks for a label (auto-prefilled), persists on confirm.
  // Load (from SnapshotsTab): mode-switch if needed (which may re-fetch decks
  // for the new mode), then restore the captured inputs + result and flip to
  // the Optimize tab so the user sees the grid populated.

  let saveSnapshotOpen = $state(false);

  function openSaveSnapshot() {
    if (!app.result || !app.deck) return;
    saveSnapshotOpen = true;
  }
  function confirmSaveSnapshot(label: string) {
    const snap = captureSnapshot(label);
    if (snap === null) { saveSnapshotOpen = false; return; }
    saveSnapshot(snap);
    saveSnapshotOpen = false;
  }
  function cancelSaveSnapshot() { saveSnapshotOpen = false; }

  const saveSnapshotDefaultLabel = $derived(
    app.deck ? snapshotDefaultLabel(app.deck.name, app.cardClass) : "Snapshot",
  );

  async function onLoadSnapshot(snap: Snapshot) {
    if (!app.bundle) return;
    // Mode-switch may be needed (snapshot is mode-locked). Funnel through the
    // unsaved-builder guard so dirty work isn't lost silently.
    requestNavigate(async () => {
      if (snap.mode !== app.mode) {
        app.mode = snap.mode;
        app.cfg  = getMode(app.bundle!, snap.mode);
        app.autoPlaceArcane = app.cfg.arcane?.auto_place ?? true;
        app.bonusCores = app.cfg.deckmod ?? 0;
        app.decks = await loadDecks(baseUrl, app.cfg.deckmod, snap.mode);
        await reloadCardCatalog();
      }
      restoreSnapshot(snap);
      app.tab = "optimize";
    });
  }

  let exportOpen = $state(false);
  const exportJson = $derived(buildModpackJson(app.builder));

  // Exact-mode card builder wiring: the dialog hands built stacks to the
  // ExactPanel instance (which stacks duplicates ×N).
  let cardBuilderOpen = $state(false);
  let exactPanelRef: { addStack: (s: ExactStack) => void } | undefined = $state();
  function onBuilderAdd(stack: ExactStack) {
    exactPanelRef?.addStack(stack);
  }

  // The structural-core grid affordances (`+` placement, click-to-convert) are
  // shown only when the grid is rendering an actual deck/result, never during
  // the Build tab's blank-canvas mode. In Build tab they re-enable as soon as
  // a run finishes (mutations then apply to the built deck for the next run).
  const structuralUiActive = $derived(
    app.tab === "optimize" || (app.tab === "build" && app.result !== null),
  );

  // The card catalog (Preview chooser + tag legality) is mode-specific —
  // re-fetched on boot and on every mode flip. Cached per mode in modifiers.ts.
  async function reloadCardCatalog() {
    try {
      app.modifiers = await loadModifiers(baseUrl, app.mode);
      app.modifiersError = null;
    } catch (e) {
      app.modifiers = null;
      app.modifiersError = e instanceof Error ? e.message : String(e);
    }
  }

  async function onModeChange(next: string) {
    if (!app.bundle) return;
    const prevName = app.deck?.name ?? null;
    app.mode  = next;
    app.cfg   = getMode(app.bundle, next);
    // Each mode can have its own arcane.auto_place default; re-seed on flip.
    app.autoPlaceArcane = app.cfg.arcane?.auto_place ?? true;
    // Re-seed Bonus Cores from the new mode's deckmod (wolds=1, vanilla=0).
    // Intentionally drops any user override on mode flip — defaults are
    // mode-meaningful and "sticky" Bonus Cores across modes is confusing.
    app.bonusCores = app.cfg.deckmod ?? 0;
    app.decks = await loadDecks(baseUrl, app.cfg.deckmod, next);
    app.deck  = (prevName && app.decks.find((d) => d.name === prevName)) || app.decks[0] || null;
    await reloadCardCatalog();
    // Structural cores may be disallowed in the new mode; clear them outright.
    resetStructural();
    clearRunResult();
    clearPreviewAssignments();
  }

  function onDeckChange(e: Event) {
    const key = (e.currentTarget as HTMLSelectElement).value;
    app.deck = app.decks.find((d) => d.key === key) ?? app.deck;
    // Coordinates from the previous deck don't apply to the new one; a
    // Mystery pair belongs to the Mystery deck only.
    app.mysteryPicks = null;
    resetStructural();
    clearRunResult();
    clearPreviewAssignments();
  }

  function onClassChange(e: Event) {
    app.cardClass = (e.currentTarget as HTMLSelectElement).value as CardClass;
    clearRunResult();
    clearPreviewAssignments();
  }

  async function run() {
    if (!app.cfg || !baseDeckForTab || !fxDeck || app.running) return;
    // Build tab can run an SA against an empty canvas — that just yields a
    // degenerate result. Catch the obvious case here with a friendlier error.
    if (app.tab === "build" && allBuilderSlots(app.builder).length === 0) {
      app.runError = "Place at least one slot on the canvas before running.";
      return;
    }
    // Hard error before we spin up the SA: structural cores ate the budget.
    // Surfaces cleanly to the user without taking down the page.
    if (rawCoreBudget < 0) {
      app.runError =
        `Structural cores need ${structuralCost} core slot(s) but the deck ` +
        `only has ${baseDeckForTab.base_core_slots + app.bonusCores} available ` +
        `(base ${baseDeckForTab.base_core_slots} + bonus ${app.bonusCores}). ` +
        `Deselect a core or raise Bonus Cores.`;
      return;
    }
    app.running = true;
    app.runError = null;
    app.result = null;
    app.elapsedMs = null;

    const t0 = performance.now();
    try {
      // The SA sees the structural-modified deck (Construction additions +
      // Arcane conversions baked into slots / arcaneSlots / peers). We then
      // splice in the post-structural-cost core budget so candidate
      // enumeration sees the right number of slots to fill.
      const deckSnap = $state.snapshot(fxDeck);
      deckSnap.core_slots = effectiveCoreSlots;

      const depth = DEPTH_PARAMS[app.depth];

      // $state.snapshot() unwraps Svelte 5 reactive proxies so structured-clone
      // can ship them across the worker boundary.
      const r = await optimizeTaggedAsync({
        deck:            deckSnap,
        cardClass:       app.cardClass,
        mode:            app.optMode,
        appMode:         app.mode,
        targetedRules:   $state.snapshot(app.targetedRules) as TagRuleRow[],
        exactStacks:     $state.snapshot(app.exactStacks) as ExactStack[],
        mysteryPicks:    app.mysteryPicks ? [...app.mysteryPicks] : null,
        implicitCatalog: implicitCatalog(),
        legalCombos:     legalTagCombos(app.mode),
        complexCards:    app.complexCards,
        minStatPlaced:   app.minRegularPlaced,
        autoPlaceArcane: app.autoPlaceArcane,
        cores:           selectedCores(),
        nIter:           depth.nIter,
        restarts:        depth.restarts,
        cfg:             $state.snapshot(app.cfg),
      });
      app.result = r;
      app.whatIf = new Map();

      // Migrate preview assignments to the new layout (drops orphans).
      if (app.modifiers) {
        const { kept } = resetAssignmentsOnRun(
          app.previewAssignments, app.result, app.cardClass, app.modifiers,
        );
        app.previewAssignments = kept;
      }
    } catch (e) {
      app.runError = e instanceof Error ? e.message : String(e);
    } finally {
      app.elapsedMs = performance.now() - t0;
      app.running   = false;
    }
  }

  // ─── Slot click dispatch — different action per tab ─────────────────────
  // Optimize/Build: plain click = tag-edit what-if popup; Shift+click = the
  // full slot breakdown (spec §9.6 rebind). Preview keeps its assign dialog.
  let tagEditOpen = $state(false);
  let tagEditPos: Position | null = $state(null);

  function onSlotClick(key: string, bd: SlotBreakdown, shiftKey: boolean) {
    const [r, c] = key.split(",").map(Number);
    const pos = [r, c] as Position;
    if (app.tab === "optimize" || app.tab === "build") {
      if (shiftKey) {
        // Show the breakdown for the LIVE (what-if) state when edits exist.
        breakdownPos  = pos;
        breakdownBd   = liveBreakdown?.perSlot.get(key) ?? bd;
        breakdownOpen = true;
      } else {
        tagEditPos  = pos;
        tagEditOpen = true;
      }
    } else {
      // Preview tab: only open the assign dialog on assignable, scoring
      // slots. Non-stat (Resource/Temporal) slots host no stat cards.
      if (!isAssignableSlot(bd.cardType, app.cardClass, bd.groups)) return;
      assignPos      = pos;
      assignSlotType = bd.cardType;
      assignSlotTags = slotCategoryTags(bd.groups);
      assignNdm      = bd.finalNdm;
      assignOpen     = true;
    }
  }
  function closeBreakdown() { breakdownOpen = false; breakdownPos = null; breakdownBd = null; }
  function closeAssign()    { assignOpen    = false; assignPos    = null; assignSlotType = null; }
  function closeTagEdit()   { tagEditOpen   = false; tagEditPos   = null; }

  function assignCard(cardKey: string, tier: number) {
    if (!assignPos) return;
    const key = `${assignPos[0]},${assignPos[1]}`;
    const next = new Map(app.previewAssignments);
    next.set(key, { cardKey, tier });
    app.previewAssignments = next;
    closeAssign();
  }

  function clearAssignment() {
    if (!assignPos) return;
    const key = `${assignPos[0]},${assignPos[1]}`;
    const next = new Map(app.previewAssignments);
    next.delete(key);
    app.previewAssignments = next;
    closeAssign();
  }

  // Selected assignment for the open dialog (drives the "active tier" highlight).
  const currentAssignment = $derived.by(() => {
    if (!assignPos) return null;
    return app.previewAssignments.get(`${assignPos[0]},${assignPos[1]}`) ?? null;
  });

  // For the Preview tab, the dialog needs the family of the clicked slot.
  const dialogFamily = $derived(
    assignSlotType ? slotFamily(assignSlotType, app.cardClass) : null,
  );
</script>

<header class="app-header">
  <h1>Wold's Vaults — Deck Optimizer</h1>
  <div class="header-right">
    <nav class="tabs">
      <button type="button"
        class:active={app.tab === "optimize"}
        onclick={() => switchTab("optimize")}>Optimize</button>
      <button type="button"
        class:active={app.tab === "preview"}
        onclick={() => switchTab("preview")}>Preview</button>
      <button type="button"
        class:active={app.tab === "build"}
        onclick={() => switchTab("build")}>Build</button>
      <button type="button"
        class:active={app.tab === "snapshots"}
        onclick={() => switchTab("snapshots")}>Snapshots</button>
    </nav>
    {#if app.bundle}
      <ModeToggle
        bind:mode={app.mode}
        modes={Object.keys(app.bundle.modes)}
        onChange={onModeChange}
      />
    {/if}
  </div>
</header>

{#if app.bootError}
  <div class="banner err">Boot failed: {app.bootError}</div>
{:else if !app.cfg || !app.deck}
  <div class="banner">Loading…</div>
{:else if app.tab === "snapshots"}
  <main class="layout-wide">
    <SnapshotsTab onLoad={onLoadSnapshot} />
  </main>
{:else}
  <main class="layout">
    <!-- ── Left: controls (shared) ────────────────────────────────── -->
    <aside class="col-left">
      {#if app.tab === "build"}
        <BuilderPanel
          onRequestExport={() => (exportOpen = true)}
          requestNavigate={requestNavigate}
        />
      {/if}

      <section class="card">
        <h3>Deck</h3>
        {#if app.tab !== "build"}
        <div class="row">
          <select value={app.deck.key} onchange={onDeckChange}>
            {#each app.decks as d}
              <option value={d.key}>{d.name}</option>
            {/each}
          </select>
        </div>
        {/if}
        <div class="row">
          <label>
            Class
            <select value={app.cardClass} onchange={onClassChange}>
              <!-- Vanilla relabels Shiny → Stat (purely typeless decks). -->
              <option value={CardClass.SHINY}>{classSelectLabel(app.mode, CardClass.SHINY)}</option>
              <option value={CardClass.EVO}>{classSelectLabel(app.mode, CardClass.EVO)}</option>
            </select>
          </label>
        </div>
        <!-- Arcane auto-place toggle. ON = SA keeps arcane slots as ARCANE
             (color swaps still allowed within them). OFF = SA may swap to
             DEAD for void-core trade-offs. -->
        <div class="row">
          <label class="check-row" title="ON: arcane slots stay ARCANE (color swaps allowed). OFF: SA may swap arcane slots to DEAD for void.">
            <input type="checkbox" bind:checked={app.autoPlaceArcane} />
            <span>Auto-place arcane</span>
          </label>
        </div>
        <div class="meta">
          {(fxDeck ?? app.deck).slots.length} slots ·
          {effectiveCoreSlots} cores{structuralCost > 0 ? ` (−${structuralCost} structural)` : ""} ·
          {(fxDeck ?? app.deck).arcaneSlots.length} arcane
        </div>
        {#if app.tab !== "build"}
          <!-- Deck implicit (Wold's only): effect summary + the Mystery
               deck's rolled-pair dropdowns. -->
          <ImplicitInfo />
        {/if}
      </section>

      {#if app.tab === "optimize" || app.tab === "build"}
        <OptimizerSettings onRun={run} onSaveSnapshot={openSaveSnapshot} />
        <CorePicker />
      {/if}

      {#if app.tab === "preview"}
        <StatsPanel />
      {/if}
    </aside>

    <!-- ── Center: deck grid + result bar ─────────────────────────── -->
    <section class="col-center">
      {#if app.result && (app.tab === "optimize" || app.tab === "build")}
        {@const r = app.result}
        {@const delta = Math.abs(r.wasmScore - r.tsScore)}
        {@const ok = delta <= VERIFY_TOL}
        <!-- Structural cores aren't in r.coresUsed (they never reach the SA);
             surface them in the cores chip alongside the SA-picked ones. -->
        {@const structLabels = [
          ...(app.structural.constructionEnabled ? [`construction_core×${app.structural.addedSlots.length}`] : []),
          ...(app.structural.arcaneCoreEnabled   ? [`arcane_core×${app.structural.convertedSlots.length}`]   : []),
        ]}
        {@const allCoreLabels = [
          ...r.coresUsed.map((c) => c.color
            ? `${coreKeyDisplay(c.core_type)}(${c.color})`
            : coreKeyDisplay(c.core_type)),
          ...structLabels,
        ]}
        <div class="result-bar">
          <span class="score">NDM <strong>{r.wasmScore.toFixed(3)}</strong></span>
          {#if app.whatIf.size > 0 && liveBreakdown}
            <span class="badge whatif" title="Ephemeral tag edits applied — discarded on the next Run">
              what-if {liveBreakdown.total.toFixed(3)}
              ({liveBreakdown.total - r.tsScore >= 0 ? "+" : ""}{(liveBreakdown.total - r.tsScore).toFixed(3)})
            </span>
          {/if}
          <span class="badge" class:ok class:bad={!ok}>
            {ok
              ? `✓ WASM / TS agree (Δ ${delta.toExponential(2)})`
              : `✗ mismatch — WASM ${r.wasmScore.toFixed(3)} vs TS ${r.tsScore.toFixed(3)}`}
          </span>
          {#if app.elapsedMs !== null}
            <span class="time">{app.elapsedMs.toFixed(0)} ms</span>
          {/if}
          {#if allCoreLabels.length}
            <span class="cores">cores: {allCoreLabels.join(", ")}</span>
          {/if}
        </div>
      {/if}

      {#if app.tab === "preview" && !app.result}
        <div class="result-bar muted">
          Run the optimizer (Optimize tab) to produce a layout before assigning cards.
        </div>
      {/if}

      <DeckGrid
        deck={fxDeck ?? app.deck}
        assignment={app.result?.assignment ?? null}
        perSlotNdm={perSlotNdm}
        breakdown={liveBreakdown?.perSlot ?? null}
        tagsBySlot={tagsBySlot}
        onSlotClick={onSlotClick}
        placementMode={app.structural.constructionEnabled && structuralUiActive}
        conversionMode={app.structural.arcaneCoreEnabled && structuralUiActive}
        placementCandidates={placementCandidates}
        addedSlots={app.structural.addedSlots}
        convertedSlots={app.structural.convertedSlots}
        onPlaceSlot={(p) => addConstructionSlot(p)}
        onRemoveSlot={(p) => removeConstructionSlot(p)}
        onConvertSlot={(p) => convertSlotToArcane(p)}
        onUnconvertSlot={(p) => unconvertArcaneSlot(p)}
        buildMode={app.tab === "build" && !app.result}
        buildRows={BUILDER_GRID_HEIGHT}
        buildCols={BUILDER_GRID_WIDTH}
        onBuildClick={builderCanvasClick}
        onBuildContextClick={builderCanvasContextClick}
      />

      <Legend />
    </section>

    <!-- ── Right: mode-dependent side panel (spec §9.3) ─────────────── -->
    {#if app.tab === "optimize" || app.tab === "build"}
      <aside class="col-right">
        {#if app.optMode === OptimizerMode.MAX}
          <MaxReadout />
        {:else if app.optMode === OptimizerMode.TARGETED}
          <TargetedPanel />
        {:else}
          <ExactPanel bind:this={exactPanelRef}
            onOpenBuilder={() => (cardBuilderOpen = true)} />
        {/if}
      </aside>
    {/if}
  </main>
{/if}

<BreakdownDialog
  open={breakdownOpen}
  pos={breakdownPos}
  bd={breakdownBd}
  onClose={closeBreakdown}
/>

<AssignDialog
  open={assignOpen}
  pos={assignPos}
  family={dialogFamily}
  slotTags={assignSlotTags}
  ndm={assignNdm}
  modifiers={app.modifiers}
  current={currentAssignment}
  onAssign={assignCard}
  onClear={clearAssignment}
  onClose={closeAssign}
/>

<ExportJsonDialog
  open={exportOpen}
  json={exportJson}
  onClose={() => (exportOpen = false)}
/>

<UnsavedChangesDialog
  open={unsavedOpen}
  onSave={unsavedSave}
  onDiscard={unsavedDiscard}
  onCancel={unsavedCancel}
/>

<SaveSnapshotDialog
  open={saveSnapshotOpen}
  defaultLabel={saveSnapshotDefaultLabel}
  onConfirm={confirmSaveSnapshot}
  onCancel={cancelSaveSnapshot}
/>

<TagEditDialog
  open={tagEditOpen}
  pos={tagEditPos}
  onClose={closeTagEdit}
/>

<CardBuilderDialog
  open={cardBuilderOpen}
  cardClass={app.cardClass}
  appMode={app.mode}
  complexCards={app.complexCards}
  allowDeluxe={app.cfg?.deluxe.allow ?? true}
  shinyPositional={app.cfg?.shiny.positional ?? true}
  hasArcaneSlots={((fxDeck ?? app.deck)?.arcaneSlots.length ?? 0) > 0}
  onAdd={onBuilderAdd}
  onClose={() => (cardBuilderOpen = false)}
/>

<style>
  /* ── Theme tokens ────────────────────────────────────────────────────
     Single dark-theme palette (set via data-theme on documentElement in
     onMount). All components consume these via var(--…). */
  :global(:root[data-theme="dark"]) {
    --bg-page:        #0F172A;
    --bg-card:        #1E293B;
    --bg-input:       #334155;
    --bg-hover:       rgba(99,102,241,.12);
    --bg-deck:        #0F172A;
    --bg-empty-slot:  #374151;
    --bg-forced:      #3F2F0A;
    --border:         #334155;
    --text-primary:   #F1F5F9;
    --text-secondary: #94A3B8;
    --text-muted:     #64748B;
    --accent:         #6366F1;
  }
  /* Fallback (no data-theme set) — leaves the page coherent if the script
     hasn't run yet, by using the dark palette as a default. */
  :global(:root) {
    --bg-page:        #0F172A;
    --bg-card:        #1E293B;
    --bg-input:       #334155;
    --bg-hover:       rgba(99,102,241,.12);
    --bg-deck:        #0F172A;
    --bg-empty-slot:  #374151;
    --bg-forced:      #3F2F0A;
    --border:         #334155;
    --text-primary:   #F1F5F9;
    --text-secondary: #94A3B8;
    --text-muted:     #64748B;
    --accent:         #6366F1;
  }

  :global(body) {
    margin: 0;
    font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
    background: var(--bg-page);
    color: var(--text-primary);
  }

  .app-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 24px;
    background: var(--bg-card);
    border-bottom: 1px solid var(--border);
  }
  .app-header h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    color: var(--text-primary);
  }
  .header-right { display: flex; align-items: center; gap: 14px; }

  .tabs { display: inline-flex; gap: 4px; }
  .tabs button {
    background: transparent;
    border: 0;
    padding: 6px 12px;
    font-size: 13px;
    color: var(--text-secondary);
    border-bottom: 2px solid transparent;
    cursor: pointer;
  }
  .tabs button:hover { color: var(--text-primary); }
  .tabs button.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
    font-weight: 600;
  }

  .banner {
    margin: 20px auto;
    max-width: 720px;
    padding: 12px 16px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-secondary);
    text-align: center;
  }
  .banner.err { color: #FCA5A5; border-color: #B91C1C; }

  .layout {
    display: grid;
    grid-template-columns: 280px 1fr 360px;
    gap: 16px;
    padding: 16px 24px;
    align-items: start;
  }
  @media (max-width: 1100px) {
    .layout { grid-template-columns: 1fr; }
  }
  /* Snapshots tab is full-width — no side panels. */
  .layout-wide {
    max-width: 1100px;
    margin: 0 auto;
    padding: 16px 24px;
  }

  .col-left, .col-right { display: flex; flex-direction: column; gap: 12px; }
  .col-center { display: flex; flex-direction: column; gap: 12px; align-items: stretch; }

  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 12px;
  }
  .card h3 {
    margin: 0 0 8px 0;
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .row { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  .row label { display: flex; flex-direction: column; gap: 4px; flex-grow: 1; font-size: 12px; color: var(--text-secondary); }
  /* Inline checkbox row for the auto-place toggle. */
  .check-row {
    flex-direction: row !important;
    align-items: center !important;
    cursor: pointer;
    color: var(--text-primary) !important;
    font-size: 13px !important;
  }
  .check-row input { width: auto; margin-right: 4px; }
  select, input[type="number"] {
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 13px;
    width: 100%;
    background: var(--bg-input);
    color: var(--text-primary);
  }
  .meta { font-size: 12px; color: var(--text-muted); margin-top: 4px; }

  .run {
    margin-top: 8px;
    width: 100%;
    padding: 8px 0;
    background: var(--accent);
    color: #FFFFFF;
    border: 0;
    border-radius: 6px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .run:disabled { opacity: .6; cursor: not-allowed; }
  .err { color: #FCA5A5; }
  .err.small { font-size: 12px; margin-top: 6px; }

  /* Run + Save-snapshot button row. Snapshot button shrinks to label-width. */
  .run-row {
    display: flex;
    gap: 6px;
    margin-top: 8px;
  }
  .run-row .run { margin-top: 0; flex-grow: 1; }
  .snap {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
  }
  .snap:hover { background: var(--bg-hover); border-color: var(--accent); color: var(--accent); }

  .result-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    flex-wrap: wrap;
    font-size: 13px;
    color: var(--text-primary);
  }
  .result-bar.muted { color: var(--text-muted); font-style: italic; }
  .score strong { font-size: 16px; }
  .badge {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-family: 'JetBrains Mono', monospace;
  }
  .badge.ok  { background: rgba(16,185,129,.18); color: #6EE7B7; }
  .badge.bad { background: rgba(220,38,38,.22); color: #FCA5A5; }
  .badge.whatif { background: rgba(232,195,59,.18); color: #FCD34D; }
  .time  { color: var(--text-muted); font-family: 'JetBrains Mono', monospace; }
  .cores { color: var(--text-secondary); font-family: 'JetBrains Mono', monospace; font-size: 12px; }
</style>
