<script lang="ts">
  import { CardType, type Color } from "../lib/types";
  import type { Deck } from "../lib/deck";
  import type { Placed } from "../lib/types";
  import type { SlotBreakdown } from "../lib/breakdown";
  import { slotBg, COLOR_HEX, TYPE_GLYPH } from "../lib/palette";

  interface Props {
    deck: Deck;
    assignment?: Map<string, Placed> | null;
    perSlotNdm?: Map<string, number> | null;
    breakdown?: Map<string, SlotBreakdown> | null;
    onSlotClick?: (key: string, bd: SlotBreakdown) => void;
  }

  let { deck, assignment = null, perSlotNdm = null, breakdown = null, onSlotClick }: Props = $props();

  const SLOT_PX = 64;
  const GAP_PX  = 6;

  // Derived bounding box for the grid (deck slots can have negative offsets;
  // we render every cell in the rect but only paint actual slot positions).
  const bbox = $derived.by(() => {
    const rs = deck.slots.map((p) => p[0]);
    const cs = deck.slots.map((p) => p[1]);
    return {
      minR: Math.min(...rs), maxR: Math.max(...rs),
      minC: Math.min(...cs), maxC: Math.max(...cs),
    };
  });
  const width  = $derived(bbox.maxC - bbox.minC + 1);
  const height = $derived(bbox.maxR - bbox.minR + 1);

  const slotSet  = $derived(new Set(deck.slots.map((p) => `${p[0]},${p[1]}`)));
  // Position-key set for arcane slots — used to render the purple border.
  const arcaneSet = $derived(new Set(deck.arcaneSlots.map((p) => `${p[0]},${p[1]}`)));

  function placedAt(r: number, c: number): { t: CardType; color: Color | null } {
    if (!assignment) return { t: CardType.EMPTY, color: null };
    const p = assignment.get(`${r},${c}`);
    if (!p) return { t: CardType.EMPTY, color: null };
    return { t: p[0], color: p[1] };
  }

  function handleClick(key: string) {
    if (!onSlotClick || !breakdown) return;
    const bd = breakdown.get(key);
    if (bd) onSlotClick(key, bd);
  }
</script>

<div
  class="grid"
  style:grid-template-columns="repeat({width}, {SLOT_PX}px)"
  style:grid-template-rows="repeat({height}, {SLOT_PX}px)"
  style:gap="{GAP_PX}px"
>
  {#each Array(height) as _row, ri}
    {#each Array(width) as _col, ci}
      {@const r = bbox.minR + ri}
      {@const c = bbox.minC + ci}
      {@const key = `${r},${c}`}
      {#if !slotSet.has(key)}
        <div style:width="{SLOT_PX}px" style:height="{SLOT_PX}px" style:background="transparent"></div>
      {:else}
        {@const { t, color } = placedAt(r, c)}
        {@const ndm = perSlotNdm?.get(key)}
        {@const clickable = !!(breakdown?.get(key) && onSlotClick)}
        {@const isArcane = arcaneSet.has(key)}
        <button
          type="button"
          class="slot"
          class:clickable
          class:arcane={isArcane}
          style:width="{SLOT_PX}px"
          style:height="{SLOT_PX}px"
          style:background={t === CardType.EMPTY ? "var(--bg-empty-slot)" : slotBg(t)}
          onclick={() => handleClick(key)}
          aria-label="Slot {r},{c} — {t}{isArcane ? ' (arcane)' : ''}"
        >
          <span class="glyph">
            {#if t === CardType.EMPTY}
              {isArcane ? "a" : "□"}
            {:else}
              {TYPE_GLYPH[t] ?? "·"}
            {/if}
          </span>
          {#if ndm !== undefined && ndm > 0}
            <span class="ndm">{ndm.toFixed(1)}</span>
          {/if}
          {#if color !== null}
            <span class="color-dot" style:background={COLOR_HEX[color]}></span>
          {/if}
        </button>
      {/if}
    {/each}
  {/each}
</div>

<style>
  .grid {
    display: grid;
    padding: 12px;
    background: var(--bg-deck);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .slot {
    position: relative;
    border: 1px solid rgba(0,0,0,.08);
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    font-family: 'JetBrains Mono', 'Consolas', monospace;
    padding: 0;
    cursor: default;
  }
  /* Purple-bordered arcane slots, regardless of contents. */
  .slot.arcane {
    border: 2px solid #A78BFA;
  }
  .slot.clickable { cursor: help; }
  .slot.clickable:hover { outline: 2px solid var(--accent); outline-offset: -2px; }
  .glyph {
    font-size: 22px;
    font-weight: 600;
    line-height: 1;
    /* Slot tile glyphs are dark by default so they stay readable on the
       light-pastel chip backgrounds regardless of page theme. */
    color: #1F2937;
  }
  .ndm {
    font-size: 10px;
    color: #374151;
    margin-top: 2px;
  }
  .color-dot {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1px solid rgba(0,0,0,.2);
  }
</style>
