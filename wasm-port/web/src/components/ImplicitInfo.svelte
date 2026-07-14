<script lang="ts">
  // Deck-implicit info block for the Deck card (Wold's only): shows the
  // active implicit's effect, and for the Mystery deck exposes the two
  // dropdowns to enter the pair the player's crafted deck actually rolled.

  import { app, clearRunResult, setImplicitsEnabled } from "../lib/state.svelte";
  import { implicitCatalog } from "../lib/deck";
  import { mysteryChoices, isScoringImplicit } from "../lib/implicits";

  const def = $derived(app.mode === "vanilla" ? null : app.deck?.implicit ?? null);
  const isMystery = $derived(def?.kind === "mystery");
  const choices = $derived(isMystery ? mysteryChoices(implicitCatalog()) : []);
  // The on/off box only makes sense when the implicit can reach the kernel
  // (scoring implicit, or Mystery whose picks might be scoring).
  const toggleable = $derived(isScoringImplicit(def) || isMystery);

  function pick(idx: 0 | 1, e: Event) {
    const key = (e.currentTarget as HTMLSelectElement).value;
    const cur: [string, string] = app.mysteryPicks ?? ["", ""];
    const next: [string, string] = [...cur] as [string, string];
    next[idx] = key;
    app.mysteryPicks = next[0] || next[1] ? next : null;
    clearRunResult();
  }

  const pickedDefs = $derived.by(() => {
    if (!isMystery || !app.mysteryPicks) return [];
    const cat = implicitCatalog();
    return app.mysteryPicks.filter(Boolean).map((k) => cat[k]).filter(Boolean);
  });
</script>

{#if def}
  <div class="implicit"
    class:inert={!isScoringImplicit(def) && !isMystery}
    class:off={toggleable && !app.implicitsEnabled}>
    <div class="head">
      <span class="badge">implicit</span>
      <span class="name">{def.name ?? "Deck Modifier"}</span>
      {#if toggleable}
        <!-- Base-vs-implicit comparison switch: OFF scores the bare layout.
             Defaults to ON; toggling clears the run (re-run to compare). -->
        <label class="onoff" title="Untick to optimize the bare layout — compare base NDM vs what the implicit adds. Cleared results re-run under the new setting.">
          <input type="checkbox"
            checked={app.implicitsEnabled}
            onchange={(e) => setImplicitsEnabled((e.currentTarget as HTMLInputElement).checked)} />
          <span>{app.implicitsEnabled ? "on" : "off"}</span>
        </label>
      {/if}
    </div>
    <div class="desc">{def.desc ?? ""}</div>
    {#if toggleable && !app.implicitsEnabled}
      <div class="offnote">Implicit disabled — runs score the base layout.</div>
    {/if}

    {#if isMystery}
      <div class="mystery">
        {#each [0, 1] as idx}
          <select value={app.mysteryPicks?.[idx as 0 | 1] ?? ""}
            onchange={(e) => pick(idx as 0 | 1, e)}>
            <option value="">— rolled implicit {idx + 1} —</option>
            {#each choices as [key, d]}
              <option value={key}>{d.name ?? key}</option>
            {/each}
          </select>
        {/each}
        {#each pickedDefs as d}
          <div class="picked">{d.desc ?? d.name}</div>
        {/each}
        {#if !app.mysteryPicks}
          <div class="warn">Pick the two implicits your crafted deck rolled — until then Mystery runs with none.</div>
        {/if}
      </div>
    {:else if !isScoringImplicit(def)}
      <div class="inert-note">No NDM effect — shown for completeness.</div>
    {/if}
  </div>
{/if}

<style>
  .implicit {
    margin-top: 8px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
  }
  .implicit.inert { opacity: .75; }
  .implicit.off .desc, .implicit.off .name { opacity: .55; }
  .head { display: flex; align-items: center; gap: 6px; }
  .onoff {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-secondary);
    cursor: pointer;
    user-select: none;
  }
  .offnote { font-size: 10px; color: #FCD34D; margin-top: 4px; }
  .badge {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 1px 6px;
    border-radius: 999px;
    background: rgba(99,102,241,.2);
    color: var(--accent);
    border: 1px solid var(--accent);
  }
  .name { font-size: 12px; font-weight: 600; color: var(--text-primary); }
  .desc { font-size: 11px; color: var(--text-secondary); margin-top: 4px; }
  .inert-note { font-size: 10px; color: var(--text-muted); margin-top: 4px; font-style: italic; }
  .mystery { display: flex; flex-direction: column; gap: 4px; margin-top: 6px; }
  select {
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-card);
    color: var(--text-primary);
    font-size: 12px;
    width: 100%;
  }
  .picked { font-size: 10px; color: var(--text-secondary); padding-left: 2px; }
  .warn { font-size: 10px; color: #FCD34D; }
</style>
