<script lang="ts">
  // Max-mode side panel (spec §9.3): a read-out of the PLACED cards,
  // stat-bearing cards first, with the tags a card must carry to earn the
  // deck implicit ("required" — assign these when you build the real card).

  import { app, currentImplicits } from "../lib/state.svelte";
  import { CardType, type GroupTag } from "../lib/types";
  import { COLOR_HEX, TYPE_LABEL } from "../lib/palette";
  import { NOTCH_COLOR, displayTags } from "../lib/notches";

  const STAT_TYPES = new Set<CardType>([
    CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
    CardType.DELUXE, CardType.TYPELESS,
  ]);

  interface Line {
    t: CardType;
    color: string | null;
    groups: GroupTag[];
    count: number;
    stat: boolean;
  }

  const lines = $derived.by<Line[]>(() => {
    if (!app.result) return [];
    const byKey = new Map<string, Line>();
    for (const c of app.result.cards) {
      if (c.t === CardType.DEAD) continue;
      const k = `${c.t}|${c.color}|${[...c.groups].sort().join(",")}`;
      const existing = byKey.get(k);
      if (existing) existing.count++;
      else byKey.set(k, {
        t: c.t, color: c.color, groups: displayTags(c.groups),
        count: 1, stat: STAT_TYPES.has(c.t),
      });
    }
    // Stat-bearing first (spec), then by count descending.
    return [...byKey.values()].sort((a, b) =>
      Number(b.stat) - Number(a.stat) || b.count - a.count);
  });

  // Required colors from color-keyed implicits (idona etc. / gilded etc.).
  const requiredColors = $derived.by<string[]>(() => {
    const out: string[] = [];
    for (const imp of currentImplicits()) {
      for (const c of imp.colors) if (!out.includes(c)) out.push(c);
    }
    return out;
  });

  const deadCount = $derived(
    app.result ? app.result.cards.filter((c) => c.t === CardType.DEAD).length : 0,
  );
</script>

<section class="card">
  <h3>Placed cards</h3>
  {#if !app.result}
    <div class="empty">Run the optimizer to see the ideal build list.</div>
  {:else}
    {#if requiredColors.length > 0}
      <div class="req-color">
        Implicit wants
        {#each requiredColors as c, i}
          {#if i > 0}{" / "}{/if}
          <span class="cdot" style:background={COLOR_HEX[c as keyof typeof COLOR_HEX]}></span>{c}
        {/each}
        cards — Max assumes the whole deck matches.
      </div>
    {:else}
      <div class="req-color muted">Color-agnostic — build the deck in any single color.</div>
    {/if}

    <div class="lines">
      {#each lines as l}
        <div class="line" class:stat={l.stat}>
          <span class="count">×{l.count}</span>
          <span class="tname">{TYPE_LABEL[l.t] ?? l.t}</span>
          <span class="tags">
            {#each l.groups as g}
              <span class="tag" style:--c={NOTCH_COLOR[g]}>{g}</span>
            {/each}
            {#if l.groups.length === 0}
              <span class="notag">no tags needed</span>
            {/if}
          </span>
        </div>
      {/each}
      {#if deadCount > 0}
        <div class="line dead">
          <span class="count">×{deadCount}</span>
          <span class="tname">empty (dead)</span>
          <span class="notag">feeds Void / Shadow scaling</span>
        </div>
      {/if}
    </div>
    <div class="hint">
      Colored tags are <strong>required</strong> for the implicit bonus — any
      extra tags on the real card are harmless.
    </div>
  {/if}
</section>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 12px;
  }
  h3 {
    margin: 0 0 8px;
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .empty { font-size: 12px; color: var(--text-muted); }
  .req-color {
    font-size: 12px;
    color: var(--text-primary);
    margin-bottom: 8px;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-input);
  }
  .req-color.muted { color: var(--text-muted); }
  .cdot {
    display: inline-block;
    width: 9px; height: 9px;
    border-radius: 50%;
    border: 1px solid rgba(0,0,0,.3);
    margin-right: 3px;
    vertical-align: baseline;
  }
  .lines { display: flex; flex-direction: column; gap: 4px; max-height: 50vh; overflow-y: auto; }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-input);
    font-size: 12px;
  }
  .line.stat { border-left: 3px solid var(--accent); }
  .line.dead { opacity: .6; }
  .count { font-family: 'JetBrains Mono', monospace; color: var(--text-secondary); min-width: 30px; }
  .tname { color: var(--text-primary); font-weight: 600; white-space: nowrap; }
  .tags { display: flex; flex-wrap: wrap; gap: 4px; }
  .tag {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--c) 25%, transparent);
    border: 1px solid var(--c);
    color: var(--text-primary);
  }
  .notag { font-size: 11px; color: var(--text-muted); }
  .hint { font-size: 11px; color: var(--text-muted); margin-top: 8px; }
</style>
