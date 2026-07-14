<script lang="ts">
  // Targeted-mode side panel (spec §9.3): the list of cappable tags, colors
  // and positional/card types first, then the freeform tags. Each row has a
  // Min and a Max input (blank = unbounded, 0 max = ban).

  import { app, clearTargetedRules, clearRunResult } from "../lib/state.svelte";
  import { NOTCH_COLOR } from "../lib/notches";
  import { COLOR_HEX, TYPE_LABEL } from "../lib/palette";
  import type { TagRuleRow, GroupTag, CardType, Color } from "../lib/types";

  function label(r: TagRuleRow): string {
    if (r.axis === "color") return r.key.charAt(0).toUpperCase() + r.key.slice(1);
    if (r.axis === "greed") return "Greed (total)";
    if (r.axis === "group") return r.key;
    return TYPE_LABEL[r.key as CardType] ?? r.key;
  }

  function swatch(r: TagRuleRow): string | null {
    if (r.axis === "color") return COLOR_HEX[r.key as Color] ?? null;
    if (r.axis === "group") return NOTCH_COLOR[r.key as GroupTag] ?? null;
    return null;
  }

  function num(v: string): number | null {
    if (v.trim() === "") return null;
    const n = Math.max(0, Math.floor(Number(v)));
    return Number.isFinite(n) ? n : null;
  }

  function setMin(r: TagRuleRow, e: Event) {
    r.min = num((e.currentTarget as HTMLInputElement).value);
    clearRunResult();
  }
  function setMax(r: TagRuleRow, e: Event) {
    r.max = num((e.currentTarget as HTMLInputElement).value);
    clearRunResult();
  }

  const activeCount = $derived(
    app.targetedRules.filter((r) => r.min !== null || r.max !== null).length,
  );

  // Section splits for headers.
  const isColorSection = (r: TagRuleRow) => r.axis === "color";
  const isTypeSection = (r: TagRuleRow) =>
    r.axis === "type" || r.axis === "greed";
  const colorRows = $derived(app.targetedRules.filter(isColorSection));
  const typeRows = $derived(app.targetedRules.filter(isTypeSection));
  const groupRows = $derived(app.targetedRules.filter((r) => r.axis === "group"));
</script>

<section class="card">
  <div class="head">
    <h3>Tag limits</h3>
    <button type="button" class="mini" onclick={() => { clearTargetedRules(); clearRunResult(); }}
      disabled={activeCount === 0}>Clear all</button>
  </div>
  <div class="hint">Min forces at least N; Max caps at N (0 = ban). Blank = unlimited. A card counts toward every tag it carries.</div>

  {#snippet ruleRow(r: TagRuleRow)}
    <div class="rule" class:set={r.min !== null || r.max !== null}>
      <span class="tag">
        {#if swatch(r)}<span class="dot" style:background={swatch(r)}></span>{/if}
        {label(r)}
      </span>
      <input type="number" min="0" placeholder="min" value={r.min ?? ""}
        oninput={(e) => setMin(r, e)} />
      <input type="number" min="0" placeholder="max" value={r.max ?? ""}
        oninput={(e) => setMax(r, e)} />
    </div>
  {/snippet}

  <div class="section">Colors</div>
  {#each colorRows as r}{@render ruleRow(r)}{/each}

  <div class="section">Card types</div>
  {#each typeRows as r}{@render ruleRow(r)}{/each}

  <div class="section">Tags</div>
  {#each groupRows as r}{@render ruleRow(r)}{/each}
</section>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 12px;
    max-height: 70vh;
    overflow-y: auto;
  }
  .head { display: flex; justify-content: space-between; align-items: center; }
  h3 {
    margin: 0;
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .mini {
    font-size: 11px;
    padding: 2px 8px;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text-secondary);
    cursor: pointer;
  }
  .mini:hover:not(:disabled) { color: var(--text-primary); border-color: var(--accent); }
  .mini:disabled { opacity: .4; cursor: default; }
  .hint { font-size: 11px; color: var(--text-muted); margin: 6px 0 4px; }
  .section {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    margin: 10px 0 4px;
    border-bottom: 1px solid var(--border);
    padding-bottom: 2px;
  }
  .rule {
    display: grid;
    grid-template-columns: 1fr 52px 52px;
    gap: 6px;
    align-items: center;
    padding: 2px 0;
  }
  .rule.set .tag { color: var(--accent); font-weight: 600; }
  .tag {
    font-size: 12px;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .dot {
    width: 9px; height: 9px;
    border-radius: 2px;
    border: 1px solid rgba(0,0,0,.35);
    flex-shrink: 0;
  }
  input[type="number"] {
    width: 100%;
    padding: 3px 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 12px;
    background: var(--bg-input);
    color: var(--text-primary);
  }
  input::placeholder { color: var(--text-muted); font-size: 10px; }
</style>
