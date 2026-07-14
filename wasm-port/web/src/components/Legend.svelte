<script lang="ts">
  import { CardType, CATEGORY_GROUPS, type GroupTag } from "../lib/types";
  import { slotBg } from "../lib/palette";
  import { NOTCH_COLOR } from "../lib/notches";

  // Optimizer 2.0 vocabulary: real greed = the 4 orthogonal directions only
  // (spec §2.3); Wild joins as its own special card.
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
    ["W", "Wild",     CardType.WILD],
    ["_", "Dead",     CardType.DEAD],
  ];
  const dirGreeds: [string, string, CardType][] = [
    ["↑", "Greed Up",    CardType.DIR_GREED_UP],
    ["↓", "Greed Down",  CardType.DIR_GREED_DOWN],
    ["←", "Greed Left",  CardType.DIR_GREED_LEFT],
    ["→", "Greed Right", CardType.DIR_GREED_RIGHT],
  ];
  const entries = [...positional, ...other, ...dirGreeds];

  const notchTags: GroupTag[] = [...CATEGORY_GROUPS, "Stat", "Foil"];
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

  <h3>Tag notches</h3>
  <div class="notch-row">
    {#each notchTags as g}
      <span class="notch-chip">
        <span class="notch" style:background={NOTCH_COLOR[g]}></span>
        {g}
      </span>
    {/each}
  </div>

  <hr />

  <h3>How to use</h3>
  <ul>
    <li>Pick a deck and class, choose an optimizer <em>Mode</em>:
        <strong>Max</strong> = theoretical ceiling with unlimited ideal cards;
        <strong>Targeted</strong> = Max under your per-tag Min/Max limits
        (0 max = ban); <strong>Exact</strong> = places only the cards you
        built in the inventory panel.</li>
    <li>The <em>Depth</em> slider trades speed for search quality
        (Fast 50k×6 · Default 75k×12 · Deep 125k×24).</li>
    <li>Toggle the cores you own. Overrides replace the config default —
        for <code>PURE</code> / <code>DELUXE_CORE</code> / <code>VOID_CORE</code>
        only the <em>scale</em> term; static cores get a flat replacement.</li>
    <li>Hit <em>Run</em>. Tiles show the card glyph, its NDM, and colored
        <em>notches</em> for carried tags. <strong>Hover</strong> a card to
        list its tags; <strong>click</strong> to add/remove tags as a live
        what-if (score updates instantly, discarded on re-run);
        <strong>Shift+click</strong> for the full per-slot math.</li>
    <li>The badge above the deck reports whether the WASM and TS re-score
        paths agree on the total. Green = agreement, red = mismatch.</li>
  </ul>

  <h3>Deck implicits (Wold's)</h3>
  <ul>
    <li>Every Wold's deck's built-in modifier is evaluated automatically —
        shown under the deck picker. Category tags are free in Max/Targeted
        (they only matter through the implicit); Exact reads the real tags
        on your built cards.</li>
    <li>The Mystery deck rolls two random implicits at craft time — pick the
        pair your deck actually rolled under the deck dropdown.</li>
  </ul>

  <h3>Arcane slots</h3>
  <ul>
    <li>Slots marked with the layout char <code>A</code> render with a purple
        border and accept only <code>ARCANE</code> or <code>DEAD</code> cards.
        Placed arcane cards score 0 NDM but count for Pure's <code>n_ns</code>,
        neighbors' peer counts, and the Archive core's exponent.</li>
    <li><em>Auto-place arcane</em> ON keeps arcane slots locked to ARCANE
        (color swaps allowed); OFF lets the optimizer trade them to DEAD to
        feed the Void core.</li>
  </ul>
</div>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 12px;
    font-size: 12px;
    color: var(--text-secondary);
  }
  h3 {
    margin: 10px 0 6px;
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  h3:first-child { margin-top: 0; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 4px;
    border: 1px solid rgba(0,0,0,.15);
    color: #1F2937;
  }
  .glyph { font-family: 'JetBrains Mono', monospace; font-weight: 700; }
  .label { font-size: 11px; }
  .notch-row { display: flex; flex-wrap: wrap; gap: 6px; }
  .notch-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-primary);
  }
  .notch {
    width: 9px; height: 7px;
    border-radius: 2px;
    border: 1px solid rgba(0,0,0,.4);
    display: inline-block;
  }
  hr { border: 0; border-top: 1px solid var(--border); margin: 10px 0; }
  ul { margin: 0; padding-left: 18px; }
  li { margin-bottom: 6px; }
  code, em, strong { color: var(--text-primary); }
</style>
