<script lang="ts">
  import {
    app, setAllCores, setGreedMult, defaultGreedMult,
    toggleConstructionCore, toggleArcaneCore, toggleGreaterStructural,
  } from "../lib/state.svelte";
  import {
    CORE_OPTIONS, coreLabel, coreDefaultPlaceholder,
    coreValueKind, storedToPct, pctToStored,
  } from "../lib/coreOptions";
  import { hiddenCoreTypes } from "../lib/visibility";
  import { maxConstruction, maxArcaneConvert } from "../lib/structural";

  // Overrides are entered as a % (matching the in-game item tooltip) but
  // STORED in raw kernel units, so config defaults, snapshots and the SA
  // payload are untouched. Scaling cores (Pure / Deluxe / Void) take the
  // per-unit increment (6.3 → 0.063); the rest take the flat bonus above 1
  // (150 → 2.5). See coreOptions.coreValueKind.
  function setOverride(i: number, raw: string): void {
    const v = raw.trim();
    app.coreState[i].override = v === ""
      ? null
      : pctToStored(Number(v), coreValueKind(CORE_OPTIONS[i].coreType));
  }

  function overridePct(i: number): number | "" {
    const stored = app.coreState[i].override;
    if (stored === null) return "";
    return storedToPct(stored, coreValueKind(CORE_OPTIONS[i].coreType));
  }

  // Mode-driven row visibility. Vanilla hides deluxe / void rows;
  // we recompute on every read since state.mode + state.cfg can change.
  const hidden = $derived(
    app.cfg ? hiddenCoreTypes(app.mode, app.cfg) : new Set(),
  );

  // Structural-cores subsection visibility. Only render the whole panel when
  // at least one of the two structural cores is allowed in this mode.
  const showConstruction = $derived(app.cfg?.cores.construction_allow ?? false);
  const showArcaneCore   = $derived(app.cfg?.cores.arcane_core_allow  ?? false);
  const showStructural   = $derived(showConstruction || showArcaneCore);

  // Live counters — "placements left" / "conversions left". These wire into
  // the deck grid (it gates the "+" candidates and convert clicks on the same
  // numbers), so showing them here is informational only. Clamp at 0 in case
  // the Greater toggle was switched off while the user was over the base cap.
  const placementsLeft  = $derived(Math.max(0, maxConstruction(app.structural)   - app.structural.addedSlots.length));
  const conversionsLeft = $derived(Math.max(0, maxArcaneConvert(app.structural) - app.structural.convertedSlots.length));

  // Greed knob input: empty box → null (solver falls back to the mode
  // default); anything non-numeric mid-edit is treated the same way.
  function onGreedInput(raw: string): void {
    const t = raw.trim();
    if (t === "") { setGreedMult(null); return; }
    const v = Number(t);
    setGreedMult(Number.isFinite(v) ? v : null);
  }
</script>

