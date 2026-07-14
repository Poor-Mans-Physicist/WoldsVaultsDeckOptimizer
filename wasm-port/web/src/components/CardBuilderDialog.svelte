<script lang="ts">
  // Exact-mode card builder popup (spec §9.4): color → type → tags
  // (+ scale color under Complex Cards) → batch count. Foil rules are
  // enforced here: Wold's shiny cards are always foil (locked on); evo
  // cards take foil only from the Shiny core (internal key `foil`), so the
  // builder locks it off except where legal.

  import {
    CardClass, CardType, Color, ALL_COLORS, CATEGORY_GROUPS, REAL_GREEDS,
    type ExactStack, type GroupTag,
  } from "../lib/types";
  import { COLOR_HEX, TYPE_LABEL } from "../lib/palette";
  import { NOTCH_COLOR } from "../lib/notches";
  import { isLegalCategorySet } from "../lib/implicits";
  import { legalTagCombos } from "../lib/deck";

  interface Props {
    open: boolean;
    cardClass: CardClass;
    appMode: string;             // "wolds" | "vanilla"
    complexCards: boolean;
    allowDeluxe: boolean;
    shinyPositional: boolean;
    hasArcaneSlots: boolean;
    onAdd: (stack: ExactStack) => void;
    onClose: () => void;
  }
  let {
    open, cardClass, appMode, complexCards, allowDeluxe, shinyPositional,
    hasArcaneSlots, onAdd, onClose,
  }: Props = $props();

  let color = $state<Color>(Color.RED);
  let scaleColor = $state<Color>(Color.RED);
  let ctype = $state<CardType>(CardType.ROW);
  let groups = $state<GroupTag[]>([]);
  let count = $state(1);

  const typeChoices = $derived.by<CardType[]>(() => {
    const out: CardType[] = [];
    if (!(cardClass === CardClass.SHINY && !shinyPositional)) {
      out.push(CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG);
    }
    out.push(CardType.TYPELESS);
    if (allowDeluxe) out.push(CardType.DELUXE);
    out.push(...REAL_GREEDS);
    if (hasArcaneSlots) out.push(CardType.ARCANE);
    out.push(CardType.WILD);
    return out;
  });

  const isGreed = $derived((REAL_GREEDS as readonly CardType[]).includes(ctype));
  const isWild = $derived(ctype === CardType.WILD);
  const isArcane = $derived(ctype === CardType.ARCANE);
  const isScorableOrArcane = $derived(!isGreed && !isWild);
  // Arcane cards carry no tags at all (playtest ruling); greed likewise.
  const tagsAllowed = $derived(!isGreed && !isWild && !isArcane);

  /** Would adding `g` create a category set no real card has? (subset rule
   *  over the game-data combo catalog; Wild exempt — no chips shown.) */
  function comboIllegal(g: GroupTag): boolean {
    if (groups.includes(g)) return false;   // removal is always fine
    return !isLegalCategorySet([...groups, g], legalTagCombos(appMode));
  }

  // Foil legality (§5): Wold's shiny ⇒ locked ON for scorable/arcane cards;
  // greed cards carry no groups; vanilla / evo cards are never foil at build
  // time (evo foil comes from the Shiny core at run time).
  // Stat is NOT offered — it's run-derived (shiny ⇒ stat cards carry it,
  // evo ⇒ never); the kernel adds it automatically.
  const foilLocked = $derived(
    appMode !== "vanilla" && cardClass === CardClass.SHINY && !isGreed && !isWild && !isArcane,
  );

  function toggleGroup(g: GroupTag) {
    if (groups.includes(g)) groups = groups.filter((x) => x !== g);
    else groups = [...groups, g];
  }

  function effectiveGroups(): GroupTag[] {
    if (isGreed || isWild || isArcane) return [];
    let out: GroupTag[] = groups.filter((g) => g !== "Foil" && g !== "Stat");
    if (foilLocked || groups.includes("Foil")) {
      if (foilLocked || cardClass !== CardClass.EVO) out = [...out, "Foil"];
    }
    return out;
  }

  function add() {
    const n = Math.max(1, Math.floor(count) || 1);
    onAdd({
      t: ctype,
      color,
      scaleColor: complexCards ? scaleColor : color,
      groups: effectiveGroups(),
      count: n,
      mustPlace: false,
    });
    onClose();
  }
</script>

