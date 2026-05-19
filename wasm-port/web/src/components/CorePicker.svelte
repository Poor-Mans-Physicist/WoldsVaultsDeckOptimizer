<script lang="ts">
  import { app, setAllCores } from "../lib/state.svelte";
  import { CORE_OPTIONS, coreLabel, coreDefaultPlaceholder } from "../lib/coreOptions";
  import { hiddenCoreTypes } from "../lib/visibility";

  function setOverride(i: number, raw: string): void {
    const v = raw.trim();
    app.coreState[i].override = v === "" ? null : Number(v);
  }

  // Mode-driven row visibility. Vanilla hides deluxe / void / pluto rows;
  // we recompute on every read since state.mode + state.cfg can change.
  const hidden = $derived(
    app.cfg ? hiddenCoreTypes(app.mode, app.cfg) : new Set(),
  );
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
        <input
          type="number"
          class="override"
          step="0.05"
          placeholder={app.cfg ? coreDefaultPlaceholder(opt, app.cfg) : "override"}
          value={app.coreState[i].override ?? ""}
          oninput={(e) => setOverride(i, (e.currentTarget as HTMLInputElement).value)}
        />
      </div>
    {/if}
  {/each}
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
</style>