<div class="card">
  <header class="card-head">
    <h3>Cores</h3>
    <div class="btns">
      <button type="button" class="btn-flat primary" onclick={() => setAllCores(true)}>Enable all</button>
      <button type="button" class="btn-flat" onclick={() => setAllCores(false)}>Disable all</button>
    </div>
  </header>

  {#each CORE_OPTIONS as opt, i}
    {#if !hidden.has(opt.coreType)}
      <div class="row">
        <label class="check">
          <input type="checkbox" bind:checked={app.coreState[i].enabled} />
          <span>{coreLabel(opt)}</span>
        </label>
        <span class="override-wrap">
          <input
            type="number"
            class="override"
            step="any"
            placeholder={app.cfg ? coreDefaultPlaceholder(opt, app.cfg) : "override"}
            value={overridePct(i)}
            oninput={(e) => setOverride(i, (e.currentTarget as HTMLInputElement).value)}
          />
          <span class="pct" aria-hidden="true">%</span>
        </span>
      </div>
    {/if}
  {/each}

  <!-- ── Bonus Cores ────────────────────────────────────────────────────
       User-adjustable delta on top of the deck's raw core-slot count. The
       optimizer uses `max(0, base + bonusCores)` so the value is unbounded
       in both directions (clamping happens silently if the user types a
       big negative). Defaults to the active mode's `deckmod` and is
       re-seeded on mode flip.
  -->
  <div class="bonus-row">
    <!-- Plain div (not <label>) — the `<details>` info popover would break
         the label/input association anyway, so we associate the input via
         id + for=. -->
    <div class="bonus-label">
      <label for="bonus-cores-input">Bonus Cores</label>
      <details class="info">
        <summary aria-label="What is Bonus Cores?">?</summary>
        <div class="info-body">
          <p>
            Adjusts how many core slots the optimizer has to fill, beyond
            what the deck normally provides.
          </p>
          <p>
            <strong>Positive</strong> — add slots. In Wold's, the Core
            Expertise ability lets you craft a deck with one extra slot and
            strip the temp core in a Deck Altar afterwards (this is why the
            default in Wold's is <code>1</code>).
          </p>
          <p>
            <strong>Negative</strong> — reserve slots for cores the
            optimizer doesn't consider. E.g. you plan to slot a Bounty Core
            for resource cards — set Bonus Cores to <code>-1</code> so the
            optimizer only fills the remaining slots.
          </p>
          <p>
            Vanilla has no equivalent free-slot mechanic, so the default
            there is <code>0</code>.
          </p>
        </div>
      </details>
    </div>
    <input
      id="bonus-cores-input"
      type="number"
      class="override"
      step="1"
      bind:value={app.bonusCores}
    />
  </div>

  <!-- ── Structural cores (Construction + Arcane Core) ────────────────────
       Web-only cores that mutate the deck layout in the UI before SA runs.
       Each costs one of the deck's core-slot budget — the meta line in the
       Deck card shows the effective post-cost number. Toggling either core
       on or off wipes any placed cards (handled in state.toggle* helpers).
  -->
  {#if showStructural}
    <div class="struct-section">
      <div class="struct-head">Structural</div>

      <!-- "Greater" structural variant — bumps both caps from 3 to 5. Sits
           at the top of the section so it's the first thing the user sees
           when expanding the structural panel. -->
      <div class="row greater-row">
        <label class="check">
          <input
            type="checkbox"
            checked={app.structural.greaterStructural}
            onchange={(e) => toggleGreaterStructural((e.currentTarget as HTMLInputElement).checked)}
          />
          <span>Greater structural cores</span>
        </label>
      </div>
      <div class="hint greater-hint">
        Caps go from 3 → 5 for both Construction and Arcane Core.
      </div>

      {#if showConstruction}
        <div class="row">
          <label class="check">
            <input
              type="checkbox"
              checked={app.structural.constructionEnabled}
              onchange={(e) => toggleConstructionCore((e.currentTarget as HTMLInputElement).checked)}
            />
            <span>Construction Core</span>
          </label>
          {#if app.structural.constructionEnabled}
            <span class="counter" class:done={placementsLeft === 0}>
              {placementsLeft} left
            </span>
          {/if}
        </div>
        {#if app.structural.constructionEnabled}
          <div class="hint">
            Click a <span class="hint-plus">+</span> tile to add it · right-click an added tile to remove.
            <em>After a Run, clicks show the slot's breakdown — right-click an added tile to revert and free up the budget.</em>
          </div>
        {/if}
      {/if}

      {#if showArcaneCore}
        <div class="row">
          <label class="check">
            <input
              type="checkbox"
              checked={app.structural.arcaneCoreEnabled}
              onchange={(e) => toggleArcaneCore((e.currentTarget as HTMLInputElement).checked)}
            />
            <span>Arcane Core</span>
          </label>
          {#if app.structural.arcaneCoreEnabled}
            <span class="counter" class:done={conversionsLeft === 0}>
              {conversionsLeft} left
            </span>
          {/if}
        </div>
        {#if app.structural.arcaneCoreEnabled}
          <div class="hint">
            Click a regular slot to convert to arcane · right-click to revert.
            <em>After a Run, clicks show the slot's breakdown — right-click a converted slot to revert and free up the budget.</em>
          </div>
        {/if}
      {/if}
    </div>
  {/if}

  <!-- ── Greed card multiplier ─────────────────────────────────────────────
       One knob for every greed card, entered as the in-game FACE multiplier
       (a best-roll tier-3 card reads ×5). Greeds stack additively, so the
       solver applies (value − 1) as each greed's bonus: 5 ⇒ +400% (the
       default), 3.5 ⇒ +250%. Editing it clears any live result (which was
       optimized under the old value). -->
  <div class="bonus-row">
    <div class="bonus-label">
      <label for="greed-mult-input">Greed card multiplier</label>
      <details class="info">
        <summary aria-label="What is Greed card multiplier?">?</summary>
        <div class="info-body">
          <p>
            The face multiplier on your greed cards — a best-roll tier-3
            card reads <code>×5</code>, so the default is <code>5</code>.
            Non-integers are fine.
          </p>
          <p>
            Greeds stack additively in-game, so the solver counts each greed
            as <code>+(value − 1) × 100%</code> on its target:
            <code>5</code> ⇒ +400%, <code>3.5</code> ⇒ +250%.
          </p>
        </div>
      </details>
    </div>
    <input
      id="greed-mult-input"
      type="number"
      class="override"
      step="any"
      placeholder={String(defaultGreedMult())}
      value={app.greedMult ?? ""}
      oninput={(e) => onGreedInput((e.currentTarget as HTMLInputElement).value)}
    />
  </div>
</div>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 12px;
  }
  .card-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }
  .card-head h3 {
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    margin: 0;
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .btns { display: flex; gap: 6px; }
  .btn-flat {
    background: transparent;
    border: 0;
    cursor: pointer;
    font-size: 12px;
    text-transform: uppercase;
    font-weight: 600;
    padding: 4px 8px;
    border-radius: 4px;
    color: var(--text-secondary);
  }
  .btn-flat.primary { color: var(--accent); }
  .btn-flat:hover { background: var(--bg-hover); }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-grow: 1;
    font-size: 13px;
    cursor: pointer;
    color: var(--text-primary);
  }
  .override {
    width: 100px;
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 13px;
    text-align: right;
    background: var(--bg-input);
    color: var(--text-primary);
  }
  /* % suffix pinned inside the override box so the unit is always visible. */
  .override-wrap { position: relative; display: inline-flex; }
  .override-wrap .override { padding-right: 22px; }
  .override-wrap .pct {
    position: absolute;
    right: 7px;
    top: 50%;
    transform: translateY(-50%);
    font-size: 12px;
    color: var(--text-muted);
    pointer-events: none;
  }

  /* Bonus Cores row — visually divided from the core checkboxes above. */
  .bonus-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 10px;
    margin-top: 8px;
    border-top: 1px solid var(--border);
  }
  .bonus-label {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-grow: 1;
    font-size: 13px;
    color: var(--text-primary);
  }
  /* Collapsible `?` info popover — `<details>` keeps the explanatory text
     out of the way until the user clicks. Anchored next to the label so the
     opened pane drops below without shifting the input. */
  .info { position: relative; }
  .info summary {
    list-style: none;
    cursor: pointer;
    width: 18px;
    height: 18px;
    line-height: 16px;
    text-align: center;
    border-radius: 50%;
    border: 1px solid var(--border);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 600;
    background: var(--bg-input);
  }
  .info summary::-webkit-details-marker { display: none; }
  .info summary:hover {
    color: var(--text-primary);
    border-color: var(--accent);
  }
  .info[open] > summary { color: var(--accent); border-color: var(--accent); }
  .info[open] > .info-body {
    position: absolute;
    top: 22px;
    left: 0;
    width: 260px;
    z-index: 10;
    padding: 10px 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-primary);
    box-shadow: 0 4px 12px rgba(0,0,0,0.4);
  }
  .info-body p { margin: 0; }
  .info-body p + p { margin-top: 6px; }
  .info-body code {
    background: var(--bg-input);
    padding: 1px 4px;
    border-radius: 3px;
    font-size: 11px;
  }

  /* Structural cores subsection — visually separated from the rest. */
  .struct-section {
    padding-top: 10px;
    margin-top: 8px;
    border-top: 1px solid var(--border);
  }
  .struct-head {
    font-size: 11px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
    margin-bottom: 6px;
  }
  .counter {
    font-size: 11px;
    color: var(--accent);
    font-family: 'JetBrains Mono', monospace;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--bg-hover);
  }
  .counter.done { color: var(--text-muted); }
  .hint {
    font-size: 11px;
    color: var(--text-muted);
    padding: 0 0 4px 22px;
    line-height: 1.4;
  }
  .hint em {
    display: block;
    margin-top: 3px;
    color: var(--text-secondary);
    font-style: normal;
  }

  /* Greater-cores toggle visually distinct from the per-core toggles. */
  .greater-row {
    padding: 4px 6px;
    border-radius: 4px;
    margin-bottom: 0;
  }
  .greater-hint {
    padding-left: 6px;
    padding-bottom: 8px;
  }
  .hint-plus {
    display: inline-block;
    width: 14px;
    height: 14px;
    line-height: 12px;
    text-align: center;
    background: #4B5563;
    color: #F1F5F9;
    border-radius: 3px;
    font-weight: 600;
    font-size: 10px;
  }
</style>
