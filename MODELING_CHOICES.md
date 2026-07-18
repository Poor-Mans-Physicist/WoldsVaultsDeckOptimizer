# Modeling Choices

This file is the source-of-truth specification for **how the Vault Hunters
Deck Optimizer scores a deck**. It pins down every multiplier, every
class-gating rule, every counting rule for cores and cards.

> **Optimizer 2.0:** the tag-aware kernel (`ndm_core/src/tagsim.rs`, shared
> verbatim with the wasm crate) now backs both channels. Everything below
> still holds — 2.0's Max mode reproduces it bit-for-bit (see the parity
> gate) — and the new mechanics are specified in the
> **[Optimizer 2.0 addendum](#optimizer-20-addendum)** at the bottom.

Authoritative behavior described here = the **WASM web app**
(`wasm-port/web/` + `wasm-port/ndm_core/src/inventory.rs`). The Python
spreadsheet CLI (outer `src/` + outer `ndm_core/`) tracks the same model
except where called out under **Platform discrepancies** at the bottom.
Anything not under that section is identical across both channels.

> The desktop NiceGUI inventory tool referenced in earlier revisions of
> this doc was deleted in the channel-consolidation refactor. Any
> "NiceGUI" callouts below are stale and apply only if a future caller
> resurrects an inventory-aware Python entry point.

**This document is a maintenance contract.** When you change scoring
logic, multiplier values, `n_ns` rules, core gating, greed mechanics,
constraints, slot rules, or stacking modes anywhere in the code, update
this file in the same commit. See `CLAUDE.md` for the rule.

---

## Reading this document

- **Card types** in code: enum `CardType` (`src/types.py`,
  `wasm-port/web/src/lib/types.ts`). String values must match exactly
  between Python, TypeScript, and the Rust `card_type_from_str` matcher
  (`ndm_core/src/lib.rs` + `wasm-port/ndm_core/src/inventory.rs`).
- **Core types**: enum `CoreType`, same multi-language sync requirement.
- **Default multiplier values** below are taken from the top of
  `config.yaml`; vanilla overrides them via `modes.vanilla` (see the
  **Wold's vs Vanilla** table).
- **NDM** = "Net Deck Multiplier". It's the sum of every scorable card's
  contribution, where one card's contribution =
  `base_value × greed_boost × core_multiplier`. See **How a run scores**.

---

## How a run scores (overview)

A "run" picks a `(card_class, cores)` pair plus a placement of cards into
deck slots, then computes one NDM value.

1. **Card classifier.** Walk the placement and bucket every placed card
   into one of: `greed`, `regular` (positionals), `deluxe`, `typeless`,
   `arcane`. `DEAD` cards are counted via `n_dead` separately.
2. **`n_ns` for Pure-core.** Computed from the bucket sizes per the
   class+FOIL rule below.
3. **Build the per-card-type core multipliers.** Three variants exist —
   `regular_core_mult`, `deluxe_card_core_mult`, `typeless_core_mult` —
   because DELUXE_CORE skips deluxe cards.
4. **Compute `archive_mult`** — a single deck-wide factor of
   `archive_core_base ^ (2.1·√n_arcane_placed)` when ARCHIVE_CORE is
   picked, else `1.0`. Bypasses the additive-vs-multiplicative stacking
   switch entirely; applied as a final outside-the-stack factor on every
   scoring card. See **Archive core** below.
5. **Greed boosts.** Every greed card applies its target-specific boost
   to a `boost: position → float` map (only scorable targets receive it).
6. **Sum NDM per category:**
   - regular: `pos_count × regular_core_mult × boost × archive_mult`
   - deluxe:  `MULT_DELUXE_FLAT × deluxe_card_core_mult × boost × archive_mult`
   - typeless: `1.0 × typeless_core_mult × boost × archive_mult`
   - arcane: 0 (never scores directly, but counts in row/col peer
     counts for adjacent positionals, counts in `n_ns`, and counts in
     ARCHIVE's exponent).
   - dead: 0 (consumed by void core only).

---

## Slot types

The deck layout grid (in `decks/*.json` `value:` strings or `decks/*.yaml`
`layout:`) uses single chars:

| Char  | Slot type          | What can be placed                                      |
| ----- | ------------------ | ------------------------------------------------------- |
| `O` | Regular            | Any placeable card except `ARCANE`                    |
| `A` | Arcane             | Only `ARCANE` or `DEAD` cards                       |
| `X` | Empty / not-a-slot | Nothing — this position is not part of the deck at all |

`arcane_slots` are tracked as a sub-set of `slots`; geometry (row/col/surr/diag
peer sets) is computed over the **full** slot set so arcane neighbors still
count for positional peer scans.

---

## Card types — quick reference

| Type                       | Base NDM contribution                                  | Scoring source                                       | Class                       | Counts in `n_ns`?                      |
| -------------------------- | ------------------------------------------------------ | ---------------------------------------------------- | --------------------------- | ---------------------------------------- |
| `ROW`                    | `pos_count × core_mult × boost`                    | Same-row peer count (incl. self via `row_count`)   | EVO + SHINY-with-positional | EVO-no-FOIL only                         |
| `COL`                    | `pos_count × core_mult × boost`                    | Same-column peer count                               | EVO + SHINY-with-positional | EVO-no-FOIL only                         |
| `SURR`                   | `pos_count × core_mult × boost`                    | Count of filled 8-neighbors (excludes self)          | EVO + SHINY-with-positional | EVO-no-FOIL only                         |
| `DIAG`                   | `pos_count × core_mult × boost`                    | NW-SE plus NE-SW diagonal peer count (does NOT count self), clamped to a minimum of 1 so a lone DIAG card still contributes its base value | EVO + SHINY-with-positional | EVO-no-FOIL only                         |
| `DELUXE`                 | `MULT_DELUXE_FLAT × deluxe_card_core_mult × boost` | Flat base (config `deluxe.flat`, default 2)        | EVO + SHINY                 | **No** (rides DELUXE_CORE instead) |
| `TYPELESS` (T)           | `1.0 × typeless_core_mult × boost`                 | Always 1.0 base                                      | EVO + SHINY                 | **No** ("always shiny" by design)  |
| `ARCANE`                 | 0 (always)                                             | n/a — no direct NDM, no cores apply, no greed boost | EVO + SHINY                 | **Always** counts                  |
| `DEAD`                   | 0 (always)                                             | Consumed by VOID_CORE via `n_dead` count           | EVO + SHINY                 | No                                       |
| `FILLER_GREED`           | 0 — display-only marker, never placed by the SA       | n/a                                                  | n/a                         | No                                       |
| 10 GREED types (see below) | 0 directly (greed cards never score)                   | Boost neighbors only                                 | (varies — see below)       | **Always** count                   |

**Important** ARCANE rules:

- Placeable **only** in `A` slots, no exceptions.
- Always counts as "filled" for row/col/surr/diag peer counts of neighbors
  — so a `ROW` card next to an arcane sees the arcane in its row count.
- In the WASM (inventory) model, ARCANE participates in **same-color** peer
  counts; in the classic CLI model it counts color-blind. See discrepancies.

**Important** DEAD rules:

- Listed in `PLACEABLE` (`src/types.py`) — the SA *can* propose DEAD in
  **any** slot (regular `O` or arcane `A`); there is no slot-type
  restriction.
- Without VOID_CORE on the deck, the SA will not actually place DEAD
  because a dead slot strictly loses NDM (it contributes 0 and is not
  boosted by anything), and any other placement is strictly better.
- With VOID_CORE on, DEAD feeds `n_dead` and so the SA may choose to
  sacrifice slots to feed the void scaling.
- The arcane-auto-place=OFF toggle (web app) expands the inventory SA's
  per-arcane-slot proposal alphabet to include DEAD as well — useful
  when void is on. When auto-place is ON, arcane slots stay locked to
  ARCANE (with color-only swaps allowed).

---

## Greed cards — exact scaling

All ten greed types are non-scoring (their own base contribution is 0).
They modify a neighbor's `boost` value. The neighbor must be a **scorable**
card (in `regular`, `deluxe`, or `typeless`) — arcane / dead / empty / other
greed cards are skipped.

| Card type           | Target slot relative to greed (r, c)       | Multiplier source       | Default value | Notes                                                                                                         |
| ------------------- | ------------------------------------------ | ----------------------- | ------------- | ------------------------------------------------------------------------------------------------------------- |
| `DIR_GREED_UP`    | `(r-1, c)` directly above                | `greed.dir_vert`      | **4**   |                                                                                                               |
| `DIR_GREED_DOWN`  | `(r+1, c)` directly below                | `greed.dir_vert`      | **4**   |                                                                                                               |
| `DIR_GREED_LEFT`  | `(r, c-1)` directly left                 | `greed.dir_horiz`     | **4**   |                                                                                                               |
| `DIR_GREED_RIGHT` | `(r, c+1)` directly right                | `greed.dir_horiz`     | **4**   |                                                                                                               |
| `DIR_GREED_NE`    | `(r-1, c+1)`                             | `greed.dir_diag_up`   | **0**   | Diagonal greeds are inert at default 0                                                                        |
| `DIR_GREED_NW`    | `(r-1, c-1)`                             | `greed.dir_diag_up`   | **0**   |                                                                                                               |
| `DIR_GREED_SE`    | `(r+1, c+1)`                             | `greed.dir_diag_down` | **0**   |                                                                                                               |
| `DIR_GREED_SW`    | `(r+1, c-1)`                             | `greed.dir_diag_down` | **0**   |                                                                                                               |
| `EVO_GREED`       | `(r+1, c)` directly below                | `greed.evo`           | **0**   | **EVO-class-only**, and **only** if target is a regular positional (not typeless, deluxe, arcane) |
| `SURR_GREED`      | All 8 surrounding peers (within ≤ 1 step) | `greed.surr`          | **0**   | Applies to every scorable peer independently                                                                  |

### Greed stacking — additive vs multiplicative

Controlled by `stacking.greed_additive` (default **true** in both modes).

- **Additive** (`true`): the boost starts at **1.0** and each greed
  pointing at the slot adds its raw multiplier value. Final boost =
  `1.0 + Σ amount_i` over all greeds hitting the slot — so a no-greed
  slot stays at a neutral 1× and every greed adds on top of that base
  rather than replacing it.
  Worked examples (default `dir_vert: 4`):

  | Greeds pointing at slot | Final boost |
  | --- | --- |
  | 0                            | 1   |
  | 1× dir_vert                  | 5   |
  | 2× dir_vert                  | 9   |
  | 3× dir_vert                  | 13  |
  | 1× dir_vert + 1× surr_greed at 3 | 8 |

- **Multiplicative** (`false`): each greed multiplies the running boost
  starting from 1.0. Final boost = `Π amount_i`. **Not clamped** — if any
  contributing multiplier is 0, the slot's contribution becomes 0. This
  is a legacy stacking model; neither Wold's nor Vanilla uses it today.

Implementation: `_apply_greed()` in `src/simulate.py`, the `apply_greed!`
macro in `ndm_core/src/lib.rs`, the inline `apply` closure in
`ndm_core/src/inventory.rs` + `wasm-port/ndm_core/src/inventory.rs`, and
`applyGreed()` in `wasm-port/web/src/lib/breakdown.ts`. The boost map
is initialized to `1.0` at the start of every scoring call in both
stacking modes — additive accumulates additional greeds on top of that
base, multiplicative scales it.

---

## Cores

| Core            | Default value (Wold's)                  | What it boosts                                                          | What it does NOT boost                   | Scaling formula                                          | Class gating                                                                   |
| --------------- | --------------------------------------- | ----------------------------------------------------------------------- | ---------------------------------------- | -------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `PURE`        | base**1.0**, scale **0.07** | regulars, deluxe cards (additive baseline), typeless                    | greed                                    | `pure_base + pure_scale × n_ns` (variable per layout) | Universal                                                                      |
| `EQUILIBRIUM` | **1.5**                           | regulars, typeless                                                      | deluxe cards (additive), greed           | Flat                                                     | **SHINY-only**                                                           |
| `STEADFAST`   | **2.1**                           | regulars, typeless                                                      | deluxe cards, greed                      | Flat                                                     | **SHINY-only**                                                           |
| `SPARKLING`   | **2.5**                           | regulars, typeless                                                      | deluxe cards, greed                      | Flat                                                     | **SHINY-only**; gated by `cores.sparkling_allow` (off in vanilla)        |
| `COLOR`       | **1.75**                          | every scorable card (WASM model: only matching-color cards)             | greed                                    | Flat                                                     | Universal                                                                      |
| `FOIL`        | **2.5**                           | regulars, deluxe cards (baseline), typeless                             | greed                                    | Flat                                                     | Universal;**also flips EVO's `n_ns` to the SHINY formula** (see below). Displayed as **"Shiny"** in the UI — the in-game name (`foil` stays the internal key everywhere) |
| `DELUXE_CORE` | base**1.0**, scale **0.2**  | regulars, typeless                                                      | **deluxe cards themselves**, greed | `deluxe_core_base + deluxe_core_scale × n_deluxe`     | Universal; gated by `deluxe.allow` (off in vanilla)                          |
| `VOID_CORE`   | base**1.0**, scale **0.3**  | regulars, deluxe cards, typeless                                        | dead cards themselves, greed             | `void_base + void_scale × n_dead`                     | Universal; gated by `cores.void_allow` (off in vanilla)                      |
| `ARCHIVE_CORE` | rolled base **1.2**           | regulars, deluxe cards, typeless                                        | greed (arcane/dead score 0 anyway)       | `archive_core ^ (2.1·√n_arcane_placed)` — applied **outside** the per-card `core_mult` (see callout below) | Gated by `cores.archive_allow` (off in vanilla); when on, additionally **enumerated only when the deck has ≥ 1 arcane slot** |

Cores **never** apply to greed cards. They never apply to ARCANE cards
(arcane = 0 NDM, fixed). DEAD cards score 0 regardless and so are not
affected.

### Archive core — the only "outside-the-stack" core

Every other core folds into one per-card `core_mult` that respects the
`stacking.additive_cores` flag (sum in Wold's, product in Vanilla).
Archive does **not**. After all the other math, each scoring card's
contribution is multiplied by an Archive factor of
`archive_core ^ (2.1·√n_arcane_placed)` — the live
`GroupSynergyMultiplierModifier` formula from the pack's final Archive
balance pass (upstream 0e54a67f, 2026-07-14; replaced per-card compounding,
which itself had briefly been a log-softcap the optimizer never shipped):

```
final_ndm_per_card = base × core_mult × greed_boost × archive_mult
                                                     ^^^^^^^^^^^^
                                                    where archive_mult =
                                                      base_value ^ (2.1·√n_arcane_placed)
                                                      (1.0 when Archive isn't picked)
```

Worked example (Wold's default `archive_core: 1.2`, vs the old `base^n`):

| Arcane cards placed | Archive factor (live √) | old `base^n` |
| --- | --- | --- |
| 0 | 1.0  | 1.0 |
| 1 | 1.47 | 1.2  |
| 2 | 1.72 | 1.44 |
| 4 | 2.15 | 2.07 |
| 8 | 2.95 | 4.30 |
| 12 | 3.77 | 8.92 |

Small counts got a mild buff; big stacks are tamed. (A short-lived
"experimental additive Archive" toggle explored folding `base^n` into the
core stack — superseded by this live formula and removed.)

Override semantics: when the user sets an override on Archive, the
override replaces the **rolled base** `(1 + v)`, not the final multiplier.
So an override of `1.5` yields a final factor of `1.5 ^ (2.1·√n)`.
Mirrors PURE / DELUXE_CORE / VOID_CORE, where the override replaces the
per-N scale term rather than the resolved value.

Two-stage gating:

1. **Mode gate** — `cores.archive_allow` (vanilla off). When false, the
   Archive Core row is hidden in the core picker and the candidate
   enumerator never considers it, regardless of deck shape.
2. **Geometry gate** — even with `archive_allow: true`, enumeration only
   proposes ARCHIVE_CORE when the deck has at least one arcane slot.
   Otherwise `n_arcane_placed` is permanently 0, the factor is
   permanently 1.0, and the core would waste a slot. This is the only
   candidate-enumeration gate that depends on deck *geometry* (the mode
   gate is the standard cfg-flag pattern).

### Pure core's `n_ns` formula

`n_ns` is the count of "non-shiny" placements in the deck — what `PURE`
scales against. ARCANE always counts. Beyond that, the class+FOIL state
decides:

| `card_class` | FOIL in cores? | `n_ns`                      |
| -------------- | -------------- | ----------------------------- |
| `EVO`        | No             | `regulars + greed + arcane` |
| `EVO`        | Yes            | `greed + arcane`            |
| `SHINY`      | (any)          | `greed + arcane`            |

`TYPELESS` is **never** in `n_ns` — typeless cards are "always shiny" by
design. `DELUXE` is **never** in `n_ns` — deluxe rides its own scoring
track via DELUXE_CORE; double-counting would be wrong.

Pure mult is then `1.0 + 0.07 × n_ns`. For e.g. a SHINY+Pure run on the
Starter deck with 7 greed + 1 arcane placed: `n_ns = 8`, Pure mult = `1.56×`.

### Pluto core (REMOVED)

The Pluto core was removed from the optimizer entirely in the modpack
update that nerfed it to never be optimal. No code path, config key, or
UI surface for it remains. Historical reference only: it used to be an
EVO-only flat 3× core that targeted the smaller of {EVO regulars, deluxe
cards}, with ties boosting both groups. If a future pack version
reintroduces a similar mechanic, restore from the previous commit
history rather than re-deriving the design here.

---

## Structural cores (Construction + Arcane) — WEB ONLY

Two Wold's-exclusive cores let the player **mutate the deck layout** in
the UI before the SA runs. The SA kernel never learns about them: by the
time `runSaInventory` is invoked, the deck it scores already has the
extra tiles / converted arcane positions baked in.

Both are wired only into the WASM web app (`wasm-port/`). The Python
spreadsheet CLI has no equivalent surface — these cores would change the
deck topology, which is meaningless in a panel-grid spreadsheet sweep
that compares every deck at its stock layout.

| Core              | Effect                                            | Limit | Allow flag                  | Costs core slot? |
| ----------------- | ------------------------------------------------- | ----- | --------------------------- | ---------------- |
| Construction Core | Add new regular (`O`) slots to the deck grid      | ≤ 3   | `cores.construction_allow`  | Yes (1)          |
| Arcane Core       | Convert existing regular slots to arcane (`A`)    | ≤ 3   | `cores.arcane_core_allow`   | Yes (1)          |

> ⚠️ **Arcane Core ≠ Archive Core.** The Archive Core (scoring
> multiplier, `base ^ (2.1·√n_arcane_placed)`) is unchanged by structural
> cores themselves. The Arcane Core is a **structural** core that creates
> new arcane slots. They synergize: each Arcane Core conversion bumps
> `n_arcane_placed` (assuming the SA fills the new arcane slot), which
> raises the Archive exponent — though the √ softens stacking deep.

### Allow flags and platform gating

- Both flags default `true` (Wold's). `modes.vanilla.cores` flips both to
  `false` so the picker entries are hidden mode-wide, mirroring how
  `archive_allow` works. The flags live in the shared `config.yaml`; the
  outer CLI ignores them silently.
- The picker surface (`CorePicker.svelte`) only renders the Structural
  subsection when at least one of the two flags is true for the active
  mode.

### Core-slot cost

Each enabled structural core consumes one of the deck's core-slot budget
**before** SA candidate enumeration sees it:

```
effective_core_slots_for_SA =
    max(0, base_core_slots + bonusCores − structuralCost)
```

`structuralCost = (constructionEnabled ? 1 : 0) + (arcaneCoreEnabled ? 1 : 0)`.

If the pre-clamp value is negative (the structural cores cost more slots
than the deck has after Bonus Cores), `App.svelte::run` errors out
before launching the SA. The user sees a banner; nothing crashes.

The Deck-card meta line shows the cost: `… X cores (−N structural) · …`
so the budget reduction is visible without opening the cores panel.

### Construction Core — connectivity rule

A construction placement is valid only at a cell that is **8-direction
adjacent** to at least one existing slot. "Existing slot" means a slot
in either:

- the original deck (`Deck.slots`), or
- any tile already placed by the Construction Core in this session
  (`structural.addedSlots`).

Placement chains: adding slot #1 unlocks new candidates 8-adjacent to
#1. Adding slot #2 (which may have only been possible because #1
existed) is fine.

Removal (right-click on a construction tile) is allowed iff the
remaining additions stay **connected** to the original layout via
8-adjacency through originals + remaining additions. So a tile that
later additions hang off of is locked until those later additions are
removed first. This is the only "ordering" rule on the Construction
Core; we don't track placement order otherwise.

#### Hard footprint cap — 9 × 6

The bounding box of all placeable slots (native + construction-added
`O`/`A` positions) is capped at **9 cells wide × 6 cells tall**. The
Construction Core never surfaces a candidate that would push the bbox
past those dimensions. This matches the engine's hard ceiling on deck
size — you cannot construct your way past the largest legal deck.

A candidate is filtered iff:

- adding it would make the new max-col − min-col + 1 > 9, OR
- adding it would make the new max-row − min-row + 1 > 6.

Candidates *inside* the current bbox are always safe (the bbox doesn't
grow). A deck already at 9×6 only sees candidates in its current
bbox (i.e. infilling holes between existing slots).

Implementation: `wasm-port/web/src/lib/structural.ts`:

- `constructionCandidates(base, sc)` — set of cells legal for the next
  placement, filtered by both 8-adjacency and the 9×6 bbox cap.
  Returns empty once the cap of 3 is reached.
- `canRemoveConstructionTile(pos, base, sc)` — BFS from all originals
  through the universe `originals ∪ (additions − {pos})`; succeeds iff
  every remaining addition is reached.

### Arcane Core — conversion rule

The Arcane Core converts existing regular (`O`) slots into arcane (`A`)
slots, up to 3 per session. Conversions can target either:

- a native regular slot from the deck layout, or
- a regular slot placed by the Construction Core in the same session.

Native arcane slots (from the deck JSON/YAML) are **untouched**: they
stay arcane and don't count against the conversion limit. Right-click on
a converted slot reverts it back to regular.

If the Construction Core is later deselected (or a specific construction
tile is removed), any conversions that pointed at the now-deleted tiles
are pruned automatically (`pruneConvertedSlots`).

### State-clear semantics

Any structural state mutation — toggling a core on/off, adding a
construction tile, removing one, converting a slot, reverting a
conversion — clears the prior SA result and the preview-tab assignments.
This avoids the UI ever showing a placement that points to slots that no
longer exist (or to a layout the SA didn't actually score).

Mode flips and deck-dropdown changes call `resetStructural()` for the
same reason — and because the coordinate space (Position values) is
specific to the previous deck.

### Click precedence on the deck grid

When both a Run result AND a structural-core tool mode are active at the
same time, **left-click on a slot prioritizes the breakdown popup over
the tool action**. So clicking a filled slot post-Run never quietly
mutates the layout — the user sees the per-slot math instead. The tool
actions reactivate as soon as the result is cleared (which any
structural mutation also does, transitively).

Practical implication: to convert / place more tiles after a Run, the
user either right-clicks an existing converted/added tile to revert
it (frees up the structural-core budget and clears the result, leaving
slots empty for the next left-click), or toggles the relevant
structural core off and back on. Implementation: `DeckGrid.svelte`'s
`handleSlotClick` checks `breakdown.get(key)` first; only if absent
does it dispatch to `onConvertSlot` / `onUnconvertSlot`. Right-click
behavior is unchanged — it always reverts.

### What the SA sees

`effectiveDeck(base, sc, deckmod)` (in `structural.ts`) returns the
mutated `Deck` used by the SA:

- `slots = base.slots ∪ addedSlots`
- `arcaneSlots = (base.arcaneSlots ∪ convertedSlots) ∩ slots`
- `rowPeers / colPeers / surrPeers / diagPeers` recomputed from the new
  slot set
- `arcaneSlotIndices` rebuilt against the new ordering
- `base_core_slots`, `min_regular`, `max_greed` unchanged

The kernel + breakdown re-score then run unchanged. There is **no
scoring multiplier associated with these cores** — their only effect is
the layout change plus the budget cost.

---

## Build-your-own-deck tab — WEB ONLY

The Build tab (`wasm-port/web/src/components/BuilderPanel.svelte`) is a
pre-SA layout factory, not a new scoring rule. The user draws an
arbitrary 9×6 layout on a blank canvas (Regular / Arcane / Erase tools),
names it, picks a core count, then runs the same inventory optimizer
the Optimize tab uses. Saved decks persist in `localStorage` only —
nothing about the SA, the scoring math, or the deck pipeline differs.

Pipeline: `BuilderState` → `builderToDeck()` produces a `Deck` via the
same `buildDeck()` constructor that JSON / YAML loads use. From there,
the structural cores (if equipped) layer on via `effectiveDeck()`
exactly as for a roster deck. The structural-core 9×6 cap and the
Builder's canvas size share the same `MAX_GRID_WIDTH` / `MAX_GRID_HEIGHT`
constants.

Export is JSON-only — the modpack's deck-data shape (per
`decks/wolds_decks.json`):

```json
{
  "<key>": {
    "model":       "woldsvaults:deck/<key>#inventory",
    "name":        "<display name>",
    "essence":     { "min": 5, "max": 5 },
    "layout":      [ { "value": ["...", "..."], "weight": 1.0 } ],
    "socketCount": { "min": <cores>, "max": <cores> }
  }
}
```

`<key>` derives from the name (lowercase, non-alphanumerics →
underscores, strip leading digits / underscores; collisions get
`_2`, `_3`, … suffixes when saving). `min_regular` / `max_greed` are
**not** surfaced in the Builder — neither field exists in the modpack
JSON shape — and they default to `-1` (unconstrained) when the built
deck flows into the SA.

The Build tab uses the same `CorePicker` and `InventoryTable` panels as
Optimize. Saved decks **never** appear in the Optimize tab's deck
dropdown, only in the Builder's own "Saved decks" selector — this keeps
user experiments visually separate from the modpack's roster.

There is no platform-discrepancy entry to add here: the spreadsheet CLI
has no Build surface (panel sweeps over a fixed roster), and the WASM
Build tab feeds its synthesized `Deck` through the exact same scoring
pipeline as everything else.

---

## Snapshots tab — WEB ONLY

The Snapshots tab is a localStorage-backed history of past Runs. Each
snapshot is **self-contained**: the deck layout is embedded directly in
the record (rather than stored by roster key), so renames or removals
of modpack decks never orphan a saved capture. Snapshots are also
**mode-locked** — each record stores the mode it was taken in, and
loading auto-switches `app.mode` (with the unsaved-builder guard
in front) before restoring.

### What's captured

`captureSnapshot()` in `lib/state.svelte.ts` mirrors live `app.*` state:

| Field | Source |
| --- | --- |
| `deck` | `app.deck` — slots / arcaneSlots / base_core_slots / etc. |
| `mode` | `app.mode` (e.g. `"wolds"`) |
| `cardClass`, `bonusCores`, `autoPlaceArcane` | `app.*` |
| `inventoryCounts`, `forcedCounts` | `app.*` (shallow-copied) |
| `minRegularPlaced` | `app.*` — stat-giving floor at the time of run (old snapshots default to 0) |
| `cores` | `app.result.coresUsed` — the SA-chosen combo, **not** the picker state |
| `structural` | `app.structural` (including `addedSlots` + `convertedSlots`) |
| `assignment` | `app.result.assignment`, serialized parallel to `deck.slots` |
| `wasmScore` | `app.result.wasmScore` |

The breakdown is **not** stored — it's recomputed via
`simulateInventoryBreakdown()` on restore so the click-for-breakdown
popup just works.

### Restore semantics

`restoreSnapshot()` rebuilds `app.deck` via the same `buildDeck()`
constructor JSON/YAML loads use, repopulates every input field, syncs
the CorePicker checkboxes from `snap.cores`, then synthesizes a fresh
`OptimizeResult` (assignment + recomputed breakdown). The user is then
flipped to the Optimize tab.

**No edit-after-load round-trip.** Loading a snapshot disassociates
from it — once loaded, the user is editing live `app.*` state. Saving a
new snapshot creates a new record; there is no "modified snapshot"
dirty-flag flow.

### What's not captured

- No SA params (`nIter`, `restarts`) — these are tuning, not part of
  the captured result. Restored snapshots show the SA's actual output;
  re-running uses the current SA-params values.
- No `tab` state — snapshots load into Optimize regardless of which tab
  they were taken in. The `isBuiltDeck` flag is preserved purely as a
  visual badge in the Snapshots list.

### Storage

`localStorage` key `wvdo.snapshots.v1`. Single JSON array;
`loadAllSnapshots()` rewrites the whole array on every CRUD call.
Parse failures fall back to `[]` with a `console.error` rather than
bricking the tab. Per-record size is ~1-3 KB; the typical 5 MB quota
holds hundreds of snapshots.

---

## Core stacking — additive vs multiplicative

Controlled by `stacking.additive_cores` (**true** in Wold's, **false** in
Vanilla).

Let `baseline_contribs` = list of base multipliers from all enabled
non-card-specific cores (Pure, Equi, Steadfast, Color, Foil at their
computed values).

**Additive** (`true`, Wold's): `core_mult = 1.0 + Σ(c − 1)` over all
baseline contribs, plus per-category addends:

- `regular_core_mult` adds `(deluxe_core_value − 1)` if DELUXE_CORE on,
  and `(void_core_value − 1)` if VOID_CORE on.
- `deluxe_card_core_mult` adds `(void_core_value − 1)` but **does not**
  add the deluxe-core addend (deluxe cards don't boost themselves).
- `typeless_core_mult` adds `(deluxe_core_value − 1)` and
  `(void_core_value − 1)`.

**Multiplicative** (`false`, Vanilla): `baseline_prod = Π baseline_contribs`,
then per category multiplied by the relevant DELUXE/VOID factors. Same
gating rules.

In both modes, the per-card NDM is `base × core_mult × boost`. So an EVO
ROW card in an additive deck with FOIL + COLOR + DELUXE_CORE (any deluxe
cards) gets:

```
core_mult = 1 + (2.5 − 1) + (1.75 − 1) + (deluxe_value − 1)
          = 1 + 1.5 + 0.75 + (deluxe_value − 1)
```

---

## Per-class rules — SHINY vs EVO vs FOIL

The class gates several scoring paths:

- **EQUILIBRIUM, STEADFAST** apply only when `card_class == SHINY`. In
  EVO runs they're silently dropped from the baseline (the candidate-core
  enumerator never includes them on the EVO side).
- **EVO_GREED** applies only when `card_class == EVO`. In SHINY runs an
  evo-greed card is a no-op (counts as a greed for `n_ns` but boosts
  nothing).
- **`n_ns` formula** (above) differs per class, with FOIL flipping EVO
  to act like SHINY for the purposes of `n_ns` only — FOIL does **not**
  re-classify cards themselves, it just changes how Pure scales.
- **Positional cards** (ROW/COL/SURR/DIAG): in Wold's, valid for both
  SHINY and EVO. In Vanilla with `shiny.positional: false`, SHINY decks
  are **typeless-only** — positional cards are hidden from the inventory
  picker and filtered out at optimize-time before reaching the SA (see
  `wasm-port/web/src/lib/visibility.ts::hiddenInventoryTypes` and
  `App.svelte::run`).

---

## Deck-level parameters

| Parameter                  | Source                                              | Role                                                |
| -------------------------- | --------------------------------------------------- | --------------------------------------------------- |
| `slots`                  | Deck JSON/YAML layout                               | Positions of every regular OR arcane slot           |
| `arcane_slots`           | Subset of `slots`                                 | Positions restricted to ARCANE/DEAD placement       |
| `base_core_slots`        | Deck JSON `socketCount.max` / YAML `core_slots` | Raw core-slot count from the in-game deck           |
| `core_slots` (effective) | `max(0, base_core_slots + Bonus Cores)`           | What the SA enumerates against                      |
| `deckmod`                | `config.yaml` per mode (1 Wold's, 0 Vanilla)      | Initial Bonus Cores default when a mode is selected |
| `min_regular`            | YAML deck or panel-config override                  | Lower bound on placed regular cards                 |
| `max_greed`              | YAML deck or panel-config override                  | Upper bound on placed greed cards                   |

**Bonus Cores** is the user-adjustable override on top of `deckmod`. It's
unbounded; when negative beyond `base_core_slots`, effective core slots
clamp to 0. Re-seeded to `cfg.deckmod` on every mode flip.

In the WASM app: state lives at `app.bonusCores` in
`wasm-port/web/src/lib/state.svelte.ts`. In the NiceGUI: `state.bonus_cores`
in `src/gui.py` (rebuilds a temporary `Deck` via `Deck.with_core_slots()`
before each run so the canonical DECKS list stays clean).

---

## Wold's vs Vanilla — config differences

Every key listed below is identical at default and only differs between
modes via the `modes.vanilla:` block of `config.yaml`. Unlisted keys are
identical.

| Key                         | Wold's               | Vanilla           | Effect                                                                                                      |
| --------------------------- | -------------------- | ----------------- | ----------------------------------------------------------------------------------------------------------- |
| `deckmod`                 | **1**          | **0**       | Wold's starts with +1 effective core slot (Core Expertise free-slot mechanic); Vanilla has no such mechanic |
| `shiny.positional`        | **true**       | **false**   | Vanilla SHINY decks have no positional cards — typeless-only ("Stat" decks)                                |
| `deluxe.allow`            | **true**       | **false**   | Vanilla disables DELUXE cards + DELUXE_CORE entirely                                                        |
| `cores.void_allow`        | **true**       | **false**   | Vanilla has no Void core (and therefore no DEAD-card optimization)                                          |
| `cores.archive_allow`     | **true**       | **false**   | Vanilla hides Archive Core from the picker and the enumerator regardless of whether the deck has arcane slots |
| `cores.construction_allow` | **true**      | **false**   | Vanilla hides the structural Construction Core (web only) |
| `cores.arcane_core_allow` | **true**       | **false**   | Vanilla hides the structural Arcane Core (web only)       |
| `cores.sparkling_allow`   | **true**       | **false**   | Vanilla hides the SHINY-only Sparkling core               |
| `stacking.additive_cores` | **true**       | **false**   | Vanilla multiplies cores instead of summing                                                                 |
| `decks.json_file`         | `wolds_decks.json` | `vh_decks.json` | Different deck rosters per mode                                                                             |
| `cards.json_file`         | `modifiers.json` | `vh_modifiers.json` | Different game-card rosters (303 vs 183 entries) — feeds the Preview chooser + the legal tag-combo catalog (28 vs 17 combos) |
| `cores.equilibrium`       | **1.5**        | **1.7**     | Best in-game roll per mode (see convention below)                                                           |
| `cores.steadfast`         | **2.1**        | **2.2**     | Best in-game roll per mode                                                                                  |
| `cores.color`             | **1.75**       | **1.5**     | Best in-game roll per mode                                                                                  |
| `cores.pure_scale`        | **0.07**       | **0.05**    | Wold's greater-pure max vs vanilla base max (vanilla's greater tier is a broken 0.3–0.5 placeholder — see below) |

Greed defaults (`dir_vert: 4`, `dir_horiz: 4`, others 0) and the remaining
core multipliers (`foil: 2.5`, `sparkling: 2.5`, `void_scale: 0.3`,
`archive_core: 1.2`, `deluxe.core_scale: 0.2`, `deluxe.flat: 2`) are shared.

**Core-default convention**: each default is the **best roll the game can
drop** for that core — the max of the base/lesser/greater tiers in the
game's `card/deck_modifiers.json` (Wold's: the pack repo; the addon cores
void/archive/sparkling/premium come from the woldsvaults datagen
`new_cores.json`). Re-synced 2026-07-14. Vanilla values were read from a
stock Vault Hunters instance (`the_vault 3.21.1`); its greater tier is a
copy-pasted `0.3–0.5` block on several cores (greater shiny/steadfast roll
*below* their base tiers, greater pure reads 10× base — the exact values the
Wold's pack later "fixed"), so where that block would be the max we use the
best *coherent* tier instead: vanilla pure stays `0.05` (base max), while
vanilla color takes the `0.5` (plausible greater). If VH ever fixes the
block, re-sync.

The UI also relabels in Vanilla: SHINY → "Stat" in class pickers
(`wasm-port/web/src/lib/visibility.ts::classSelectLabel`).

---

## Inventory bounds (WASM web app)

The inventory-aware optimizer supports two distinct constraint shapes on
top of the regular pool's per-(type, color) **upper bound**:

| Constraint | UI | Shape | Where enforced |
| --- | --- | --- | --- |
| Per-(type, color) lower bound | "Forced" view of the Inventory table | `forced_inventory[t,c]` — placed[t,c] must stay ≥ forced[t,c] | `initial_fill` (Phase 1) + `sa_one_restart` per-move check (`wasm-port/ndm_core/src/inventory.rs`) |
| Aggregate floor on stat-giving cards | "Minimum stat-giving cards placed" input below the forced grid | `min_regular_placed` — `Σ placed[t,_] for t ∈ STAT` must stay ≥ this | `sa_one_restart` per-move check (same file) |

**STAT set** = `{ROW, COL, SURR, DIAG, DELUXE, TYPELESS}`. Anything else
(greeds, ARCANE, DEAD) is non-stat. The set is hard-coded as
`is_stat_giving(t)` in `wasm-port/ndm_core/src/inventory.rs` (the u8
layout makes it `t ≤ TYPELESS`).

The aggregate floor is checked in TS upstream of the worker dispatch
(`enumerateCandidates` in `wasm-port/web/src/lib/optimize.ts`) for two
infeasibilities — not enough stat-giving cards in inventory + forced, or
not enough non-arcane slots left after forced non-stat placements — and
throws a user-readable error before the SA runs.

If a caller skips the TS pre-flight (e.g. the parity scripts in
`wasm-port/scripts/`) and `min_regular_placed` exceeds what
`initial_fill` can achieve, the kernel clamps the effective floor to
whatever init placed. This avoids the SA freezing on every proposal but
silently degrades the constraint — the TS path is the authoritative
infeasibility detector.

The aggregate floor is a WASM-only feature. The spreadsheet CLI does not
take inventory at all; its closest analogue is the panel CLI's
`min_regular` / `max_greed` deck-level fields, which are unrelated.

---

## Platform discrepancies

Everything above describes the **WASM web app**. Anything not listed here
is identical across all three platforms.

### 1. Color-aware vs color-blind scoring

The **WASM web app** uses the inventory-color-aware optimizer
(`wasm-port/ndm_core/src/inventory.rs`). In that model:

- Positional peer counts (`row_count`, `col_count`, surr/diag) consider
  **only same-color cards** in scan range.
- `COLOR` core only boosts cards whose color matches the core's color
  selection.

The **spreadsheet CLI** (`src/simulate.py` + `ndm_core/src/lib.rs`) is
**color-blind**: every filled neighbor counts for positional peers
regardless of color, and COLOR's `1.75×` applies to every scorable card
flat. This is a real semantic difference in the SA's optimum, not a bug —
the CLI predates the color-aware model and serves as the simpler
spec-style implementation.

> **Historical note (now obsolete):** Earlier revisions of this doc
> tracked an EVO-no-FOIL `n_ns` drift between the WASM web app and the
> desktop NiceGUI tool, plus vestigial `batch.rs` and `wasm-port/src/`
> copies. Those have all been deleted in the channel-consolidation
> refactor — the only remaining discrepancy is color-aware vs
> color-blind, called out above.

---

## When you change scoring code, also update this file

Touch points to keep in sync, by surface:

- **Add a new card type** → update the **Card types** table + (if greedy)
  the **Greed cards** table.
- **Add or rename a core** → **Cores** table + relevant gating sections.
  If it's a structural (layout-mutating) core like Construction / Arcane,
  also update **Structural cores** with the new mechanic.
- **Change a multiplier default** in `config.yaml` → update the Wold's
  column of **Cores** / **Greed cards** + the **Wold's vs Vanilla**
  table.
- **Change the `n_ns` formula** → update the formula table under **Pure
  core's `n_ns` formula** + the **Platform discrepancies** entry if the
  change makes one path diverge from the others.
- **Change a class-gating rule** (e.g. allowing a SHINY-only core in
  EVO) → update **Per-class rules** + the **Cores** table's "Class
  gating" column.
- **Change stacking mode behavior** → **Core stacking** + **Greed
  stacking** sections.

If a code change touches scoring and you can't figure out which section
applies, add a note under the most-related section explaining the new
behavior — better to have an awkward note than a silent divergence.

---

## Optimizer 2.0 addendum

One tag-aware SA kernel (`ndm_core/src/tagsim.rs`, included verbatim by the
wasm crate via `#[path]`) backs three run modes. The design spec is
`WV_DECK_OPTIMIZER_2.0.md`; this section pins the behaviors as implemented.

### The three modes (one kernel, three configurations)

| Mode | Supply | Colors | Tags | Extra |
|---|---|---|---|---|
| **Max** | unlimited, mono-color | color-blind (`colors_real=false`) | blanket favorable (implicit-rewarded groups free on every non-greed card) | = spreadsheet engine; reproduces classic 1.x numbers (see parity gate) |
| **Targeted** | unlimited | real per-card colors **iff** any color rule or Complex Cards is active, else Max-style | per-tag Min/Max rules; capped implicit-relevant groups become per-slot SA toggle moves | multi-tag counting: a card counts toward every rule it matches |
| **Exact** | finite per-stack multiset (+ per-stack must-place minimum) | always real | real per-card groups from the builder | no blanket assumptions anywhere |

### Card model

`Card = (type, card_color, scale_color, groups: u16 bitmask)`. Groups =
the 9 category tags (Offensive, Defensive, Physical, Magical, Utility,
Resource, Knack, Temporal, Essence) + `Foil` + `Stat`. Category tags are
**NDM-inert except through a deck implicit** (author-confirmed); the two
NDM-relevant exceptions are **Foil** and **Wild**.

- **Card-type vocabulary:** real greed = the 4 orthogonal directions only.
  `SURR_GREED`, `EVO_GREED`, and diagonal greeds are gone from the 2.0
  engine (unobtainable in-game; as 0-multiplier fillers they were
  score-equivalent to an orthogonal greed, so optima are preserved).
  `DEAD` = empty slot; always permitted, never capped.
- **Wild** is its own card type: 0 NDM, counts as **any color and any
  group for neighbors'** positional counts, chain connectivity, mirror
  checks, and adjacency implicits. For Targeted rule counting it counts
  toward only its own type + literal color (documented choice). Excluded
  from Max supply (never optimal under mono assumptions).
- **Stat** is run-derived in EVERY mode and on NO user surface: shiny ⇒
  every stat-giving card carries it, evo ⇒ never (playtest ruling). The
  kernel adds it automatically (incl. Exact builds) for treasure/mutant;
  it is not a builder chip, not a notch, not in the tag-edit popup.
- **Non-stat categories (`Resource`, `Temporal`)** — a card carrying one
  gives no player stats, so it scores **0 NDM itself** (playtest ruling;
  kernel `NONSTAT_GROUPS`). It still fills its slot: row/col/peer counts,
  `n_ns`, chain connectivity, and implicits that read it (merchant's
  column Resource count) all see it. Consequences: these tags are
  **never blanket-assigned**; wherever an implicit reads one (merchant;
  mutant's diversity) it becomes an **assignable** per-slot SA decision —
  the optimizer weighs each 0-NDM "battery" card against the implicit
  value it feeds. Non-stat-tagged cards do NOT count toward the min-stat
  floor (the toggle move updates the floor counter).
- **Max mono color** follows a color-keyed implicit (velara ⇒ green,
  ornate ⇒ red, …) so the displayed deck matches the build guidance;
  scoring stays color-blind either way. Score-tied COLOR-core candidates
  resolve toward the same color.
- **Arcane cards carry no tags, ever** (playtest ruling): no categories,
  no Foil, in every mode (the kernel strips them even off Exact stacks).
  A foil arcane could only cost you; our `n_ns` counts arcane
  unconditionally, which always matches the never-foil reality. The
  builder and tag-edit popup refuse tags on arcane.
- **Legal tag combos**: the distinct category-tag sets on REAL cards
  (extracted from `modifiers.json` gear + task_loot entries — 28 combos,
  emitted into the web bundle as `_tagCombos` and loaded by the CLI from
  the same file) bound every tag surface. A card's category set must be a
  **subset** of some real card's set (Wild exempt). Enforced in: the SA's
  tag-toggle moves (kernel `legal_combos`), the Exact builder's chips,
  and the what-if popup. A blanket UNION that isn't buildable as one real
  card (Mystery pairs like champion+fairy, mutant's all-category
  diversity) demotes wholesale to assignable — the SA then distributes
  legal per-card subsets.
- **Preview chooser** (UI consequence): a slot only offers real cards
  carrying ALL of its optimized category tags; non-stat (Resource /
  Temporal) battery slots are not assignable at all for now.

### Foil, per-card (§5)

`Foil` is a per-card tag. Run rule (Max/Targeted): **Wold's shiny ⇒ foil**
on every scorable/arcane card; **evo ⇒ foil iff the FOIL core is in the
candidate set**; vanilla shiny is never forced foil. Pure's `n_ns` is now
per-card: `n_ns = greed + arcane + non-foil positionals` (typeless/deluxe
still never count). This reproduces the classic class+FOIL table exactly
while enabling:

- **§6 non-foil-evo final pass** (Wold's only, evo+FOIL runs): after SA,
  each *wasted* greed (orthogonal target off-deck or non-scorable) is
  replaced by the best non-foil evo positional if that strictly improves
  the full re-simulated score. This is an intentional model improvement
  over 1.x — Wold's EVO+FOIL numbers may exceed the classic optimizer's.
  Vanilla never runs the pass (it stays the regression baseline).
- **Targeted foil ban (max 0):** evo cards all non-foil regardless of the
  FOIL core; on a Wold's shiny run nothing is placeable → empty deck (UI
  warns).

### Deck implicits (Wold's only)

Data: `decks/wolds_implicits.json`, extracted from the woldsvaults datagen
(28 decks). Aggregation matches `MixinCardDeck`: implicit values fold
**additively** into the per-card core multiplier (`1 + Σ(v−1) + …`),
except **runic**, which **multiplies** the whole per-card value. Kinds:

| kind | decks | per-card effect (additive unless noted) |
|---|---|---|
| `global` | treasure, idona/velara/tenos/wendarr, cactus (Off∧Def — ALL groups required), champion, anvil, belt, fairy, gilded/ornate/living (Foil+color) | `+value` when the card carries ALL required groups and a required color. Color-blind runs treat color conditions as satisfied (favorable blanket). |
| `freq` | wall (col ×2), pillager (surr ×2), bishop (diag ×3) | multiplies the matching positional type's **count** by `round(value)`; DIAG keeps its ≥1 floor after scaling |
| `adjacency` | merchant (column/Resource ×0.5), skull (surr/Knack ×0.5) | `+value ×` matching-group cards in range (greeds/dead never match; arcane can; wild always) |
| `color_mismatch` | puzzle | `+value ×` orth neighbors of a different color (blanket: every filled orth neighbor) |
| `row_pos` | cake | `+value × rows-from-bottom`, bottom row = 1 |
| `chain` | snake | `+value × (component−1)`, same-color orthogonal flood-fill (blanket: filled connectivity; wild bridges) |
| `empty_slots` | shadow | `+value × n_dead` on every scoring card |
| `unique_groups` | mutant | Stat cards only: `+value ×` (distinct groups incl. class markers present) |
| `mirror` | runic | **×value** when the horizontal bbox-mirror slot holds a same-color card; center column auto-passes; blanket: mirror filled |
| `gameplay` | extended, arcane, relic, villager | NDM-inert (villager boosts only arcane-slot cards, which score 0) — display note only |
| `mystery` | mystery | player picks the two rolled implicits in the UI; both evaluated; conflicting Max blankets resolved best-single-assumption |

### Constraint semantics

- **Tag rules** (`Targeted`): axes = color / card type / group / greed-total,
  each with optional Min and Max. A card counts toward **every** rule it
  matches. Min on an implicit-relevant group forces that many cards to
  carry it (free — tags are inert); Max turns per-card carriage into SA
  toggle moves. DEAD is exempt from all rules.
- **Min-stat floor**: kernel-level lower bound on placed stat-giving cards
  (positional + typeless; deluxe counts only when `floor_counts_deluxe` —
  the web app says yes, the spreadsheet mirrors classic
  `deluxe_counted_as_regular=false`).
- **Spreadsheet panel configs** map: `min_regular → min-stat floor`
  (with the classic conflict-nullification rule), `max_greed → greed-total
  Max rule`.

### Complex Cards (§7)

Toggle, default OFF. OFF: `scale_color == card_color` everywhere; greed is
color-agnostic; identical to 1.x (the validation gate runs this way). ON
(forces real colors): a positional card counts neighbors whose
**card_color == its own scale_color** (self counts only if its own color
matches its scale color); greed boosts only targets matching the greed's
scale_color; the Exact builder exposes both colors.

### Engine switch + validation gate

`config.yaml engine: tagged|classic` picks the spreadsheet kernel (default
`tagged` — implicits included on Wold's). `scripts/parity_2_0.py` is the
gate: **Part A** proves scoring equivalence (random assignments over the
shared vocabulary, Python reference `simulate()` vs Rust `score_tagged`;
passes at rel-Δ ≤ 5e-16), **Part B** checks SA-optimum convergence
(classic vs tagged-Max with implicits stripped and §6 off; all combos
converge at the 60k×12 production budget). Run it after any kernel change.

### Core value entry is % (2026-07-14 rework)

The side-panel override boxes take the **% printed on the core item
in-game**, converted at the UI boundary only — config.yaml, snapshots and
the kernel payload keep raw stored units, so nothing downstream changed:

- **Scaling cores** (`PURE`, `DELUXE_CORE`, `VOID_CORE`): the box is the
  per-unit increment; stored = pct / 100 (a +6.3%-per-card Pure roll is
  entered as `6.3` → 0.063).
- **Everything else** (`EQUILIBRIUM`, `STEADFAST`, `SPARKLING`, `FOIL`,
  `COLOR`, `ARCHIVE_CORE`): the box is the flat bonus above 1; stored =
  1 + pct / 100 (a +150% Shiny core is entered as `150` → 2.5). Archive's
  % reads "per placed arcane card" (stored 1.2 ⇄ 20%) — numerically the
  same rule.

Mapping lives in `wasm-port/web/src/lib/coreOptions.ts`
(`coreValueKind` / `storedToPct` / `pctToStored`); placeholders show the
config default in the same % units. The `FOIL` core is displayed as
**"Shiny"** (its in-game name — it boosts Foil-group cards; the *Sparkling*
core is the one that boosts shiny cards); `foil` remains the internal key in
config / kernel / snapshots, so old snapshots restore unchanged.

### Per-mode game-card dumps (2026-07-14)

`cards.json_file` in config.yaml picks the mode's card dump:
`modifiers.json` (Wold's — verbatim from the pack's
`config/the_vault/card/modifiers.json`, refreshed 2026-07-14: 28 entries had
been retuned upstream) vs `vh_modifiers.json` (vanilla — verbatim from a
stock VH `the_vault 3.21.1` instance). `build_data.py` ships them as
`modifiers_<mode>.json` and computes `_tagCombos` **per mode** (wolds 28
combos, vanilla 17), so the Preview chooser, the Exact builder chips, the
what-if popup and the kernel's tag toggles all enforce the active mode's own
card reality. The CLI reads the same per-mode file via
`src/config.py::CARDS_JSON_FILE` → `src/implicits.py`.

### Puzzle (color_mismatch) forces real colors; implicit on/off toggle (2026-07-14)

- **color_mismatch (puzzle) always runs with `colors_real`** — in Max /
  Targeted too, not just Exact/Complex. Under the blanket mono model the
  kernel scored the §4 "best case" (every filled orthogonal neighbor counts
  as mismatched) while the grid displayed a single-color deck — score and
  display disagreed. Now the supply carries all four colors and the SA
  optimizes them for real (it converges on the expected two-color
  checkerboard; positional cards pay the true same-color-peer cost). Applies
  to both channels (`tagged.ts::colorsRealFor`, `simulate.py::
  sa_optimize_tagged`); the CLI also gives a selected COLOR core a concrete
  color under this rule (a colorless one is inert when colors are real —
  any single color is symmetric in unlimited supply).
- **Deck-implicit toggle** (Deck card, default ON; reset to ON on deck/mode
  change): OFF scores the bare layout so base-vs-implicit NDM can be
  compared. Toggling clears the run result (a stale result would disagree
  with every re-score under the new setting). Snapshots capture the flag
  (absent = ON). CLI equivalent: config `implicits.enabled` and the
  `--no-implicits` flag (vanilla is unaffected — it never has implicits).

### "Include selected construction cores" + experimental additive Archive (2026-07-14 balance pass)

- **Structural balance layouts** (`structural_cores.include_selected` /
  `--structural-cores`, spreadsheet CLI only): decks listed in
  `decks/structural_layouts.json` swap to their pre-built greater
  Construction / Arcane-core layouts, and `structural_cores_used` is
  subtracted from the deck's core budget (Wold −2, Fairy −2, Mystery −1) so
  the run matches the in-game cost. A layout entry may force implicit keys —
  Mystery runs its chosen **runic + bishop** pair this way (each key resolves
  through the normal per-deck implicit catalog, so the kernel receives the
  two implicits exactly like a web Mystery pair). Empty/pending layouts warn
  to stderr and fall back to stock.
- **Experimental additive Archive** (`experimental.archive_additive` /
  `--archive-additive`): Archive keeps its self-compounding `base^n_arcane`,
  but the factor **joins the core stack** — `base^n − 1` is added alongside
  the other cores (×-composed under multiplicative stacking) — instead of
  multiplying every card's whole contribution from outside the stack.
  Coverage is unchanged (baseline applies to every scoring card, exactly the
  set the outside factor used to hit); only the composition softens. Default
  OFF everywhere; the web app never sets it (wasm payload field defaults
  false; the deployed wasm is unchanged until the next wasm-pack build).

### Upstream final Archive formula + implicit retunes (2026-07-14, wv 0e54a67f)

- **Archive is now `base^(2.1·√n_arcane)`** in every model surface (tagged
  kernel, both classic kernels, the Python reference `simulate()`, the
  heatmap, and the TS mirrors). Still an outside-the-stack whole-contribution
  multiplier; only the exponent function changed. The short-lived
  `experimental.archive_additive` toggle is removed (superseded).
- **Implicit value sync** (`decks/wolds_implicits.json`): treasure 1.0→1.25,
  merchant 0.5→1.0, cactus 2.0→2.5, **bishop 3.0→2.0**, snake 0.075→0.05.
