<script lang="ts">
  import { CardType, type Color, type GroupTag, type Position } from "../lib/types";
  import type { Deck } from "../lib/deck";
  import type { Placed } from "../lib/types";
  import type { TaggedSlotBreakdown } from "../lib/taggedBreakdown";
  import { slotBg, COLOR_HEX, TYPE_GLYPH } from "../lib/palette";
  import { NOTCH_COLOR, sortTags } from "../lib/notches";

  type SlotBreakdown = TaggedSlotBreakdown;

  interface Props {
    deck: Deck;
    assignment?: Map<string, Placed> | null;
    perSlotNdm?: Map<string, number> | null;
    breakdown?: Map<string, SlotBreakdown> | null;
    /** Per-slot carried tags (post what-if overlay) — drives the notches
     *  and the hover popup (spec §9.5/§9.6). */
    tagsBySlot?: Map<string, GroupTag[]> | null;
    /** Click a placed card. `shiftKey` selects the breakdown view; plain
     *  click opens the tag editor (spec §9.6 rebind). */
    onSlotClick?: (key: string, bd: SlotBreakdown, shiftKey: boolean) => void;

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
    deck, assignment = null, perSlotNdm = null, breakdown = null,
    tagsBySlot = null, onSlotClick,
    placementMode = false, conversionMode = false,
    placementCandidates = [], addedSlots = [], convertedSlots = [],
    onPlaceSlot, onRemoveSlot, onConvertSlot, onUnconvertSlot,
    buildMode = false, buildRows = 6, buildCols = 9,
    onBuildClick, onBuildContextClick,
  }: Props = $props();

  // Hover popup (spec §9.6): tags of the hovered card as colored bubbles.
  let hoverKey: string | null = $state(null);
  let hoverX = $state(0);
  let hoverY = $state(0);
  let gridEl: HTMLDivElement | undefined = $state();

  const hoverTags = $derived.by<GroupTag[] | null>(() => {
    if (hoverKey === null || !tagsBySlot) return null;
    const tags = tagsBySlot.get(hoverKey);
    return tags && tags.length > 0 ? sortTags(tags) : null;
  });

  function onTileEnter(key: string, e: MouseEvent) {
    hoverKey = key;
    trackHover(e);
  }
  function trackHover(e: MouseEvent) {
    if (!gridEl) return;
    const r = gridEl.getBoundingClientRect();
    hoverX = e.clientX - r.left + 14;
    hoverY = e.clientY - r.top + 10;
  }
  function onTileLeave() { hoverKey = null; }

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

  function handleSlotClick(key: string, r: number, c: number, e: MouseEvent) {
    // Build mode owns every cell — left-click → tool action (caller dispatches
    // by app.builder.tool). No SA result or breakdown is active in this mode.
    if (buildMode) {
      onBuildClick?.([r, c]);
      return;
    }
    // Priority: if a run result is available for this slot, the card popup
    // wins over any structural-core tool action (plain click = tag editor,
    // Shift+click = breakdown — spec §9.6). To convert / place again
    // post-run, the user right-clicks an existing converted/added tile to
    // revert, or toggles the core off and on.
    if (breakdown && onSlotClick) {
      const bd = breakdown.get(key);
      if (bd) { onSlotClick(key, bd, e.shiftKey); return; }
    }
    // No result yet → structural-core tools own the click.
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
    // Placement mode: clicks on actual slots are a no-op (placements happen
    // on empty cells via handlePlacementClick); right-click removes.
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
  bind:this={gridEl}
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
        {@const tags = tagsBySlot?.get(key) ?? null}
        <button
          type="button"
          class="slot"
          class:clickable
          class:arcane={isArcane}
          class:added={isAdded}
          class:converted={isConverted}
          class:convert-target={conversionMode && !isArcane}
          class:wild={t === CardType.WILD}
          style:width="{SLOT_PX}px"
          style:height="{SLOT_PX}px"
          style:background={t === CardType.EMPTY ? "var(--bg-empty-slot)" : slotBg(t)}
          onclick={(e) => handleSlotClick(key, r, c, e)}
          oncontextmenu={(e) => handleSlotContext(e, key, r, c)}
          onmouseenter={(e) => onTileEnter(key, e)}
          onmousemove={trackHover}
          onmouseleave={onTileLeave}
          aria-label="Slot {r},{c} — {t}{isArcane ? ' (arcane)' : ''}{isAdded ? ' (added)' : ''}"
        >
          <span class="glyph">
            {#if t === CardType.EMPTY}
              {isArcane ? "a" : "□"}
            {:else if t === CardType.WILD}
              W
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
          {#if tags && tags.length > 0}
            <span class="notches">
              {#each sortTags(tags) as g}
                <span class="notch" style:background={NOTCH_COLOR[g]} title={g}></span>
              {/each}
            </span>
          {/if}
        </button>
      {/if}
    {/each}
  {/each}

  {#if hoverTags}
    <div class="tag-popup" style:left="{hoverX}px" style:top="{hoverY}px">
      {#each hoverTags as g}
        <span class="bubble" style:--c={NOTCH_COLOR[g]}>{g}</span>
      {/each}
    </div>
  {/if}
</div>

<style>
  .grid {
    display: grid;
    padding: 12px;
    background: var(--bg-deck);
    border: 1px solid var(--border);
    border-radius: 10px;
    position: relative;   /* hover tag-popup anchors here */
  }
  /* Tag notches — small colored blocks along the tile's bottom edge (§9.5). */
  .notches {
    position: absolute;
    bottom: 3px;
    left: 4px;
    right: 4px;
    display: flex;
    gap: 2px;
    justify-content: center;
    pointer-events: none;
  }
  .notch {
    width: 7px;
    height: 5px;
    border-radius: 1.5px;
    border: 1px solid rgba(0,0,0,.4);
    flex-shrink: 1;
    min-width: 3px;
  }
  /* Hover popup: tag bubbles colored to their notch color (§9.6). */
  .tag-popup {
    position: absolute;
    z-index: 20;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    max-width: 220px;
    padding: 6px 8px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 4px 14px rgba(0,0,0,.45);
    pointer-events: none;
  }
  .bubble {
    font-size: 10px;
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--c) 26%, transparent);
    border: 1px solid var(--c);
    color: var(--text-primary);
    white-space: nowrap;
  }
  /* Wild tile accent — chartreuse ring (not a group notch). */
  .slot.wild {
    box-shadow: inset 0 0 0 2px #9BCF3B;
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
