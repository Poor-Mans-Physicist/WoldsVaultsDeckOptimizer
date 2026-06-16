# Modeling Choices

This file is the source-of-truth specification for **how the Vault Hunters
Deck Optimizer scores a deck**. It pins down every multiplier, every
class-gating rule, every counting rule for cores and cards.

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
   `archive_core_base ^ n_arcane_placed` when ARCHIVE_CORE is picked,
   else `1.0`. Bypasses the additive-vs-multiplicative stacking switch
   entirely; applied as a final outside-the-stack factor on every
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
| `DIAG`                   | `pos_count × core_mult × boost`                    | NW-SE plus NE-SW diagonal peer count + 1 (self)      | EVO + SHINY-with-positional | EVO-no-FOIL only                         |
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
| `DIR_GREED_UP`    | `(r-1, c)` directly above                | `greed.dir_vert`      | **5**   |                                                                                                               |
| `DIR_GREED_DOWN`  | `(r+1, c)` directly below                | `greed.dir_vert`      | **5**   |                                                                                                               |
| `DIR_GREED_LEFT`  | `(r, c-1)` directly left                 | `greed.dir_horiz`     | **5**   |                                                                                                               |
| `DIR_GREED_RIGHT` | `(r, c+1)` directly right                | `greed.dir_horiz`     | **5**   |                                                                                                               |
| `DIR_GREED_NE`    | `(r-1, c+1)`                             | `greed.dir_diag_up`   | **0**   | Diagonal greeds are inert at default 0                                                                        |
| `DIR_GREED_NW`    | `(r-1, c-1)`                             | `greed.dir_diag_up`   | **0**   |                                                                                                               |
| `DIR_GREED_SE`    | `(r+1, c+1)`                             | `greed.dir_diag_down` | **0**   |                                                                                                               |
| `DIR_GREED_SW`    | `(r+1, c-1)`                             | `greed.dir_diag_down` | **0**   |                                                                                                               |
| `EVO_GREED`       | `(r+1, c)` directly below                | `greed.evo`           | **0**   | **EVO-class-only**, and **only** if target is a regular positional (not typeless, deluxe, arcane) |
| `SURR_GREED`      | All 8 surrounding peers (within ≤ 1 step) | `greed.surr`          | **0**   | Applies to every scorable peer independently                                                                  |

### Greed stacking — additive vs multiplicative

Controlled by `stacking.greed_additive` (default **true** in both modes).

- **Additive** (`true`): each greed pointing at a slot contributes its
  raw multiplier value to a running sum. Final boost =
  `max(1.0, Σ amount_i)` over all greeds hitting the slot — the `max`
  floor handles the no-greed case so the slot doesn't drop to 0× boost.
  Worked examples (default `dir_vert: 5`):

  | Greeds pointing at slot | Final boost |
  | --- | --- |
  | 0                            | 1.0  |
  | 1× dir_vert                  | 5    |
  | 2× dir_vert                  | 10   |
  | 3× dir_vert                  | 15   |
  | 1× dir_vert + 1× surr_greed at 3 | 8 |

- **Multiplicative** (`false`): each greed multiplies the running boost
  starting from 1.0. Final boost = `Π amount_i`. **Not floored** — if any
  contributing multiplier is 0, the slot's contribution becomes 0. This
  is a legacy stacking model; neither Wold's nor Vanilla uses it today.

Implementation: `_apply_greed()` in `src/simulate.py` /
`src/inventory_optimize.py`, the `apply_greed!` macro in
`ndm_core/src/lib.rs`, the inline `apply` closure in
`ndm_core/src/inventory.rs` + `wasm-port/ndm_core/src/inventory.rs`, and
`applyGreed()` in `wasm-port/web/src/lib/breakdown.ts`. The boost map
is initialized to `0` (additive) or `1` (multiplicative) at the start of
every scoring call.

---

## Cores

