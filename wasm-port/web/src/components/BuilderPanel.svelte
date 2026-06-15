<script lang="ts">
  // Builder controls — sits in the left column of the Build tab. Owns the
  // name/core inputs, the canvas tool radio, the saved-decks list, and the
  // Save / Save-As / Export / New buttons.
  //
  // The 9×6 canvas itself is the center column (DeckGrid in buildMode); App
  // .svelte owns that placement so the same component renders in Optimize
  // + Build tabs.

  import {
    app,
    builderSetTool, builderSetName, builderSetCoreCount,
    builderNew, loadSavedDeck, saveBuilderDeck, deleteSavedDeck,
  } from "../lib/state.svelte";
  import { allBuilderSlots } from "../lib/builder";

  interface Props {
    onRequestExport: () => void;
    /** Caller-provided guard — must wrap any action that would lose unsaved
     *  changes (load, new). Returns true to proceed. */
    requestNavigate: (proceed: () => void) => void;
  }
  let { onRequestExport, requestNavigate }: Props = $props();

  // Saved-decks dropdown initial population happens once on mount via the
  // parent App.svelte; reloads happen after save/delete. Local "selected" is
  // bound to a sentinel string so we don't fight Svelte over null values.
  let selectedSaved = $state("");

  function onLoad(key: string) {
    if (!key) return;
    requestNavigate(() => {
      loadSavedDeck(key);
      selectedSaved = key;
    });
  }

  function onDelete(key: string) {
    if (!key) return;
    if (!confirm(`Delete saved deck "${key}"?`)) return;
    deleteSavedDeck(key);
    if (selectedSaved === key) selectedSaved = "";
  }

  function onSave() {
    if (!app.builder.name.trim()) {
      alert("Give the deck a name before saving.");
      return;
    }
    if (allBuilderSlots(app.builder).length === 0) {
      alert("Place at least one slot before saving.");
      return;
    }
    const k = saveBuilderDeck(false);
    selectedSaved = k;
  }

  function onSaveAs() {
    if (!app.builder.name.trim()) {
      alert("Give the deck a name before saving.");
      return;
    }
    if (allBuilderSlots(app.builder).length === 0) {
      alert("Place at least one slot before saving.");
      return;
    }
    const k = saveBuilderDeck(true);
    selectedSaved = k;
  }

  function onNew() {
    requestNavigate(() => {
      builderNew();
      selectedSaved = "";
    });
  }

  const slotCount = $derived(allBuilderSlots(app.builder).length);

  // Keep the dropdown selection in sync with what's actually loaded — covers
  // the case where the user loads a deck via different means in the future.
  $effect(() => {
    if (app.builder.loadedKey !== null && selectedSaved !== app.builder.loadedKey) {
      selectedSaved = app.builder.loadedKey;
    }
  });
</script>

<section class="card">
  <h3>Build a deck</h3>

  <div class="row">
    <label>
      Name
      <input
        type="text"
        placeholder="e.g. Custom Hexagon"
        value={app.builder.name}
        oninput={(e) => builderSetName((e.currentTarget as HTMLInputElement).value)}
      />
    </label>
  </div>

  <div class="row">
    <label>
      Core slots
      <input
        type="number"
        min="0"
        step="1"
        value={app.builder.coreCount}
        oninput={(e) => builderSetCoreCount(Number((e.currentTarget as HTMLInputElement).value))}
      />
    </label>
  </div>

  <div class="meta">
    {slotCount} placed slot{slotCount === 1 ? "" : "s"} ·
    {app.builder.arcaneSlots.length} arcane
    {#if app.builder.dirty} · <span class="dirty">unsaved</span>{/if}
  </div>

  <div class="tool-section">
    <div class="tool-head">Tool</div>
    <div class="tool-radio">
      <label class:active={app.builder.tool === "regular"}>
        <input
          type="radio"
          name="builder-tool"
          checked={app.builder.tool === "regular"}
          onchange={() => builderSetTool("regular")}
        />
        <span>Regular (O)</span>
      </label>
      <label class:active={app.builder.tool === "arcane"}>
        <input
          type="radio"
          name="builder-tool"
          checked={app.builder.tool === "arcane"}
          onchange={() => builderSetTool("arcane")}
        />
        <span>Arcane (A)</span>
      </label>
      <label class:active={app.builder.tool === "erase"}>
        <input
          type="radio"
          name="builder-tool"
          checked={app.builder.tool === "erase"}
          onchange={() => builderSetTool("erase")}
        />
        <span>Erase</span>
      </label>
    </div>
    <div class="hint">Right-click any cell erases regardless of the active tool.</div>
  </div>

  <div class="btn-row">
    <button type="button" class="primary" onclick={onSave}>Save</button>
    <button type="button" onclick={onSaveAs}>Save as…</button>
    <button type="button" onclick={onNew}>New</button>
  </div>
  <div class="btn-row">
    <button type="button" class="export" onclick={onRequestExport} disabled={slotCount === 0}>
      Export JSON
    </button>
  </div>

  {#if app.savedDecks.length > 0}
    <div class="saved-section">
      <div class="saved-head">Saved decks</div>
      <div class="row">
        <select
          bind:value={selectedSaved}
          onchange={(e) => onLoad((e.currentTarget as HTMLSelectElement).value)}
        >
          <option value="">— load a saved deck —</option>
          {#each app.savedDecks as d}
            <option value={d.key}>{d.name} ({d.key})</option>
          {/each}
        </select>
        {#if selectedSaved}
          <button type="button" class="trash" onclick={() => onDelete(selectedSaved)} title="Delete this saved deck">🗑</button>
        {/if}
      </div>
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
    margin: 0 0 8px 0;
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .row { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
  .row label {
    display: flex; flex-direction: column; gap: 4px;
    flex-grow: 1;
    font-size: 12px;
    color: var(--text-secondary);
  }
  input[type="text"], input[type="number"], select {
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 13px;
    width: 100%;
    background: var(--bg-input);
    color: var(--text-primary);
    box-sizing: border-box;
  }
  .meta { font-size: 12px; color: var(--text-muted); margin: 4px 0 10px 0; }
  .dirty { color: #FCD34D; }

  .tool-section {
    padding-top: 10px;
    margin-top: 4px;
    border-top: 1px solid var(--border);
  }
  .tool-head {
    font-size: 11px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
    margin-bottom: 6px;
  }
  .tool-radio {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .tool-radio label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--text-primary);
    cursor: pointer;
    padding: 4px 6px;
    border-radius: 4px;
  }
  .tool-radio label.active {
    background: var(--bg-hover);
    color: var(--accent);
    font-weight: 600;
  }
  .hint {
    font-size: 11px;
    color: var(--text-muted);
    margin: 4px 0 10px 6px;
    line-height: 1.4;
  }

  .btn-row {
    display: flex;
    gap: 6px;
    margin-top: 8px;
  }
  .btn-row button {
    flex-grow: 1;
    padding: 6px 10px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 12px;
    cursor: pointer;
  }
  .btn-row button:hover { background: var(--bg-hover); }
  .btn-row .primary {
    background: var(--accent);
    color: #FFFFFF;
    border-color: var(--accent);
    font-weight: 600;
  }
  .btn-row .export {
    background: transparent;
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }
  .btn-row .export:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .saved-section {
    margin-top: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .saved-head {
    font-size: 11px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
    margin-bottom: 6px;
  }
  .trash {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-secondary);
    padding: 5px 8px;
    border-radius: 4px;
    cursor: pointer;
  }
  .trash:hover { color: #FCA5A5; border-color: #B91C1C; }
</style>
