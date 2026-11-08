<script lang="ts">
  // Structural Optimization subsection of the Optimizer card: effort slider,
  // run button, Your/Ideal layout switch (changes only what the center grid
  // renders), and the roll-rarity readout for the player's own layout.

  import { app } from "../lib/state.svelte";
  import { STRUCT_EFFORT, type StructEffort } from "../lib/structuralOpt";

  interface Props {
    /** off = no Construction Core; pending = core in, no valid optimized
     *  layout yet; ready = runnable. */
    status: "off" | "pending" | "ready";
    note?: string | null;
    onRun: () => void;
  }
  let { status, note = null, onRun }: Props = $props();

  const EFFORTS: { value: StructEffort; label: string; hint: string }[] = [
    { value: "standard", label: "Standard",
      hint: "Fast search. Rarity figures are approximate (validated median error 13–60 %)." },
    { value: "deep", label: "Deep",
      hint: "Heavier search and a finer screen. Rarity figures validated to median error 0–22 %." },
  ];

  const s = $derived(app.structOpt);
  const r = $derived(s.report);
  const busy = $derived(s.running || app.running);
  const eff = $derived(STRUCT_EFFORT[s.effort]);

  function compact(n: number): string {
    if (!Number.isFinite(n)) return "∞";
    if (n >= 1e9) return `${(n / 1e9).toPrecision(2)}B`;
    if (n >= 1e6) return `${(n / 1e6).toPrecision(2)}M`;
    if (n >= 1e4) return `${Math.round(n / 1e3)}k`;
    if (n >= 1e3) return `${(n / 1e3).toPrecision(2)}k`;
    return n >= 10 ? `${Math.round(n)}` : n.toPrecision(2);
  }
  const odds = (p: number) => (p > 0 ? `1 in ${compact(1 / p)}` : "—");
  function pct(x: number): string {
    const v = x * 100;
    if (v < 0.01) return "<0.01%";
    return `${v < 1 ? v.toPrecision(2) : v.toFixed(v < 10 ? 1 : 0)}%`;
  }

  const userIsIdeal = $derived(
    !!r && r.ideal.added.length === r.user.added.length
      && r.ideal.added.every(([a, b]) => r.user.added.some(([c, d]) => a === c && b === d)),
  );

  const rollsToBeat = $derived.by(() => {
    if (!r) return null;
    if (r.pBetterHi <= 0) return { main: "none found", sub: "" };
    const main = r.pBetter > 0 ? `~${compact(1 / r.pBetter)}` : "none found";
    if (r.pBetterHi - r.pBetterLo <= r.pBetter * 1e-6) return { main, sub: "" };
    return { main, sub: `${compact(1 / r.pBetterHi)}–${r.pBetterLo > 0 ? compact(1 / r.pBetterLo) : "∞"}` };
  });
</script>

<div class="struct" class:off={status === "off"}>
  <div class="head">Structural Optimization</div>

  {#if status === "pending"}
    <div class="msg">
      {note ?? "Place some structural slots and run the optimizer first before doing structural optimization"}
    </div>
  {:else}
    <fieldset disabled={status === "off"}>
      <div class="seg" role="radiogroup" aria-label="Structural search effort">
        {#each EFFORTS as e}
          <button type="button" class="seg-btn" class:active={s.effort === e.value}
            title={e.hint} disabled={busy} onclick={() => (app.structOpt.effort = e.value)}>
            {e.label}
          </button>
        {/each}
      </div>
      <div class="meta">
        {eff.chains} chains · {eff.chainSteps} steps · screen {eff.screenIter / 1000}k×{eff.screenRestarts}
      </div>

      <button class="run" type="button" onclick={onRun} disabled={busy || status !== "ready"}>
        {s.running ? (s.progress ?? "Optimizing structure…") : "Optimize structure"}
      </button>
      {#if s.error}<div class="err">{s.error}</div>{/if}

      <div class="seg view" class:locked={!r} role="radiogroup" aria-label="Rendered layout">
        {#if r}
          <button type="button" class="seg-btn" class:active={s.view === "user"}
            onclick={() => (app.structOpt.view = "user")}>Your layout</button>
          <button type="button" class="seg-btn" class:active={s.view === "ideal"}
            onclick={() => (app.structOpt.view = "ideal")}>Ideal layout</button>
        {:else}
          <span class="seg-placeholder">Run Optimizer to see ideal layout</span>
        {/if}
      </div>
      {#if r}
        <div class="ideal-line" title="Chance per roll of this exact layout (and counting its score-identical mirror images)">
          Ideal {r.ideal.score.toFixed(1)} NDM · {odds(r.ideal.rollProb)}{#if r.ideal.classProb > r.ideal.rollProb * 1.0001}
            ({odds(r.ideal.classProb)} w/ mirrors){/if}{#if userIsIdeal} · your layout{/if}
        </div>

        <div class="stats" title={`${r.tier} Construction Core (${r.ks[0]} slots per roll) · ${r.sets.toLocaleString()} possible layouts · ${(r.elapsedMs / 1000).toFixed(1)} s${r.fromCache ? " (cached)" : ""}`}>
          <div class="stat">
            <span class="k">Roll chance</span>
            <span class="v">{odds(r.user.rollProb)}</span>
          </div>
          <div class="stat">
            <span class="k">Percentile</span>
            <span class="v">top {pct(r.pAtLeast)}</span>
          </div>
          <div class="stat">
            <span class="k">Of max</span>
            <span class="v">{(r.ofMax * 100).toFixed(1)}%</span>
          </div>
          <div class="stat" title="Expected rolls until one beats your layout, with its uncertainty range">
            <span class="k">Rolls to beat</span>
            <span class="v">{rollsToBeat?.main} <small>{rollsToBeat?.sub}</small></span>
          </div>
        </div>
      {/if}
    </fieldset>
  {/if}
</div>

<style>
  .struct {
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .struct.off { opacity: .45; }
  .head {
    font-size: 11px;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
    margin-bottom: 6px;
  }
  fieldset { border: 0; margin: 0; padding: 0; min-width: 0; }
  .msg { font-size: 12px; color: var(--text-muted); line-height: 1.4; }
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
  .seg-btn:hover:not(:disabled) { color: var(--text-primary); }
  .seg-btn:disabled { cursor: not-allowed; }
  .seg-btn.active { background: var(--accent); color: #fff; font-weight: 600; }
  .seg.view { margin-top: 10px; }
  .seg.locked { opacity: .5; }
  .seg-placeholder {
    flex: 1;
    text-align: center;
    padding: 6px 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .meta { font-size: 11px; color: var(--text-muted); margin: 4px 0 0; }
  .run {
    margin-top: 8px;
    width: 100%;
    padding: 7px 0;
    background: transparent;
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 6px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .run:hover:not(:disabled) { background: var(--bg-hover); }
  .run:disabled { opacity: .6; cursor: not-allowed; }
  .err { color: #FCA5A5; font-size: 12px; margin-top: 6px; }
  .ideal-line { font-size: 11px; color: var(--text-muted); margin-top: 4px; }
  .stats {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px 10px;
    margin-top: 8px;
  }
  .stat { display: flex; flex-direction: column; min-width: 0; }
  .k { font-size: 10px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .v {
    font-size: 13px;
    color: var(--text-primary);
    font-family: 'JetBrains Mono', monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .v small { font-size: 10px; color: var(--text-muted); }
</style>
