<script lang="ts">
  // Optimizer-settings box (spec §9.2) — replaces the raw SA-params inputs.
  // Mode slider (Max / Targeted / Exact), depth slider (Fast / Default /
  // Deep), Complex Cards toggle, min-stat floor, Run + snapshot buttons.

  import { app, clearRunResult } from "../lib/state.svelte";
  import { OptimizerMode, Depth, DEPTH_PARAMS } from "../lib/types";

  interface Props {
    onRun: () => void;
    onSaveSnapshot: () => void;
  }
  let { onRun, onSaveSnapshot }: Props = $props();

  const MODES: { value: OptimizerMode; label: string; hint: string }[] = [
    { value: OptimizerMode.MAX,      label: "Max",
      hint: "Theoretical ceiling: unlimited ideal cards, favorable tags assigned free. What the deck CAN do." },
    { value: OptimizerMode.TARGETED, label: "Targeted",
      hint: "Max + per-tag limits: ban or cap colors, card types, and tags. What the deck can do under YOUR rules." },
    { value: OptimizerMode.EXACT,    label: "Exact",
      hint: "Places only the exact cards you built in the inventory panel. What YOUR cards can do." },
  ];
  const DEPTHS: { value: Depth; label: string }[] = [
    { value: Depth.FAST,    label: "Fast" },
    { value: Depth.DEFAULT, label: "Default" },
    { value: Depth.DEEP,    label: "Deep" },
  ];

  function pickMode(m: OptimizerMode) {
    if (app.optMode === m) return;
    app.optMode = m;
    clearRunResult();
  }

  const depthInfo = $derived(DEPTH_PARAMS[app.depth]);
</script>

<section class="card">
  <h3>Optimizer</h3>

  <div class="seg-label">Mode</div>
  <div class="seg" role="radiogroup" aria-label="Optimizer mode">
    {#each MODES as m}
      <button type="button" class="seg-btn" class:active={app.optMode === m.value}
        title={m.hint} onclick={() => pickMode(m.value)}>
        {m.label}
      </button>
    {/each}
  </div>

  <div class="seg-label">Depth</div>
  <div class="seg" role="radiogroup" aria-label="Search depth">
    {#each DEPTHS as d}
      <button type="button" class="seg-btn" class:active={app.depth === d.value}
        onclick={() => (app.depth = d.value)}>
        {d.label}
      </button>
    {/each}
  </div>
  <div class="meta">
    {depthInfo.nIter.toLocaleString()} iterations × {depthInfo.restarts} restarts
  </div>

  <div class="row">
    <label class="check-row"
      title="Cards may scale off / boost a color different from their own (e.g. a red greed boosting green cards). Significantly slows the optimizer — the per-card option space multiplies by card_color × scale_color.">
      <input type="checkbox" bind:checked={app.complexCards}
        onchange={() => clearRunResult()} />
      <span>Complex Cards</span>
      <span class="warn" class:on={app.complexCards}>slow</span>
    </label>
  </div>

  <div class="row">
    <label title="Lower bound on placed stat-giving cards (positional / deluxe / typeless). 0 disables.">
      Min stat-giving cards
      <input type="number" min="0" step="1" bind:value={app.minRegularPlaced} />
    </label>
  </div>

  <div class="run-row">
    <button class="run" type="button" onclick={onRun} disabled={app.running}>
      {app.running ? "Optimizing…" : "Run"}
    </button>
    {#if app.result}
      <button class="snap" type="button" onclick={onSaveSnapshot}
        title="Save this run as a snapshot">📷</button>
    {/if}
  </div>
  {#if app.runError}
    <div class="err small">{app.runError}</div>
  {/if}
</section>

<style>
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
  .seg-label {
    font-size: 11px;
    color: var(--text-muted);
    margin: 8px 0 4px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .seg {
    display: flex;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg-btn {
    flex: 1;
    padding: 6px 0;
    font-size: 12px;
    background: transparent;
    color: var(--text-secondary);
    border: 0;
    cursor: pointer;
  }
  .seg-btn + .seg-btn { border-left: 1px solid var(--border); }
  .seg-btn:hover { color: var(--text-primary); }
  .seg-btn.active {
    background: var(--accent);
    color: #fff;
    font-weight: 600;
  }
  .meta { font-size: 11px; color: var(--text-muted); margin: 4px 0 2px; }
  .row { display: flex; align-items: center; gap: 8px; margin-top: 8px; }
  .row label {
    display: flex; flex-direction: column; gap: 4px; flex-grow: 1;
    font-size: 12px; color: var(--text-secondary);
  }
  .check-row {
    flex-direction: row !important;
    align-items: center !important;
    cursor: pointer;
    color: var(--text-primary) !important;
    font-size: 13px !important;
  }
  .check-row input { width: auto; margin-right: 4px; }
  .warn {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    background: rgba(220,38,38,.15);
    color: #FCA5A5;
    opacity: .35;
  }
  .warn.on { opacity: 1; }
  input[type="number"] {
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 13px;
    width: 100%;
    background: var(--bg-input);
    color: var(--text-primary);
  }
  .run-row { display: flex; gap: 6px; margin-top: 10px; }
  .run {
    flex-grow: 1;
    padding: 8px 0;
    background: var(--accent);
    color: #fff;
    border: 0;
    border-radius: 6px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .run:disabled { opacity: .6; cursor: not-allowed; }
  .snap {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
  }
  .snap:hover { background: var(--bg-hover); border-color: var(--accent); }
  .err { color: #FCA5A5; }
  .err.small { font-size: 12px; margin-top: 6px; }
</style>
