<script lang="ts">
  interface Props {
    mode:    string;
    modes:   string[];
    onChange?: (next: string) => void;
  }
  let { mode = $bindable(), modes, onChange }: Props = $props();

  function pick(next: string) {
    if (next === mode) return;
    mode = next;
    onChange?.(next);
  }
</script>

<div class="seg">
  {#each modes as m}
    <button
      type="button"
      class="seg-btn"
      class:active={m === mode}
      onclick={() => pick(m)}
    >
      {m}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    background: var(--bg-card);
  }
  .seg-btn {
    background: transparent;
    border: 0;
    padding: 6px 14px;
    font-size: 13px;
    cursor: pointer;
    text-transform: capitalize;
    color: var(--text-secondary);
  }
  .seg-btn + .seg-btn { border-left: 1px solid var(--border); }
  .seg-btn:hover { background: var(--bg-hover); color: var(--text-primary); }
  .seg-btn.active {
    background: var(--accent);
    color: #FFFFFF;
  }
</style>
