<script lang="ts">
  // Two-tab app: Optimize (run SA, view per-slot NDM, breakdown popup) and
  // Preview (assign concrete stat cards to scoring slots, view aggregate
  // player stats). Mirrors the workflow of src/gui.py.

  import { onMount } from "svelte";
  import {
    app, clearRunResult, clearPreviewAssignments, selectedCores,
    resetStructural,
    addConstructionSlot, removeConstructionSlot,
    convertSlotToArcane, unconvertArcaneSlot,
  } from "./lib/state.svelte";
  import { loadConfigBundle, getMode } from "./lib/config";
  import { loadDecks } from "./lib/deck";
  import { CardClass } from "./lib/types";
  import { optimizeInventoryAsync } from "./lib/workerClient";
  import { filterInventoryByHidden } from "./lib/optimize";
  import { loadModifiers } from "./lib/modifiers";
  import {
    isAssignableSlot, slotFamily, resetAssignmentsOnRun,
  } from "./lib/preview";
  import { classSelectLabel, hiddenInventoryTypes } from "./lib/visibility";
  import {
    effectiveDeck, constructionCandidates, structuralCoreCost,
  } from "./lib/structural";
  import type { SlotBreakdown } from "./lib/breakdown";
  import type { Position, CardType } from "./lib/types";

  import ModeToggle       from "./components/ModeToggle.svelte";
  import DeckGrid         from "./components/DeckGrid.svelte";
  import InventoryTable   from "./components/InventoryTable.svelte";
  import CorePicker       from "./components/CorePicker.svelte";
  import Legend           from "./components/Legend.svelte";
  import BreakdownDialog  from "./components/BreakdownDialog.svelte";
  import AssignDialog     from "./components/AssignDialog.svelte";
  import StatsPanel       from "./components/StatsPanel.svelte";

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
  let assignNdm = $state(0);

  // Per-slot NDM map for grid annotation.
  const perSlotNdm = $derived.by<Map<string, number> | null>(() => {
    if (!app.result) return null;
    const m = new Map<string, number>();
    for (const [k, b] of app.result.breakdown.perSlot) m.set(k, b.finalNdm);
    return m;
  });

  // Effective deck — base deck mutated by structural cores (Construction Core
  // additions, Arcane Core conversions). This is what both the grid renders
  // and the optimizer scores. Returns the base reference when no structural
  // changes are active, so the existing fast path is preserved.
  const fxDeck = $derived(
    app.deck && app.cfg
      ? effectiveDeck(app.deck, app.structural, app.cfg.deckmod)
      : null,
  );

  // Live Construction-Core candidates (used by DeckGrid in placement mode).
  const placementCandidates = $derived(
    app.deck ? constructionCandidates(app.deck, app.structural) : [],
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
    app.deck ? app.deck.base_core_slots + app.bonusCores - structuralCost : 0,
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
    try {
      app.modifiers = await loadModifiers(baseUrl);
    } catch (e) {
      app.modifiersError = e instanceof Error ? e.message : String(e);
    }
  });

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
    // Structural cores may be disallowed in the new mode; clear them outright.
    resetStructural();
    clearRunResult();
    clearPreviewAssignments();
  }

  function onDeckChange(e: Event) {
    const key = (e.currentTarget as HTMLSelectElement).value;
    app.deck = app.decks.find((d) => d.key === key) ?? app.deck;
    // Coordinates from the previous deck don't apply to the new one.
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
    if (!app.cfg || !app.deck || !fxDeck || app.running) return;
    // Hard error before we spin up the SA: structural cores ate the budget.
    // Surfaces cleanly to the user without taking down the page.
    if (rawCoreBudget < 0) {
      app.runError =
        `Structural cores need ${structuralCost} core slot(s) but the deck ` +
        `only has ${app.deck.base_core_slots + app.bonusCores} available ` +
        `(base ${app.deck.base_core_slots} + bonus ${app.bonusCores}). ` +
        `Deselect a core or raise Bonus Cores.`;
      return;
    }
    app.running = true;
    app.runError = null;
    app.result = null;
    app.elapsedMs = null;

    const t0 = performance.now();
    try {
      // Strip card types that the current (mode, class) combo hides — same
      // rule the inventory table uses to gate its rows. Without this, cards
      // stocked in another mode/class (e.g. positional shiny added during a
      // Wold's run) would silently land in the SA's pool when the user flips
      // to vanilla-stat. Filter is silent on purpose — the hide rule is
      // already implied by the inventory table, so a "N ignored" badge would
      // just add noise.
      const hidden = hiddenInventoryTypes(app.mode, app.cardClass, app.cfg);
      const invSnap  = $state.snapshot(app.inventoryCounts);
      const fcSnap   = $state.snapshot(app.forcedCounts);
      const { kept: invKept } = filterInventoryByHidden(invSnap, hidden);
      const { kept: fcKept  } = filterInventoryByHidden(fcSnap,  hidden);

      // The SA sees the structural-modified deck (Construction additions +
      // Arcane conversions baked into slots / arcaneSlots / peers). We then
      // splice in the post-structural-cost core budget so candidate
      // enumeration sees the right number of slots to fill.
      const deckSnap = $state.snapshot(fxDeck);
      deckSnap.core_slots = effectiveCoreSlots;

      // $state.snapshot() unwraps Svelte 5 reactive proxies so structured-clone
      // can ship them across the worker boundary.
      const r = await optimizeInventoryAsync({
        deck:            deckSnap,
        cardClass:       app.cardClass,
        inventory:       invKept,
        forcedCounts:    fcKept,
        autoPlaceArcane: app.autoPlaceArcane,
        cores:           selectedCores(),
        nIter:           app.nIter,
        restarts:        app.restarts,
        cfg:             $state.snapshot(app.cfg),
      });
      app.result = r;

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
  function onSlotClick(key: string, bd: SlotBreakdown) {
    const [r, c] = key.split(",").map(Number);
    const pos = [r, c] as Position;
    if (app.tab === "optimize") {
      breakdownPos  = pos;
      breakdownBd   = bd;
      breakdownOpen = true;
    } else {
      // Preview tab: only open the assign dialog on assignable, scoring slots.
      if (!isAssignableSlot(bd.cardType, app.cardClass)) return;
      assignPos      = pos;
      assignSlotType = bd.cardType;
      assignNdm      = bd.finalNdm;
      assignOpen     = true;
    }
  }
  function closeBreakdown() { breakdownOpen = false; breakdownPos = null; breakdownBd = null; }
  function closeAssign()    { assignOpen    = false; assignPos    = null; assignSlotType = null; }

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
        onclick={() => (app.tab = "optimize")}>Optimize</button>
      <button type="button"
        class:active={app.tab === "preview"}
        onclick={() => (app.tab = "preview")}>Preview</button>
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
{:else}
  <main class="layout">
    <!-- ── Left: controls (shared) ────────────────────────────────── -->
    <aside class="col-left">
      <section class="card">
        <h3>Deck</h3>
        <div class="row">
          <select value={app.deck.key} onchange={onDeckChange}>
            {#each app.decks as d}
              <option value={d.key}>{d.name}</option>
            {/each}
          </select>
        </div>
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
      </section>

      {#if app.tab === "optimize"}
        <section class="card">
          <h3>SA params</h3>
          <div class="row">
            <label>Iterations
              <input type="number" min="1000" step="1000" bind:value={app.nIter} />
            </label>
          </div>
          <div class="row">
            <label>Restarts
              <input type="number" min="1" max="64" step="1" bind:value={app.restarts} />
            </label>
          </div>
          <button class="run" type="button" onclick={run} disabled={app.running}>
            {app.running ? "Optimizing…" : "Run"}
          </button>
          {#if app.runError}
            <div class="err small">{app.runError}</div>
          {/if}
        </section>

        <CorePicker />
      {/if}

      {#if app.tab === "preview"}
        <StatsPanel />
      {/if}
    </aside>

    <!-- ── Center: deck grid + result bar ─────────────────────────── -->
    <section class="col-center">
      {#if app.result && app.tab === "optimize"}
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
          ...r.coresUsed.map((c) => c.color ? `${c.core_type}(${c.color})` : c.core_type),
          ...structLabels,
        ]}
        <div class="result-bar">
          <span class="score">NDM <strong>{r.wasmScore.toFixed(3)}</strong></span>
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
        breakdown={app.result?.breakdown.perSlot ?? null}
        onSlotClick={onSlotClick}
        placementMode={app.structural.constructionEnabled && app.tab === "optimize"}
        conversionMode={app.structural.arcaneCoreEnabled && app.tab === "optimize"}
        placementCandidates={placementCandidates}
        addedSlots={app.structural.addedSlots}
        convertedSlots={app.structural.convertedSlots}
        onPlaceSlot={(p) => addConstructionSlot(p)}
        onRemoveSlot={(p) => removeConstructionSlot(p)}
        onConvertSlot={(p) => convertSlotToArcane(p)}
        onUnconvertSlot={(p) => unconvertArcaneSlot(p)}
      />

      <Legend />
    </section>

    <!-- ── Right: inventory (Optimize only) ───────────────────────── -->
    {#if app.tab === "optimize"}
      <aside class="col-right">
        <InventoryTable />
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
  ndm={assignNdm}
  modifiers={app.modifiers}
  current={currentAssignment}
  onAssign={assignCard}
  onClear={clearAssignment}
  onClose={closeAssign}
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
  .time  { color: var(--text-muted); font-family: 'JetBrains Mono', monospace; }
  .cores { color: var(--text-secondary); font-family: 'JetBrains Mono', monospace; font-size: 12px; }
</style>
