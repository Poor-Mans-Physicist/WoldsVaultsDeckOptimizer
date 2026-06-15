<script lang="ts">
  import { CardType, type Color, type Position } from "../lib/types";
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

    // ─── Structural-cores integration (optional) ───────────────────────────
    //
    // When `placementMode` is true, dark-grey "+" placeholders appear on every
    // cell in `placementCandidates`; left-click promotes to a real slot, and
    // right-click on a position in `addedSlots` removes it (callers gate the
    // removal validity — DeckGrid just forwards intent).
    //
    // When `conversionMode` is true, left-click on a real slot that isn't
    // already arcane fires `onConvertSlot`; right-click on a position in
    // `convertedSlots` fires `onUnconvertSlot`.
    placementMode?:        boolean;
    conversionMode?:       boolean;
    placementCandidates?:  Position[];
    addedSlots?:           Position[];
    convertedSlots?:       Position[];
    onPlaceSlot?:     (pos: Position) => void;
    onRemoveSlot?:    (pos: Position) => void;
    onConvertSlot?:   (pos: Position) => void;
    onUnconvertSlot?: (pos: Position) => void;

    // ─── Build-mode integration ────────────────────────────────────────────
    //
    // When `buildMode` is true, DeckGrid renders a fixed-size canvas (rows ×
    // cols) instead of the deck-derived bbox. Every cell becomes interactive:
    // left-click → `onBuildClick(pos)`, right-click → `onBuildContextClick(pos)`.
    // The caller dispatches by current tool. Placed regular/arcane sets come
    // from the `deck` prop (which the caller pre-synthesizes from builder
    // state), so all rendering still goes through the same code path.
    buildMode?:        boolean;
    buildRows?:        number;
    buildCols?:        number;
    onBuildClick?:        (pos: Position) => void;
    onBuildContextClick?: (pos: Position) => void;
  }

  let {
    deck, assignment = null, perSlotNdm = null, breakdown = null, onSlotClick,
    placementMode = false, conversionMode = false,
    placementCandidates = [], addedSlots = [], convertedSlots = [],
    onPlaceSlot, onRemoveSlot, onConvertSlot, onUnconvertSlot,
    buildMode = false, buildRows = 6, buildCols = 9,
    onBuildClick, onBuildContextClick,
  }: Props = $props();

  const SLOT_PX = 64;
  const GAP_PX  = 6;

  const slotSet  = $derived(new Set(deck.slots.map((p) => `${p[0]},${p[1]}`)));
  // Position-key set for arcane slots — used to render the purple border.
  const arcaneSet = $derived(new Set(deck.arcaneSlots.map((p) => `${p[0]},${p[1]}`)));
  const candidateSet = $derived(new Set(placementCandidates.map((p) => `${p[0]},${p[1]}`)));
  const addedSet     = $derived(new Set(addedSlots.map((p) => `${p[0]},${p[1]}`)));
  const convertedSet = $derived(new Set(convertedSlots.map((p) => `${p[0]},${p[1]}`)));

  // Bounding box has to cover real slots AND the construction candidate
  // placeholders — without that, candidates that extend past the existing
  // grid edge would have nowhere to render.
  //
  // In buildMode, the bbox is fixed to the canvas size (default 6×9) so the
  // user always has the full 9×6 grid to work with, even when zero tiles are
  // placed yet. The render path is otherwise identical.
  const bbox = $derived.by(() => {
    if (buildMode) {
      return { minR: 0, maxR: buildRows - 1, minC: 0, maxC: buildCols - 1 };
    }
    const rs = deck.slots.map((p) => p[0]);
    const cs = deck.slots.map((p) => p[1]);
    if (placementMode) {
      for (const [r, c] of placementCandidates) { rs.push(r); cs.push(c); }
    }
    return {
      minR: Math.min(...rs), maxR: Math.max(...rs),
      minC: Math.min(...cs), maxC: Math.max(...cs),
    };
  });
  const width  = $derived(bbox.maxC - bbox.minC + 1);
  const height = $derived(bbox.maxR - bbox.minR + 1);

  function placedAt(r: number, c: number): { t: CardType; color: Color | null } {
    if (!assignment) return { t: CardType.EMPTY, color: null };
    const p = assignment.get(`${r},${c}`);
    if (!p) return { t: CardType.EMPTY, color: null };
    return { t: p[0], color: p[1] };
  }

  function handleSlotClick(key: string, r: number, c: number) {
    // Build mode owns every cell — left-click → tool action (caller dispatches
    // by app.builder.tool). No SA result or breakdown is active in this mode.
    if (buildMode) {
      onBuildClick?.([r, c]);
      return;
    }
    // Conversion mode wins over breakdown popups when active. The grid is
    // single-purpose while a structural mode is on.
    if (conversionMode) {
      const pos: Position = [r, c];
      if (convertedSet.has(key)) {
        // Already converted → left-click also reverts (matches the spec's
        // intent that the action is reversible without a modifier key).
        onUnconvertSlot?.(pos);
      } else if (!arcaneSet.has(key)) {
        onConvertSlot?.(pos);
      }
      return;
    }
    if (placementMode) return;   // breakdown popup is suppressed in placement mode
    if (!onSlotClick || !breakdown) return;
    const bd = breakdown.get(key);
    if (bd) onSlotClick(key, bd);
  }

  function handleSlotContext(e: MouseEvent, key: string, r: number, c: number) {
    if (buildMode) {
      e.preventDefault();
      onBuildContextClick?.([r, c]);
      return;
    }
    if (!conversionMode && !placementMode) return;
    e.preventDefault();
    const pos: Position = [r, c];
    if (conversionMode && convertedSet.has(key)) {
      onUnconvertSlot?.(pos);
    } else if (placementMode && addedSet.has(key)) {
      onRemoveSlot?.(pos);
    }
  }

  function handlePlacementClick(pos: Position) {
    onPlaceSlot?.(pos);
  }

  // Build-mode empty cells need their own click handler — `slotSet.has(key)`
  // is false for them, so the existing path renders them as transparent.
  // Wrap that branch in a button when buildMode is on.
  function handleBuildEmpty(r: number, c: number) {
    onBuildClick?.([r, c]);
  }
  function handleBuildEmptyContext(e: MouseEvent, r: number, c: number) {
    e.preventDefault();
    onBuildContextClick?.([r, c]);
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
        {#if placementMode && candidateSet.has(key)}
          <!-- Construction Core: dark-grey "+" placeholder. Left-click adds. -->
          <button
            type="button"
            class="placement-cell"
            style:width="{SLOT_PX}px"
            style:height="{SLOT_PX}px"
            onclick={() => handlePlacementClick([r, c])}
            aria-label="Add slot at {r},{c}"
          >
            <span class="placement-plus">+</span>
          </button>
        {:else if buildMode}
          <!-- Build mode: every empty cell on the 9×6 canvas is clickable.
               Subtle hover styling so the affordance is clear without
               looking like a placed tile. -->
          <button
            type="button"
            class="build-empty"
            style:width="{SLOT_PX}px"
            style:height="{SLOT_PX}px"
            onclick={() => handleBuildEmpty(r, c)}
            oncontextmenu={(e) => handleBuildEmptyContext(e, r, c)}
            aria-label="Empty cell at {r},{c}"
          ></button>
        {:else}
          <div style:width="{SLOT_PX}px" style:height="{SLOT_PX}px" style:background="transparent"></div>
        {/if}
      {:else}
        {@const { t, color } = placedAt(r, c)}
        {@const ndm = perSlotNdm?.get(key)}
        {@const clickable = !!(breakdown?.get(key) && onSlotClick) || conversionMode}
        {@const isArcane = arcaneSet.has(key)}
        {@const isAdded = addedSet.has(key)}
        {@const isConverted = convertedSet.has(key)}
        <button
          type="button"
          class="slot"
          class:clickable
          class:arcane={isArcane}
          class:added={isAdded}
          class:converted={isConverted}
          class:convert-target={conversionMode && !isArcane}
          style:width="{SLOT_PX}px"
          style:height="{SLOT_PX}px"
          style:background={t === CardType.EMPTY ? "var(--bg-empty-slot)" : slotBg(t)}
          onclick={() => handleSlotClick(key, r, c)}
          oncontextmenu={(e) => handleSlotContext(e, key, r, c)}
          aria-label="Slot {r},{c} — {t}{isArcane ? ' (arcane)' : ''}{isAdded ? ' (added)' : ''}"
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

  /* ─── Structural-core states ───────────────────────────────────────────── */

  /* Construction Core: a dark-grey "+" placeholder cell. */
  .placement-cell {
    border: 2px dashed #6B7280;
    background: #374151;
    border-radius: 8px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    opacity: 0.85;
  }
  .placement-cell:hover {
    background: #4B5563;
    opacity: 1;
  }
  .placement-plus {
    font-size: 28px;
    font-weight: 600;
    color: #D1D5DB;
    line-height: 1;
  }

  /* Newly added construction tile — faint accent ring so the player can
     pick it out when right-clicking to remove. */
  .slot.added {
    box-shadow: 0 0 0 2px rgba(99,102,241,0.55);
  }
  /* A converted arcane slot — purple border same as a native arcane, but
     subtly different inner glow to show the player it's reversible. */
  .slot.converted {
    border: 2px solid #A78BFA;
    box-shadow: inset 0 0 0 1px rgba(167,139,250,0.35);
  }
  /* In conversion mode, regular slots are interactable; lift the hover cue. */
  .slot.convert-target { cursor: pointer; }
  .slot.convert-target:hover { outline: 2px solid #A78BFA; outline-offset: -2px; }

  /* Build mode: empty cells are clickable canvas tiles. */
  .build-empty {
    border: 1px dashed #475569;
    background: transparent;
    border-radius: 6px;
    cursor: cell;
    padding: 0;
  }
  .build-empty:hover {
    background: rgba(99,102,241,0.10);
    border-color: var(--accent);
  }
</style>
