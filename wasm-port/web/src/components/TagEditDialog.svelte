<script lang="ts">
  // Click-a-placed-card popup (spec §9.6): add/remove the non-restrictive
  // tags (category tags + Foil where legal) as an EPHEMERAL what-if overlay.
  // Every toggle triggers a score-only re-sim so the player sees the live
  // NDM effect; edits are discarded on the next Run.

  import { app, whatIfBreakdown } from "../lib/state.svelte";
  import {
    CardClass, CardType, CATEGORY_GROUPS,
    type GroupTag, type Position, type TaggedPlaced,
  } from "../lib/types";
  import { NOTCH_COLOR } from "../lib/notches";
  import { TYPE_LABEL } from "../lib/palette";
  import { isLegalCategorySet } from "../lib/implicits";
  import { legalTagCombos } from "../lib/deck";

  interface Props {
    open: boolean;
    pos: Position | null;
    onClose: () => void;
  }
  let { open, pos, onClose }: Props = $props();

  const slotKey = $derived(pos ? `${pos[0]},${pos[1]}` : null);

  const slotIdx = $derived.by(() => {
    if (!app.result || slotKey === null) return -1;
    return app.result.deck.slots.findIndex(([r, c]) => `${r},${c}` === slotKey);
  });

  const baseCard = $derived.by<TaggedPlaced | null>(() => {
    if (!app.result || slotIdx < 0) return null;
    return app.result.cards[slotIdx];
  });

  const currentGroups = $derived.by<GroupTag[]>(() => {
    if (slotKey === null || baseCard === null) return [];
    return app.whatIf.get(slotKey) ?? baseCard.groups;
  });

  const editable = $derived(
    baseCard !== null && baseCard.t !== CardType.DEAD
    && baseCard.t !== CardType.WILD
    && baseCard.t !== CardType.ARCANE   // arcane cards carry no tags
    && !baseCard.t.startsWith("dir_greed"),
  );

  /** Would adding `g` create a category set no real card has? */
  function comboIllegal(g: GroupTag): boolean {
    if (currentGroups.includes(g)) return false;   // removal always fine
    return !isLegalCategorySet([...currentGroups, g], legalTagCombos());
  }

  // Foil rules in the popup (§9.6): cannot be removed from Wold's shiny
  // cards; cannot be added to evo cards (evo foil comes from the Foil core).
  const foilLocked = $derived(
    app.mode !== "vanilla" && app.cardClass === CardClass.SHINY,
  );
  const foilAddable = $derived(app.cardClass !== CardClass.EVO || currentGroups.includes("Foil"));

  function toggle(g: GroupTag) {
    if (slotKey === null || baseCard === null) return;
    if (g === "Foil") {
      if (foilLocked && currentGroups.includes("Foil")) return;
      if (!foilAddable && !currentGroups.includes("Foil")) return;
    }
    // Stat is run-derived and not toggleable — preserve whatever the card
    // carries (the popup never offers it).
    const next = currentGroups.includes(g)
      ? currentGroups.filter((x) => x !== g)
      : [...currentGroups, g];
    const m = new Map(app.whatIf);
    // Drop the override entirely when it matches the SA's original tags.
    const orig = [...baseCard.groups].sort().join(",");
    if ([...next].sort().join(",") === orig) m.delete(slotKey);
    else m.set(slotKey, next);
    app.whatIf = m;
  }

  function resetSlot() {
    if (slotKey === null) return;
    const m = new Map(app.whatIf);
    m.delete(slotKey);
    app.whatIf = m;
  }

  // Live what-if totals (score-only pass — §9.6).
  const wb = $derived(app.whatIf.size > 0 || open ? whatIfBreakdown() : null);
  const slotNdm = $derived(
    wb && slotKey !== null ? (wb.perSlot.get(slotKey)?.finalNdm ?? 0) : null,
  );
  const deckDelta = $derived(
    wb && app.result ? wb.total - app.result.tsScore : 0,
  );
</script>

