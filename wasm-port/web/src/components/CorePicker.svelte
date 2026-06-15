<script lang="ts">
  import { app, setAllCores } from "../lib/state.svelte";
  import { CORE_OPTIONS, coreLabel, coreDefaultPlaceholder } from "../lib/coreOptions";
  import { hiddenCoreTypes } from "../lib/visibility";

  function setOverride(i: number, raw: string): void {
    const v = raw.trim();
    app.coreState[i].override = v === "" ? null : Number(v);
  }

  // Mode-driven row visibility. Vanilla hides deluxe / void rows;
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
</style>
