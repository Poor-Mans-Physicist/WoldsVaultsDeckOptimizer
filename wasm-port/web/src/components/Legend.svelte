<script lang="ts">
  import { CardType } from "../lib/types";
  import { slotBg } from "../lib/palette";

  // Same grouping as `_build_legend` in src/gui.py, with the new ARCANE chip
  // sitting alongside the other "non-greed" types.
  const positional: [string, string, CardType][] = [
    ["R", "Row",  CardType.ROW],
    ["C", "Col",  CardType.COL],
    ["S", "Surr", CardType.SURR],
    ["X", "Diag", CardType.DIAG],
  ];
  const other: [string, string, CardType][] = [
    ["a", "Arcane",   CardType.ARCANE],
    ["D", "Deluxe",   CardType.DELUXE],
    ["T", "Typeless", CardType.TYPELESS],
    ["_", "Dead",     CardType.DEAD],
  ];
  const dirGreeds: [string, string, CardType][] = [
    ["↑", "Greed Up",    CardType.DIR_GREED_UP],
    ["↓", "Greed Down",  CardType.DIR_GREED_DOWN],
    ["←", "Greed Left",  CardType.DIR_GREED_LEFT],
    ["→", "Greed Right", CardType.DIR_GREED_RIGHT],
    ["↗", "Greed NE",    CardType.DIR_GREED_NE],
    ["↖", "Greed NW",    CardType.DIR_GREED_NW],
    ["↘", "Greed SE",    CardType.DIR_GREED_SE],
    ["↙", "Greed SW",    CardType.DIR_GREED_SW],
  ];
  const otherGreeds: [string, string, CardType][] = [
    ["e", "Evo Greed",  CardType.EVO_GREED],
    ["o", "Surr Greed", CardType.SURR_GREED],
  ];
  const entries = [...positional, ...other, ...dirGreeds, ...otherGreeds];
</script>

<div class="card">
  <h3>Card Key</h3>
  <div class="chips">
    {#each entries as [glyph, label, t]}
      <span class="chip" style:background={slotBg(t)}>
        <span class="glyph">{glyph}</span>
        <span class="label">{label}</span>
      </span>
    {/each}
  </div>

  <hr />

  <h3>How to use</h3>
  <ul>
    <li>Pick a deck and class, then enter how many of each <em>(type, color)</em> card
        you own in the inventory table on the right. Use <em>Unlimited (100×)</em>
        for unconstrained testing or <em>Clear</em> to reset.</li>
    <li>Toggle the cores you own. The override field replaces the config default —
        for <code>PURE</code> / <code>DELUXE_CORE</code> / <code>VOID_CORE</code>
        it overrides only the <em>scale</em> term (formula stays
        <code>base + scale × n</code>); static cores
        (<code>EQUI</code> / <code>STEAD</code> / <code>FOIL</code> / <code>COLOR</code>
        / <code>PLUTO</code>) get a flat replacement.</li>
    <li>Hit <em>Run</em>. The deck repaints with the optimizer's chosen placement;
        each tile shows the card's symbol and its NDM contribution. Click any tile
        to see the full math (base × cores × boost) for that slot.</li>
    <li>The badge above the deck reports whether the WASM and TS re-score paths
        agree on the total. Green = agreement, red = mismatch.</li>
  </ul>

  <h3>Inventory: Regular vs Forced</h3>
  <ul>
    <li><strong>Regular</strong> — the optimizer <em>may</em> place 0 to N of each
        stack. Cap per stack = regular + forced. Use the per-row / per-column
        <code>100×</code> buttons for fast unlimited-style fills.</li>
    <li><strong>Forced</strong> — the optimizer <em>must</em> place at least N
        of each stack. Useful for testing "what if I committed to 3 yellow ROW
        cards?" scenarios. Note: forced ARCANE cards must fit in the deck's
        arcane slots.</li>
  </ul>

  <h3>Arcane slots</h3>
  <ul>
    <li>Slots marked with the layout char <code>A</code> render with a purple
        border. They accept <strong>only</strong> arcane cards (or DEAD cards
        when void core trade-offs apply).</li>
    <li><em>Auto-place arcane</em> (the checkbox near the deck picker):
      <ul>
        <li><strong>ON</strong> (default): every arcane slot is filled with
            ARCANE. SA may still swap arcane colors, but it cannot leave a
            slot DEAD or empty.</li>
        <li><strong>OFF</strong>: SA may swap arcane slots to DEAD when that
            helps the void core. Useful for void-heavy builds.</li>
      </ul>
    </li>
    <li>Arcane cards contribute <strong>0 NDM</strong> directly, but they count
        toward <code>n_ns</code> for Pure-core scaling (EVO-no-FOIL only) and
        toward same-color row/col/peer counts for neighboring cards.</li>
  </ul>

  <h3>Mode toggle</h3>
  <ul>
    <li><strong>Wolds</strong>: full feature set — positional shiny, deluxe,
        evo / surr greeds, void / pluto / deluxe cores all available.</li>
    <li><strong>Vanilla</strong>: stat-card decks only (positional rows hidden
        under Stat class); deluxe / void / pluto / evo / surr disabled.
        Pulls its own deck roster (<code>vh_decks.json</code>).</li>
  </ul>
</div>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 12px;
  }
  h3 {
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    margin: 12px 0 8px 0;
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  h3:first-of-type { margin-top: 0; }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid rgba(0,0,0,.08);
    border-radius: 6px;
    padding: 3px 8px;
    font-size: 12px;
  }
  .chip .glyph {
    font-family: 'JetBrains Mono', 'Consolas', monospace;
    font-weight: 600;
    font-size: 13px;
    min-width: 14px;
    text-align: center;
    color: #1F2937;
  }
  .chip .label { color: #1F2937; }
  hr { margin: 12px 0; border: 0; border-top: 1px solid var(--border); }
  ul {
    margin: 0 0 8px 0;
    padding-left: 1.1em;
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 1.5;
  }
  ul ul { margin-top: 4px; }
  li strong, li em { color: var(--text-primary); }
  code {
    font-size: 11px;
    background: var(--bg-input);
    color: var(--text-primary);
    padding: 0 4px;
    border-radius: 3px;
  }
</style>
