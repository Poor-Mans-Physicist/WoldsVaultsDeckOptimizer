<script lang="ts">
  // Modal: pick a stat card + tier for a single deck slot. Mirrors
  // `open_assign_dialog` in src/preview.py — searchable list + tier buttons.

  import type { Position } from "../lib/types";
  import type { CardEntry } from "../lib/modifiers";
  import { cardsByFamily } from "../lib/modifiers";
  import type { CardFamily } from "../lib/preview";

  interface Props {
    open:       boolean;
    pos:        Position | null;
    family:     CardFamily | null;
    ndm:        number;
    modifiers:  Map<string, CardEntry> | null;
    current:    { cardKey: string; tier: number } | null;
    onAssign:   (cardKey: string, tier: number) => void;
    onClear:    () => void;
    onClose:    () => void;
  }
  let { open, pos, family, ndm, modifiers, current, onAssign, onClear, onClose }: Props = $props();

  let query = $state("");
  // Reset the search box every time the dialog re-opens.
  $effect(() => { if (open) query = ""; });

  const allCards = $derived<CardEntry[]>(
    (modifiers && family) ? cardsByFamily(modifiers, family) : [],
  );

  const filtered = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return allCards;
    return allCards.filter((c) =>
      c.name.toLowerCase().includes(needle) ||
      c.attributeShort.toLowerCase().includes(needle),
    );
  });

  function onBackdropClick(e: MouseEvent) { if (e.target === e.currentTarget) onClose(); }
  function onKey(e: KeyboardEvent) { if (e.key === "Escape") onClose(); }
</script>

<svelte:window onkeydown={onKey} />

{#if open && pos && family}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="backdrop"
    role="dialog"
    aria-modal="true"
    aria-label="Assign stat card"
    tabindex="-1"
    onclick={onBackdropClick}
  >
    <div class="modal">
      <header>
        <strong>
          Slot ({pos[0]},{pos[1]}) · {family.charAt(0).toUpperCase() + family.slice(1)} family
          · NDM = {ndm.toFixed(2)}
        </strong>
        <button type="button" class="close" onclick={onClose} aria-label="Close">×</button>
      </header>

      {#if allCards.length === 0}
        <p class="empty">No {family} cards loaded from modifiers.json.</p>
      {:else}
        <input
          type="search"
          placeholder="Search (e.g. attack, health, mana)…"
          bind:value={query}
        />
        <div class="list">
          {#each filtered as card (card.key)}
            {@const isSelectedCard = current?.cardKey === card.key}
            <div class="row" class:selected={isSelectedCard}>
              <div class="info">
                <div class="name-line">
                  <span class="name">{card.name}</span>
                  {#if card.isPercent}<span class="pct">%</span>{/if}
                </div>
                <div class="attr">{card.displayAttribute}</div>
              </div>
              <div class="tiers">
                {#each card.tiers as ct (ct.tier)}
                  {@const valStr = card.isPercent
                    ? `${(ct.value * 100).toFixed(2)}%`
                    : ct.value.toString()}
                  <button
                    type="button"
                    class="tier"
                    class:active={isSelectedCard && current?.tier === ct.tier}
                    title="Tier {ct.tier}: {valStr}"
                    onclick={() => onAssign(card.key, ct.tier)}
                  >T{ct.tier}</button>
                {/each}
              </div>
            </div>
          {/each}
          {#if filtered.length === 0}
            <p class="empty">(no matches)</p>
          {/if}
        </div>
      {/if}

      <footer>
        {#if current}
          <button type="button" class="danger" onclick={onClear}>Clear assignment</button>
        {/if}
        <button type="button" onclick={onClose}>Cancel</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: 0;
    background: rgba(15,23,42,.45);
    display: flex; align-items: center; justify-content: center;
    z-index: 100;
  }
  .modal {
    background: #FFFFFF;
    border: 1px solid #E5E7EB;
    border-radius: 8px;
    box-shadow: 0 12px 24px rgba(0,0,0,.18);
    width: 600px;
    max-width: 90vw;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 14px;
    border-bottom: 1px solid #E5E7EB;
    font-size: 13px;
  }
  .close {
    background: transparent;
    border: 0;
    font-size: 20px;
    line-height: 1;
    cursor: pointer;
    color: #6B7280;
    padding: 0 6px;
  }
  .close:hover { color: #111827; }
  input[type="search"] {
    margin: 10px 14px 6px;
    padding: 6px 8px;
    border: 1px solid #D1D5DB;
    border-radius: 4px;
    font-size: 13px;
  }
  .list {
    flex: 1 1 auto;
    overflow-y: auto;
    border: 1px solid #E5E7EB;
    border-radius: 6px;
    margin: 4px 14px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-bottom: 1px solid #F1F3F5;
  }
  .row:last-child { border-bottom: 0; }
  .row.selected { background: #EEF2FF; }
  .info { flex: 1 1 auto; min-width: 0; }
  .name-line { display: flex; align-items: center; gap: 6px; }
  .name { font-size: 13px; font-weight: 500; color: #1F2937; }
  .pct {
    font-size: 10px;
    background: #FEF3C7;
    color: #92400E;
    border-radius: 4px;
    padding: 0 5px;
  }
  .attr { font-size: 11px; color: #6B7280; }
  .tiers { display: flex; gap: 4px; flex-shrink: 0; }
  .tier {
    min-width: 36px;
    padding: 2px 6px;
    font-size: 12px;
    border: 1px solid #D1D5DB;
    background: #FFFFFF;
    border-radius: 4px;
    cursor: pointer;
  }
  .tier:hover { background: #F3F4F6; }
  .tier.active {
    background: #2563EB;
    color: #FFFFFF;
    border-color: #2563EB;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid #E5E7EB;
  }
  footer button {
    padding: 5px 12px;
    background: #FFFFFF;
    border: 1px solid #D1D5DB;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
  }
  footer button:hover { background: #F3F4F6; }
  footer button.danger { color: #B91C1C; border-color: #FCA5A5; }
  footer button.danger:hover { background: #FEF2F2; }
  .empty {
    color: #6B7280;
    font-size: 12px;
    text-align: center;
    padding: 10px;
    margin: 0;
  }
</style>
