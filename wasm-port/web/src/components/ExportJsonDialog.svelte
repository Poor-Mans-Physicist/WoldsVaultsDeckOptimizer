<script lang="ts">
  // Modal popup that surfaces the modpack-JSON entry for the current built
  // deck. Read-only textarea + Copy-to-Clipboard button — there's no edit
  // round-trip back into the builder; the source of truth is the canvas.

  interface Props {
    open:    boolean;
    json:    string;          // pre-formatted entry, including trailing newline
    onClose: () => void;
  }

  let { open, json, onClose }: Props = $props();

  let copied = $state(false);

  async function copyJson() {
    try {
      await navigator.clipboard.writeText(json);
      copied = true;
      // Brief reset so the user can re-trigger if they need to.
      setTimeout(() => { copied = false; }, 1500);
    } catch (e) {
      // Browsers without clipboard access (rare): leave the textarea up so the
      // user can manually select + Ctrl+C. Log to help diagnose.
      console.error("[ExportJsonDialog] clipboard write failed", e);
    }
  }
</script>

{#if open}
  <div class="backdrop" onclick={onClose} onkeydown={(e) => { if (e.key === "Escape") onClose(); }} role="button" tabindex="-1" aria-label="Close"></div>
  <div class="modal" role="dialog" aria-modal="true" aria-label="Export deck JSON">
    <header>
      <h3>Modpack JSON entry</h3>
      <button type="button" class="close" onclick={onClose} aria-label="Close">×</button>
    </header>
    <p class="hint">
      Drop this into the <code>values:</code> block of a Wold's Vaults deck-data
      file. Defaults: <code>essence 5/5</code>, <code>weight 1.0</code>,
      <code>model</code> derived from the deck name.
    </p>
    <textarea readonly>{json}</textarea>
    <footer>
      <button type="button" class="copy" onclick={copyJson}>
        {copied ? "Copied!" : "Copy to clipboard"}
      </button>
      <button type="button" class="close-btn" onclick={onClose}>Close</button>
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
    width: min(640px, 90vw);
    max-height: 80vh;
    z-index: 101;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    color: var(--text-primary);
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid var(--border);
    padding-bottom: 8px;
  }
  header h3 { margin: 0; font-size: 14px; font-weight: 600; }
  .close {
    background: transparent;
    border: 0;
    color: var(--text-secondary);
    font-size: 20px;
    cursor: pointer;
    line-height: 1;
    padding: 0 4px;
  }
  .close:hover { color: var(--text-primary); }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.45;
  }
  .hint code {
    background: var(--bg-input);
    padding: 1px 4px;
    border-radius: 3px;
    font-size: 11px;
    color: var(--text-primary);
  }
  textarea {
    width: 100%;
    min-height: 260px;
    font-family: 'JetBrains Mono', 'Consolas', monospace;
    font-size: 12px;
    line-height: 1.4;
    padding: 10px;
    background: var(--bg-input);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: 4px;
    resize: vertical;
    box-sizing: border-box;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .copy, .close-btn {
    padding: 6px 14px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
    border: 1px solid var(--border);
  }
  .copy {
    background: var(--accent);
    color: #FFFFFF;
    border-color: var(--accent);
    font-weight: 600;
  }
  .close-btn {
    background: var(--bg-input);
    color: var(--text-primary);
  }
</style>