| Core            | Default value (Wold's)                  | What it boosts                                                          | What it does NOT boost                   | Scaling formula                                          | Class gating                                                                   |
| --------------- | --------------------------------------- | ----------------------------------------------------------------------- | ---------------------------------------- | -------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `PURE`        | base**1.0**, scale **0.07** | regulars, deluxe cards (additive baseline), typeless                    | greed                                    | `pure_base + pure_scale × n_ns` (variable per layout) | Universal                                                                      |
| `EQUILIBRIUM` | **3.0**                           | regulars, typeless                                                      | deluxe cards (additive), greed           | Flat                                                     | **SHINY-only**                                                           |
| `STEADFAST`   | **2.2**                           | regulars, typeless                                                      | deluxe cards, greed                      | Flat                                                     | **SHINY-only**                                                           |
| `COLOR`       | **1.75**                          | every scorable card (WASM model: only matching-color cards)             | greed                                    | Flat                                                     | Universal                                                                      |
| `FOIL`        | **2.8**                           | regulars, deluxe cards (baseline), typeless                             | greed                                    | Flat                                                     | Universal;**also flips EVO's `n_ns` to the SHINY formula** (see below) |
| `DELUXE_CORE` | base**1.0**, scale **0.2**  | regulars, typeless                                                      | **deluxe cards themselves**, greed | `deluxe_core_base + deluxe_core_scale × n_deluxe`     | Universal; gated by `deluxe.allow` (off in vanilla)                          |
| `VOID_CORE`   | base**1.0**, scale **0.3**  | regulars, deluxe cards, typeless                                        | dead cards themselves, greed             | `void_base + void_scale × n_dead`                     | Universal; gated by `cores.void_allow` (off in vanilla)                      |
| `ARCHIVE_CORE` | per-arcane base **1.2**           | regulars, deluxe cards, typeless                                        | greed (arcane/dead score 0 anyway)       | `archive_core ^ n_arcane_placed` — applied **outside** the per-card `core_mult` (see callout below) | Gated by `cores.archive_allow` (off in vanilla); when on, additionally **enumerated only when the deck has ≥ 1 arcane slot** |

Cores **never** apply to greed cards. They never apply to ARCANE cards
(arcane = 0 NDM, fixed). DEAD cards score 0 regardless and so are not
affected.

### Archive core — the only "outside-the-stack" core

Every other core folds into one per-card `core_mult` that respects the
`stacking.additive_cores` flag (sum in Wold's, product in Vanilla).
Archive does **not**. After all the other math, each scoring card's
contribution is multiplied by an Archive factor of
`archive_core ^ n_arcane_placed`:

```
final_ndm_per_card = base × core_mult × greed_boost × archive_mult
                                                     ^^^^^^^^^^^^
                                                    where archive_mult =
                                                      base_value ^ n_arcane_placed
                                                      (1.0 when Archive isn't picked)
```

Worked example (Wold's default `archive_core: 1.2`):

| Arcane cards placed | Archive factor on every scoring card |
| --- | --- |
| 0 | 1.0  |
| 1 | 1.2  |
| 2 | 1.44 |
| 3 | 1.728 |
| 4 | 2.0736 |

Override semantics: when the user sets an override on Archive, the
override replaces the **per-arcane base**, not the final multiplier. So
an override of `1.5` yields a final factor of `1.5 ^ n_arcane_placed`.
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
> multiplier, `base ^ n_arcane_placed`) is unchanged. The Arcane Core
> is a **structural** core that creates new arcane slots. They synergize
> hard: each Arcane Core conversion bumps `n_arcane_placed` (assuming
> the SA fills the new arcane slot), which compounds the Archive factor.

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
core_mult = 1 + (2.8 − 1) + (1.75 − 1) + (deluxe_value − 1)
          = 1 + 1.8 + 0.75 + (deluxe_value − 1)
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
| `stacking.additive_cores` | **true**       | **false**   | Vanilla multiplies cores instead of summing                                                                 |
| `decks.json_file`         | `wolds_decks.json` | `vh_decks.json` | Different deck rosters per mode                                                                             |

Greed defaults (`dir_vert: 5`, `dir_horiz: 5`, others 0) and core multipliers
(`pure_scale: 0.07`, `equilibrium: 3.0`, `foil: 2.8`, `steadfast: 2.2`,
`color: 1.75`, `void_scale: 0.3`, `deluxe.flat: 2`) are shared.

The UI also relabels in Vanilla: SHINY → "Stat" in class pickers
(`wasm-port/web/src/lib/visibility.ts::classSelectLabel`).

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
