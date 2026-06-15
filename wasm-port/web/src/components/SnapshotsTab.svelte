<script lang="ts">
  // Snapshots browser — full-width table of saved Run captures. Each row's
  // Load button hands the snapshot back to the parent (App.svelte) so it can
  // run the mode-switch + unsaved-builder guard before restoring.

  import { app, deleteSnapshotById } from "../lib/state.svelte";
  import type { Snapshot } from "../lib/snapshots";

  interface Props {
    onLoad: (snap: Snapshot) => void;
  }
  let { onLoad }: Props = $props();

  function fmtDate(t: number): string {
    const d = new Date(t);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }

  function onDelete(snap: Snapshot) {
    if (!confirm(`Delete snapshot "${snap.label}"?`)) return;
    deleteSnapshotById(snap.id);
  }
</script>

<section class="card">
  <header class="head">
    <h2>Snapshots</h2>
    <span class="count">{app.snapshots.length} saved</span>
  </header>

  {#if app.snapshots.length === 0}
    <div class="empty">
      <p>No snapshots yet.</p>
      <p class="hint">
        Run the optimizer, then click <strong>Save snapshot</strong> next to
        the Run button to capture the deck, inputs, cores, and result.
        Snapshots are mode-locked and stored in your browser only.
      </p>
    </div>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Label</th>
            <th>Deck</th>
            <th>Mode</th>
            <th>Class</th>
            <th class="num">NDM</th>
            <th>Saved</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each app.snapshots as s (s.id)}
            <tr>
              <td class="label">
                {s.label}
                {#if s.deck.isBuiltDeck}<span class="badge">built</span>{/if}
              </td>
              <td>{s.deck.name}</td>
              <td class="mode">{s.mode}</td>
              <td class="cls">{s.cardClass}</td>
              <td class="num"><strong>{s.wasmScore.toFixed(2)}</strong></td>
              <td class="date">{fmtDate(s.createdAt)}</td>
              <td class="actions">
                <button type="button" class="load" onclick={() => onLoad(s)}>Load</button>
                <button type="button" class="trash" onclick={() => onDelete(s)} title="Delete snapshot">🗑</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 16px;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 14px;
  }
  .head h2 { margin: 0; font-size: 16px; font-weight: 600; color: var(--text-primary); }
  .count { font-size: 12px; color: var(--text-muted); }

  .empty {
    padding: 20px;
    text-align: center;
    color: var(--text-secondary);
  }
  .empty p { margin: 0 0 6px 0; }
  .empty .hint {
    font-size: 12px;
    color: var(--text-muted);
    max-width: 540px;
    margin: 10px auto 0;
    line-height: 1.55;
  }
  .empty strong { color: var(--text-primary); }

  .table-wrap { overflow-x: auto; }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th, td {
    text-align: left;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  th {
    font-size: 11px;
    text-transform: uppercase;
    color: var(--text-secondary);
    font-weight: 600;
    letter-spacing: 0.04em;
  }
  td.label { font-weight: 500; }
  td.mode, td.cls { color: var(--text-secondary); font-family: 'JetBrains Mono', monospace; font-size: 12px; }
  td.num { text-align: right; font-family: 'JetBrains Mono', monospace; }
  td.date { color: var(--text-muted); font-size: 12px; }
  td.actions { white-space: nowrap; text-align: right; }
  th.num { text-align: right; }

  .badge {
    display: inline-block;
    margin-left: 6px;
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 600;
    background: rgba(99,102,241,0.18);
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  button {
    padding: 4px 10px;
    border-radius: 4px;
    border: 1px solid var(--border);
    cursor: pointer;
    font-size: 12px;
    margin-left: 4px;
  }
  .load { background: var(--accent); color: #FFFFFF; border-color: var(--accent); font-weight: 600; }
  .trash { background: transparent; color: var(--text-secondary); }
  .trash:hover { color: #FCA5A5; border-color: #B91C1C; }
</style>
