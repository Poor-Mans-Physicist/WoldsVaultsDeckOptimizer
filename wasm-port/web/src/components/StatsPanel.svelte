<script lang="ts">
  import { app } from "../lib/state.svelte";
  import { aggregatePreview, humanAttrLabel } from "../lib/preview";

  const agg = $derived(
    app.modifiers
      ? aggregatePreview(app.result, app.previewAssignments, app.cardClass, app.modifiers)
      : null,
  );

  function sorted(m: Map<string, number>): [string, number][] {
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }

  function format(val: number, isPercent: boolean): string {
    return isPercent ? `+${(val * 100).toFixed(2)}%` : `+${val.toFixed(2)}`;
  }
</script>

<div class="card">
  <h3>Player stats (preview)</h3>

  {#if app.modifiersError}
    <p class="msg err">Could not load modifiers.json — preview unavailable.</p>
  {:else if !app.modifiers}
    <p class="msg">Loading modifiers…</p>
  {:else if !app.result}
    <p class="msg">Run the optimizer first to produce a layout.</p>
  {:else if agg}
    <p class="meta">{agg.nAssigned} / {agg.nAssignable} slots assigned</p>

    {#if agg.flat.size === 0 && agg.percent.size === 0}
      <p class="msg">Click a slot in the deck to assign a card.</p>
    {/if}

    {#if agg.flat.size > 0}
      <h4>Flat</h4>
      {#each sorted(agg.flat) as [attr, val]}
        <div class="row">
          <span class="label">{humanAttrLabel(attr)}</span>
          <span class="val" class:pos={val > 0}>{format(val, false)}</span>
        </div>
      {/each}
    {/if}

    {#if agg.percent.size > 0}
      <h4>Percent</h4>
      {#each sorted(agg.percent) as [attr, val]}
        <div class="row">
          <span class="label">{humanAttrLabel(attr)}</span>
          <span class="val" class:pos={val > 0}>{format(val, true)}</span>
        </div>
      {/each}
    {/if}
  {/if}
</div>

<style>
  .card {
    background: #FFFFFF;
    border: 1px solid #E5E7EB;
    border-radius: 6px;
    padding: 12px;
  }
  h3 {
    margin: 0 0 8px 0;
    font-size: 12px;
    text-transform: uppercase;
    color: #6B7280;
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  h4 {
    margin: 10px 0 4px 0;
    font-size: 11px;
    text-transform: uppercase;
    color: #6B7280;
    font-weight: 600;
    letter-spacing: 0.04em;
    border-top: 1px solid #F3F4F6;
    padding-top: 8px;
  }
  .msg { color: #6B7280; font-size: 12px; margin: 6px 0; }
  .msg.err { color: #B91C1C; }
  .meta { color: #6B7280; font-size: 12px; margin: 2px 0 6px 0; }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 2px 0;
  }
  .label { font-size: 13px; color: #1F2937; }
  .val {
    font-size: 13px;
    font-weight: 600;
    font-family: 'JetBrains Mono', 'Consolas', monospace;
    color: #6B7280;
  }
  .val.pos { color: #15803D; }
</style>
