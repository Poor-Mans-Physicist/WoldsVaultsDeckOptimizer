<script lang="ts">
  import { app, setAllInventory, fillRow, fillColumn } from "../lib/state.svelte";
  import { ALL_COLORS } from "../lib/types";
  import { INVENTORY_TYPES, TYPE_GLYPH, TYPE_LABEL, COLOR_HEX } from "../lib/palette";
  import { stackKey } from "../lib/optimize";
  import { hiddenInventoryTypes } from "../lib/visibility";

  // Mode-aware row visibility. Hidden rows are skipped in render but kept in
  // state (so values are preserved across mode toggles).
  const hidden = $derived(
    app.cfg ? hiddenInventoryTypes(app.mode, app.cardClass, app.cfg) : new Set(),
  );
  // Visible types in the table — drives both header column-buttons (use all
  // colors regardless) and the per-row stable key list for presets.
  const visibleTypes = $derived(INVENTORY_TYPES.filter((t) => !hidden.has(t)));
  // Stable list of (type|color) keys covering ALL visible cells — used by
  // Unlimited/Clear presets (which operate on the active inventory pool).
  const allKeys = $derived(
    visibleTypes.flatMap((t) => ALL_COLORS.map((c) => stackKey(t, c))),
  );

  // Active-pool helpers. The "view" toggle switches between the regular and
  // forced inventory dicts; both persist in state.
  function activeCounts(): Record<string, number> {
    return app.inventoryView === "forced" ? app.forcedCounts : app.inventoryCounts;
  }
  function valueFor(t: string, c: string): number {
    return activeCounts()[stackKey(t, c)] ?? 0;
  }

  const MAX_COUNT = 100;

  // Clamp + write through. Browsers don't actually block typing above `max`,
  // so we rewrite the DOM value when the user enters > MAX_COUNT.
  function onInput(t: string, c: string, e: Event): void {
    const el = e.currentTarget as HTMLInputElement;
    const raw = Number(el.value);
    const clamped = Math.min(MAX_COUNT, Math.max(0, Math.floor(raw) || 0));
    if (String(clamped) !== el.value) el.value = String(clamped);
    activeCounts()[stackKey(t, c)] = clamped;
  }

  // Zero out one row in the active pool (the small "×" button on the right).
  function clearRow(t: string): void {
    const target = activeCounts();
    for (const c of ALL_COLORS) target[stackKey(t, c)] = 0;
  }

  // Per-row "100×" fill (left of row label).
  function fillRowMax(t: string): void {
    fillRow(MAX_COUNT, ALL_COLORS.map((c) => stackKey(t, c)));
  }

  // Per-column "100×" fill (header, below color dot). Forced view skips this:
  // forced minimums are usually per-stack and a column-wide preset doesn't
  // really make sense for them.
  function fillColumnMax(c: string): void {
    fillColumn(MAX_COUNT, visibleTypes.map((t) => stackKey(t, c)));
  }
</script>