{#if open && baseCard}
  <div class="overlay" role="presentation" onclick={onClose}>
    <div class="dialog" role="dialog" aria-label="Edit card tags"
      onclick={(e) => e.stopPropagation()}>
      <h3>
        {TYPE_LABEL[baseCard.t] ?? baseCard.t}
        {#if baseCard.color}<span class="sub">· {baseCard.color}</span>{/if}
        <span class="sub">@ {pos?.[0]},{pos?.[1]}</span>
      </h3>

      {#if !editable}
        <div class="note">This card type carries no editable tags.</div>
      {:else}
        <div class="chips">
          {#each CATEGORY_GROUPS as g}
            <button type="button" class="chip" class:sel={currentGroups.includes(g)}
              disabled={comboIllegal(g)}
              title={comboIllegal(g)
                ? `No real card combines these tags with ${g}`
                : g}
              style:--c={NOTCH_COLOR[g]} onclick={() => toggle(g)}>
              {g}
            </button>
          {/each}
          <button type="button" class="chip"
            class:sel={currentGroups.includes("Foil")}
            disabled={(foilLocked && currentGroups.includes("Foil")) || (!foilAddable && !currentGroups.includes("Foil"))}
            title={foilLocked ? "Wold's shiny cards are always foil" : !foilAddable ? "Evo cards take foil from the Foil core, not per card" : "Foil"}
            style:--c={NOTCH_COLOR.Foil} onclick={() => toggle("Foil")}>
            Foil {foilLocked && currentGroups.includes("Foil") ? "🔒" : ""}
          </button>
        </div>

        {#if slotNdm !== null && wb && app.result}
          <div class="live">
            <div>slot NDM <strong>{slotNdm.toFixed(3)}</strong></div>
            <div>deck NDM <strong>{wb.total.toFixed(3)}</strong>
              <span class="delta" class:up={deckDelta > 1e-9} class:down={deckDelta < -1e-9}>
                {deckDelta >= 0 ? "+" : ""}{deckDelta.toFixed(3)}
              </span>
            </div>
          </div>
        {/if}

        <div class="foot">
          <span class="hint">What-if only — discarded on the next Run.</span>
          <button type="button" class="mini" onclick={resetSlot}
            disabled={slotKey === null || !app.whatIf.has(slotKey)}>Reset slot</button>
        </div>
      {/if}
      <button type="button" class="close" onclick={onClose}>Close</button>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed; inset: 0;
    background: rgba(0,0,0,.45);
    display: flex; align-items: center; justify-content: center;
    z-index: 50;
  }
  .dialog {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px;
    width: 360px;
  }
  h3 { margin: 0 0 10px; font-size: 14px; color: var(--text-primary); }
  .sub { color: var(--text-muted); font-weight: 400; font-size: 12px; }
  .note { font-size: 12px; color: var(--text-muted); margin-bottom: 8px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip {
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 12px;
    cursor: pointer;
  }
  .chip::before {
    content: "";
    display: inline-block;
    width: 8px; height: 8px;
    border-radius: 2px;
    background: var(--c);
    border: 1px solid rgba(0,0,0,.3);
    margin-right: 5px;
  }
  .chip.sel {
    border-color: var(--c);
    background: color-mix(in srgb, var(--c) 18%, var(--bg-input));
    font-weight: 600;
  }
  .chip:disabled { opacity: .55; cursor: not-allowed; }
  .live {
    display: flex;
    justify-content: space-between;
    margin-top: 12px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
    font-size: 12px;
    color: var(--text-secondary);
  }
  .live strong { color: var(--text-primary); font-size: 14px; }
  .delta { font-family: 'JetBrains Mono', monospace; font-size: 11px; margin-left: 4px; }
  .delta.up { color: #6EE7B7; }
  .delta.down { color: #FCA5A5; }
  .foot { display: flex; justify-content: space-between; align-items: center; margin-top: 10px; }
  .hint { font-size: 11px; color: var(--text-muted); }
  .mini {
    font-size: 11px;
    padding: 2px 8px;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text-secondary);
    cursor: pointer;
  }
  .mini:disabled { opacity: .4; cursor: default; }
  .close {
    width: 100%;
    margin-top: 12px;
    padding: 6px 0;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 13px;
    cursor: pointer;
  }
</style>