{#if open}
  <div class="overlay" role="presentation" onclick={onClose}>
    <div class="dialog" role="dialog" aria-label="Build a card"
      onclick={(e) => e.stopPropagation()}>
      <h3>Build cards</h3>

      <div class="field">
        <span class="lbl">Color</span>
        <div class="chips">
          {#each ALL_COLORS as c}
            <button type="button" class="chip" class:sel={color === c}
              style:--chip={COLOR_HEX[c]} onclick={() => (color = c)}>
              {c}
            </button>
          {/each}
        </div>
      </div>

      {#if complexCards && (isGreed || (isScorableOrArcane && ctype !== CardType.TYPELESS && ctype !== CardType.DELUXE && ctype !== CardType.ARCANE))}
        <div class="field">
          <span class="lbl">Scale color <em>(boosts / scales off)</em></span>
          <div class="chips">
            {#each ALL_COLORS as c}
              <button type="button" class="chip" class:sel={scaleColor === c}
                style:--chip={COLOR_HEX[c]} onclick={() => (scaleColor = c)}>
                {c}
              </button>
            {/each}
          </div>
        </div>
      {/if}

      <div class="field">
        <span class="lbl">Type</span>
        <div class="chips wrap">
          {#each typeChoices as t}
            <button type="button" class="chip plain" class:sel={ctype === t}
              onclick={() => (ctype = t)}>
              {TYPE_LABEL[t] ?? t}
            </button>
          {/each}
        </div>
      </div>

      {#if tagsAllowed}
        <div class="field">
          <span class="lbl">Tags</span>
          <div class="chips wrap">
            {#each CATEGORY_GROUPS as g}
              <button type="button" class="chip plain notch"
                class:sel={groups.includes(g)}
                disabled={comboIllegal(g)}
                title={comboIllegal(g)
                  ? `No real card combines ${[...groups.filter((x) => x !== "Foil"), g].join(" + ")}`
                  : g}
                style:--chip={NOTCH_COLOR[g]}
                onclick={() => toggleGroup(g)}>
                {g}
              </button>
            {/each}
            <button type="button" class="chip plain notch"
              class:sel={foilLocked || groups.includes("Foil")}
              disabled={foilLocked || (cardClass === CardClass.EVO)}
              title={foilLocked
                ? "Wold's shiny cards are always foil"
                : cardClass === CardClass.EVO
                  ? "Evo cards are foil only via the Shiny core at run time"
                  : "Foil"}
              style:--chip={NOTCH_COLOR.Foil}
              onclick={() => toggleGroup("Foil")}>
              Foil {foilLocked ? "🔒" : ""}
            </button>
          </div>
        </div>
      {:else}
        <div class="note">
          {isWild
            ? "Wild carries every group and matches every color for neighbors — no tags to pick."
            : isArcane
              ? "Arcane cards carry no tags (and are never foil)."
              : "Greed cards carry no tags."}
        </div>
      {/if}

      <div class="field">
        <span class="lbl">Count (batch add)</span>
        <input type="number" min="1" step="1" bind:value={count} />
      </div>

      <div class="actions">
        <button type="button" class="primary" onclick={add}>Add ×{Math.max(1, Math.floor(count) || 1)}</button>
        <button type="button" onclick={onClose}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed; inset: 0;
    background: rgba(0,0,0,.55);
    display: flex; align-items: center; justify-content: center;
    z-index: 50;
  }
  .dialog {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px;
    width: 400px;
    max-height: 84vh;
    overflow-y: auto;
  }
  h3 { margin: 0 0 10px; font-size: 14px; color: var(--text-primary); }
  .field { margin-bottom: 10px; }
  .lbl {
    display: block;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
    margin-bottom: 4px;
  }
  .lbl em { text-transform: none; color: var(--text-muted); }
  .chips { display: flex; gap: 6px; }
  .chips.wrap { flex-wrap: wrap; }
  .chip {
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 12px;
    cursor: pointer;
    position: relative;
  }
  .chip:not(.plain)::before {
    content: "";
    display: inline-block;
    width: 8px; height: 8px;
    border-radius: 50%;
    background: var(--chip);
    margin-right: 5px;
  }
  .chip.notch::before {
    content: "";
    display: inline-block;
    width: 8px; height: 8px;
    border-radius: 2px;
    background: var(--chip);
    border: 1px solid rgba(0,0,0,.3);
    margin-right: 5px;
  }
  .chip.sel { border-color: var(--accent); background: var(--bg-hover); font-weight: 600; }
  .chip:disabled { opacity: .55; cursor: not-allowed; }
  .note { font-size: 12px; color: var(--text-muted); margin-bottom: 10px; }
  input[type="number"] {
    width: 100%;
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 13px;
  }
  .actions { display: flex; gap: 8px; margin-top: 12px; }
  .actions button {
    flex: 1;
    padding: 7px 0;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 13px;
    cursor: pointer;
  }
  .actions .primary { background: var(--accent); border-color: var(--accent); color: #fff; font-weight: 600; }
</style>