<div class="card">
  <header class="card-head">
    <div class="title-row">
      <h3>Inventory</h3>
      <!-- Regular / Forced view toggle. Both pools persist in state. -->
      <div class="seg">
        {#each ["regular", "forced"] as v}
          <button
            type="button"
            class="seg-btn"
            class:active={app.inventoryView === v}
            onclick={() => (app.inventoryView = v as "regular" | "forced")}
          >{v}</button>
        {/each}
      </div>
    </div>
    <div class="btns">
      <button type="button" class="btn-flat primary" onclick={() => setAllInventory(MAX_COUNT, allKeys)}>
        Unlimited (100×)
      </button>
      <button type="button" class="btn-flat" onclick={() => setAllInventory(0, allKeys)}>
        Clear
      </button>
    </div>
  </header>

  <p class="hint">
    {#if app.inventoryView === "regular"}
      Optimizer may use 0..N of each (cap = regular + forced).
    {:else}
      Optimizer MUST place at least N of each.
    {/if}
  </p>

  <div
    class="grid"
    class:forced={app.inventoryView === "forced"}
    style:grid-template-columns="120px repeat({ALL_COLORS.length}, 1fr) 28px"
  >
    <!-- Header row: empty corner, color dots + (regular only) 100× column buttons. -->
    <div></div>
    {#each ALL_COLORS as c}
      <div class="dot-cell">
        <span class="dot" style:background={COLOR_HEX[c]}></span>
        {#if app.inventoryView === "regular"}
          <button
            type="button"
            class="bulk-btn"
            title="Fill {c} column with {MAX_COUNT}"
            onclick={() => fillColumnMax(c)}
          >{MAX_COUNT}×</button>
        {/if}
      </div>
    {/each}
    <div></div>

    {#each visibleTypes as t}
      <div class="row-label">
        {#if app.inventoryView === "regular"}
          <button
            type="button"
            class="bulk-btn"
            title="Fill every color of {TYPE_LABEL[t]} with {MAX_COUNT}"
            onclick={() => fillRowMax(t)}
          >{MAX_COUNT}×</button>
        {/if}
        <span class="glyph">{TYPE_GLYPH[t]}</span>
        <span class="label">{TYPE_LABEL[t]}</span>
      </div>
      {#each ALL_COLORS as c}
        <input
          type="number"
          inputmode="numeric"
          min="0"
          max={MAX_COUNT}
          step="1"
          value={valueFor(t, c)}
          oninput={(e) => onInput(t, c, e)}
          onfocus={(e) => (e.currentTarget as HTMLInputElement).select()}
        />
      {/each}
      <button
        type="button"
        class="row-clear"
        title="Clear {TYPE_LABEL[t]} row"
        aria-label="Clear {TYPE_LABEL[t]} row"
        onclick={() => clearRow(t)}
      >×</button>
    {/each}
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
    margin-bottom: 4px;
    gap: 12px;
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .card-head h3 {
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    margin: 0;
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  /* Regular / Forced segmented switcher. */
  .seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
  }
  .seg-btn {
    background: transparent;
    border: 0;
    padding: 3px 8px;
    font-size: 11px;
    text-transform: capitalize;
    color: var(--text-secondary);
    cursor: pointer;
  }
  .seg-btn + .seg-btn { border-left: 1px solid var(--border); }
  .seg-btn:hover { background: var(--bg-hover); }
  .seg-btn.active {
    background: var(--accent);
    color: #FFFFFF;
  }
  .hint {
    margin: 0 0 8px 0;
    font-size: 11px;
    color: var(--text-muted);
  }
  .btns { display: flex; gap: 6px; }
  .btn-flat {
    background: transparent;
    border: 0;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 12px;
    text-transform: uppercase;
    font-weight: 600;
    padding: 4px 8px;
    border-radius: 4px;
  }
  .btn-flat.primary { color: var(--accent); }
  .btn-flat:hover { background: var(--bg-hover); }

  .grid {
    display: grid;
    gap: 6px;
    align-items: center;
    padding: 6px;
    border-radius: 6px;
  }
  /* Distinct background tint when editing the forced pool so the user
     visually recognises which inventory they're editing. */
  .grid.forced { background: var(--bg-forced); }

  .dot-cell {
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    gap: 3px;
  }
  .dot {
    display: block;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid rgba(0,0,0,.15);
  }
  .bulk-btn {
    background: transparent;
    border: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: 9px;
    line-height: 14px;
    padding: 0 5px;
    border-radius: 3px;
    font-weight: 600;
  }
  .bulk-btn:hover { background: var(--bg-hover); }

  .row-label {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-left: 2px;
    color: var(--text-primary);
  }
  .row-label .glyph {
    font-family: 'JetBrains Mono', monospace;
    width: 18px;
    text-align: center;
    color: var(--text-secondary);
  }
  .row-label .label { font-size: 12px; }
  input[type="number"] {
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    height: 28px;
    padding: 0 4px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 13px;
    text-align: center;
    font-variant-numeric: tabular-nums;
    background: var(--bg-input);
    color: var(--text-primary);
    -moz-appearance: textfield;
    appearance: textfield;
  }
  input[type="number"]::-webkit-outer-spin-button,
  input[type="number"]::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }
  input[type="number"]:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px rgba(99, 102, 241, .25);
  }
  .row-clear {
    box-sizing: border-box;
    width: 28px;
    height: 28px;
    padding: 0;
    line-height: 1;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-muted);
    border-radius: 4px;
    font-size: 18px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .row-clear:hover {
    background: rgba(220, 38, 38, 0.2);
    border-color: #DC2626;
    color: #FCA5A5;
  }
</style>
