<script lang="ts">
  // Exact-mode side panel (spec §9.3/§9.4): + Add Card opens the builder;
  // the inventory shows stacked identical cards (×N) with count editing,
  // must-place flags, and named profiles (save/load/delete + file
  // export/import via localStorage).

  import { app, clearRunResult } from "../lib/state.svelte";
  import {
    listProfiles, saveProfile, loadProfile, deleteProfile,
    exportProfile, importProfile, stackIdentity,
    type ExactProfile,
  } from "../lib/exactProfiles";
  import { COLOR_HEX, TYPE_LABEL } from "../lib/palette";
  import { NOTCH_COLOR, sortTags } from "../lib/notches";
  import type { ExactStack } from "../lib/types";

  interface Props {
    onOpenBuilder: () => void;
  }
  let { onOpenBuilder }: Props = $props();

  let profiles: ExactProfile[] = $state(listProfiles());
  let profileName = $state("");
  let selectedProfile = $state("");
  let fileInput: HTMLInputElement | undefined = $state();

  // Complex OFF: mismatched-scale cards are greyed out and ignored (§7).
  const usable = (s: ExactStack) => app.complexCards || s.scaleColor === s.color;

  const totalCards = $derived(
    app.exactStacks.filter(usable).reduce((a, s) => a + s.count, 0),
  );

  export function addStack(stack: ExactStack): void {
    // Stack onto an identical entry when one exists (spec: duplicates show ×N).
    const id = stackIdentity(stack);
    const existing = app.exactStacks.find((s) => stackIdentity(s) === id);
    if (existing) existing.count += stack.count;
    else app.exactStacks.push(stack);
    clearRunResult();
  }

  function setCount(s: ExactStack, e: Event) {
    const n = Math.floor(Number((e.currentTarget as HTMLInputElement).value));
    s.count = Number.isFinite(n) && n > 0 ? n : 1;
    clearRunResult();
  }
  function remove(s: ExactStack) {
    app.exactStacks = app.exactStacks.filter((x) => x !== s);
    clearRunResult();
  }
  function toggleMust(s: ExactStack) {
    s.mustPlace = !s.mustPlace;
    clearRunResult();
  }
  function clearAll() {
    app.exactStacks = [];
    clearRunResult();
  }

  // ── Profiles ────────────────────────────────────────────────────────────
  function doSave() {
    const name = profileName.trim() || `profile ${profiles.length + 1}`;
    saveProfile(name, $state.snapshot(app.exactStacks) as ExactStack[]);
    profiles = listProfiles();
    selectedProfile = name;
    profileName = "";
  }
  function doLoad() {
    if (!selectedProfile) return;
    const stacks = loadProfile(selectedProfile);
    if (stacks === null) {
      console.error(`[exact] profile '${selectedProfile}' missing — list may be stale`);
      profiles = listProfiles();
      return;
    }
    app.exactStacks = stacks;
    clearRunResult();
  }
  function doDelete() {
    if (!selectedProfile) return;
    deleteProfile(selectedProfile);
    profiles = listProfiles();
    selectedProfile = "";
  }
  function doExport() {
    const name = selectedProfile || "inventory";
    const blob = new Blob(
      [exportProfile(name, $state.snapshot(app.exactStacks) as ExactStack[])],
      { type: "application/json" },
    );
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `deckfast-${name}.json`;
    a.click();
    URL.revokeObjectURL(a.href);
  }
  async function doImport(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    try {
      const { name, stacks } = importProfile(await file.text());
      app.exactStacks = stacks;
      saveProfile(name, stacks);
      profiles = listProfiles();
      selectedProfile = name;
      clearRunResult();
    } catch (err) {
      console.error("[exact] import failed:", err);
      alert(err instanceof Error ? err.message : String(err));
    } finally {
      (e.currentTarget as HTMLInputElement).value = "";
    }
  }
</script>

