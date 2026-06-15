<script lang="ts">
  // Confirmation dialog fired when the user tries to navigate away from the
  // Build tab (tab switch, deck load, "New deck") with unsaved changes. Three
  // outcomes: Save (persists and proceeds), Discard (drops changes and
  // proceeds), Cancel (aborts the navigation).

  interface Props {
    open:      boolean;
    onSave:    () => void;
    onDiscard: () => void;
    onCancel:  () => void;
  }

  let { open, onSave, onDiscard, onCancel }: Props = $props();
</script>

{#if open}
  <div class="backdrop" onclick={onCancel} onkeydown={(e) => { if (e.key === "Escape") onCancel(); }} role="button" tabindex="-1" aria-label="Cancel"></div>
  <div class="modal" role="dialog" aria-modal="true" aria-label="Unsaved changes">
    <header>
      <h3>Unsaved changes</h3>
    </header>
    <p>
      You have unsaved changes to the current deck. Save before leaving, or
      discard them?
    </p>
    <footer>
      <button type="button" class="cancel" onclick={onCancel}>Cancel</button>
      <button type="button" class="discard" onclick={onDiscard}>Discard</button>
      <button type="button" class="save" onclick={onSave}>Save</button>
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
    width: min(420px, 90vw);
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
  p { margin: 0; font-size: 13px; color: var(--text-secondary); line-height: 1.45; }
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
  .cancel  { background: var(--bg-input);   color: var(--text-primary); }
  .discard { background: transparent;       color: #FCA5A5; border-color: #B91C1C; }
  .save    { background: var(--accent);     color: #FFFFFF; border-color: var(--accent); font-weight: 600; }
</style>
