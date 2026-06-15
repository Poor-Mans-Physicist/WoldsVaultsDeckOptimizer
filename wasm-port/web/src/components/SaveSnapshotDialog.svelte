<script lang="ts">
  // Modal that fires when the user clicks "Save snapshot" next to Run. Label
  // input pre-fills with a sensible default (date + deck + class). Save
  // persists immediately and closes; Cancel discards the action.

  interface Props {
    open:           boolean;
    defaultLabel:   string;
    onConfirm:      (label: string) => void;
    onCancel:       () => void;
  }
  let { open, defaultLabel, onConfirm, onCancel }: Props = $props();

  // Local copy of the input so we can mutate freely without parent ↔ child
  // sync. Reset whenever the dialog reopens.
  let value = $state("");
  let lastDefault = $state("");
  $effect(() => {
    if (open && defaultLabel !== lastDefault) {
      value = defaultLabel;
      lastDefault = defaultLabel;
    }
  });

  function confirm() {
    const trimmed = value.trim();
    if (trimmed.length === 0) return;
    onConfirm(trimmed);
  }
</script>

{#if open}
  <div class="backdrop" onclick={onCancel} onkeydown={(e) => { if (e.key === "Escape") onCancel(); }} role="button" tabindex="-1" aria-label="Cancel"></div>
  <div class="modal" role="dialog" aria-modal="true" aria-label="Save snapshot">
    <header>
      <h3>Save snapshot</h3>
    </header>
    <p>
      Captures the current deck, inputs, cores, and the SA result. Snapshots are
      mode-locked (loading later auto-switches mode if needed).
    </p>
    <div class="row">
      <label>
        Label
        <input
          type="text"
          bind:value={value}
          onkeydown={(e) => { if (e.key === "Enter") confirm(); }}
        />
      </label>
    </div>
    <footer>
      <button type="button" class="cancel" onclick={onCancel}>Cancel</button>
      <button type="button" class="save" onclick={confirm} disabled={value.trim().length === 0}>
        Save snapshot
      </button>
    </footer>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed; inset: 0;
    background: rgba(0,0,0,0.45);
    z-index: 100;
    border: 0;
    padding: 0;
  }
  .modal {
    position: fixed;
    top: 50%; left: 50%;
    transform: translate(-50%, -50%);
    width: min(480px, 90vw);
    z-index: 101;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px;
    color: var(--text-primary);
  }
  header h3 {
    margin: 0 0 10px 0;
    font-size: 14px;
    font-weight: 600;
  }
  p { margin: 0 0 12px 0; font-size: 13px; color: var(--text-secondary); line-height: 1.45; }
  .row label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-secondary);
  }
  input[type="text"] {
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 13px;
    width: 100%;
    background: var(--bg-input);
    color: var(--text-primary);
    box-sizing: border-box;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
  }
  button {
    padding: 6px 14px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
    border: 1px solid var(--border);
  }
  .cancel  { background: var(--bg-input); color: var(--text-primary); }
  .save    { background: var(--accent);   color: #FFFFFF; border-color: var(--accent); font-weight: 600; }
  .save:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