<section class="card">
  <div class="head">
    <h3>Your cards</h3>
    <span class="meta">{totalCards} usable</span>
  </div>

  <button type="button" class="add" onclick={onOpenBuilder}>+ Add Card</button>

  {#if app.exactStacks.length === 0}
    <div class="empty">No cards yet — build the exact cards you own.</div>
  {:else}
    <div class="stacks">
      {#each app.exactStacks as s (stackIdentity(s) + s.count + s.mustPlace)}
        <div class="stack" class:ignored={!usable(s)}
          title={!usable(s) ? "scale color ≠ card color — ignored while Complex Cards is off" : ""}>
          <span class="cdot" style:background={COLOR_HEX[s.color]}></span>
          <span class="tname">
            {TYPE_LABEL[s.t] ?? s.t}
            {#if s.scaleColor !== s.color}
              <span class="scale">→{s.scaleColor}</span>
            {/if}
          </span>
          <span class="tags">
            {#each sortTags(s.groups) as g}
              <span class="notch" style:background={NOTCH_COLOR[g]} title={g}></span>
            {/each}
          </span>
          <input class="count" type="number" min="1" value={s.count}
            oninput={(e) => setCount(s, e)} />
          <button type="button" class="pin" class:on={s.mustPlace}
            title="Must place all of this stack" onclick={() => toggleMust(s)}>📌</button>
          <button type="button" class="del" title="Remove stack"
            onclick={() => remove(s)}>✕</button>
        </div>
      {/each}
    </div>
    <button type="button" class="mini danger" onclick={clearAll}>Clear inventory</button>
  {/if}

  <div class="profiles">
    <div class="section">Profiles</div>
    <div class="prow">
      <select bind:value={selectedProfile}>
        <option value="">— select —</option>
        {#each profiles as p}
          <option value={p.name}>{p.name} ({p.stacks.length})</option>
        {/each}
      </select>
      <button type="button" class="mini" onclick={doLoad} disabled={!selectedProfile}>Load</button>
      <button type="button" class="mini danger" onclick={doDelete} disabled={!selectedProfile}>✕</button>
    </div>
    <div class="prow">
      <input type="text" placeholder="save as…" bind:value={profileName} />
      <button type="button" class="mini" onclick={doSave}
        disabled={app.exactStacks.length === 0 && !profileName.trim()}>Save</button>
    </div>
    <div class="prow">
      <button type="button" class="mini" onclick={doExport}
        disabled={app.exactStacks.length === 0}>Export file</button>
      <button type="button" class="mini" onclick={() => fileInput?.click()}>Import file</button>
      <input type="file" accept=".json,application/json" bind:this={fileInput}
        style="display:none" onchange={doImport} />
    </div>
  </div>
</section>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 12px;
  }
  .head { display: flex; justify-content: space-between; align-items: baseline; }
  h3 {
    margin: 0;
    font-size: 12px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  .meta { font-size: 11px; color: var(--text-muted); }
  .add {
    width: 100%;
    margin: 8px 0;
    padding: 7px 0;
    background: var(--accent);
    color: #fff;
    border: 0;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .empty { font-size: 12px; color: var(--text-muted); padding: 6px 0; }
  .stacks { display: flex; flex-direction: column; gap: 4px; max-height: 44vh; overflow-y: auto; }
  .stack {
    display: grid;
    grid-template-columns: 10px 1fr auto 52px 26px 22px;
    gap: 6px;
    align-items: center;
    padding: 3px 4px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-input);
  }
  .stack.ignored { opacity: .45; }
  .cdot { width: 10px; height: 10px; border-radius: 50%; border: 1px solid rgba(0,0,0,.3); }
  .tname { font-size: 12px; color: var(--text-primary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .scale { color: var(--text-muted); font-size: 11px; }
  .tags { display: flex; gap: 2px; }
  .notch { width: 7px; height: 10px; border-radius: 2px; border: 1px solid rgba(0,0,0,.35); }
  .count {
    width: 100%;
    padding: 2px 4px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-card);
    color: var(--text-primary);
    font-size: 12px;
  }
  .pin, .del {
    background: transparent;
    border: 0;
    cursor: pointer;
    font-size: 12px;
    color: var(--text-muted);
    padding: 0;
  }
  .pin.on { filter: none; }
  .pin:not(.on) { filter: grayscale(1); opacity: .5; }
  .del:hover { color: #FCA5A5; }
  .mini {
    font-size: 11px;
    padding: 3px 8px;
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text-secondary);
    cursor: pointer;
    margin-top: 6px;
  }
  .mini:hover:not(:disabled) { color: var(--text-primary); border-color: var(--accent); }
  .mini:disabled { opacity: .4; cursor: default; }
  .mini.danger:hover:not(:disabled) { color: #FCA5A5; border-color: #B91C1C; }
  .profiles { margin-top: 10px; }
  .section {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    padding-bottom: 2px;
    margin-bottom: 6px;
  }
  .prow { display: flex; gap: 6px; align-items: center; margin-bottom: 4px; }
  .prow .mini { margin-top: 0; }
  select, input[type="text"] {
    flex: 1;
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 12px;
    min-width: 0;
  }
</style>
