# Modeling Choices

This file is the source-of-truth specification for **how the Vault Hunters
Deck Optimizer scores a deck**. It pins down every multiplier, every
class-gating rule, every counting rule for cores and cards.

Authoritative behavior described here = the **WASM web app**
(`wasm-port/web/` + `wasm-port/ndm_core/src/inventory.rs`). The desktop
Python+Rust spreadsheet CLI and the desktop NiceGUI inventory tool track
the same model except where called out under **Platform discrepancies**
at the bottom. Anything not under that section is identical across all
three.

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
4. **Greed boosts.** Every greed card applies its target-specific boost
   to a `boost: position → float` map (only scorable targets receive it).
5. **Sum NDM per category:**
   - regular: `pos_count × regular_core_mult × boost`
   - deluxe:  `MULT_DELUXE_FLAT × deluxe_card_core_mult × boost`
   - typeless: `1.0 × typeless_core_mult × boost`
   - arcane: 0 (never scores directly, but counts in row/col peer
     counts for adjacent positionals, and counts in `n_ns`).
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
- The arcane-auto-place=OFF toggle (web app / NiceGUI) expands the
  inventory SA's per-arcane-slot proposal alphabet to include DEAD as
  well — useful when void is on. When auto-place is ON, arcane slots
  stay locked to ARCANE (with color-only swaps allowed).

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

Cores **never** apply to greed cards. They never apply to ARCANE cards
(arcane = 0 NDM, fixed). DEAD cards score 0 regardless and so are not
affected.

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

### 1. `n_ns` for EVO-no-FOIL (status: aligned — historical drift)

Until the `web app n_ns matches classic kernel` commit, the WASM web app
counted a **wider** set under EVO-no-FOIL than the classic CLI:

| Path                                                                                                                           | EVO-no-FOIL `n_ns`                                   |
| ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------ |
| WASM web app + classic CLI + classic Rust                                                                                      | regulars + arcane + greed                              |
| **NiceGUI desktop** (`src/inventory_optimize.py::simulate_inventory` + `_breakdown`) | regulars +**deluxe + typeless** + arcane + greed |

The NiceGUI still uses the wider formula. To align it, drop `deluxe.size`
and `typeless.size` from both n_ns lines in `inventory_optimize.py`.
(Pending — see follow-up in the unification task.)

### 2. Color-aware vs color-blind scoring

The **WASM web app and the desktop NiceGUI** use the inventory-color-aware
optimizer (`wasm-port/ndm_core/src/inventory.rs` and
`src/inventory_optimize.py`). In that model:

- Positional peer counts (`row_count`, `col_count`, surr/diag) consider
  **only same-color cards** in scan range.
- `COLOR` core only boosts cards whose color matches the core's color
  selection.

The **classic spreadsheet CLI** (`src/simulate.py` + `ndm_core/src/lib.rs`)
is **color-blind**: every filled neighbor counts for positional peers
regardless of color, and COLOR's `1.75×` applies to every scorable card
flat. This is a real semantic difference in the SA's optimum, not a bug —
the CLI predates the color-aware model and serves as the simpler
spec-style implementation.

### 3. `wasm-port/ndm_core/src/batch.rs` arcane model (status: vestigial)

This file uses the older "arcane = deck-level slot count" model with an
explicit `+ deck.n_arcane` fudge inside the Pure-core arm — instead of
counting placed ARCANE cards as real placements. Numerically equivalent
for now, but it's the pre-arcane-card design. It's not on the live web
app path (live runs go through `inventory.rs::runSaInventory`). Will be
reconciled with the canonical model during the wasm-port unification
follow-up.

### 4. `wasm-port/src/` Python copies (status: vestigial)

`wasm-port/src/simulate.py`, `wasm-port/src/report.py`,
`wasm-port/src/inventory_optimize.py` are pre-arcane snapshots from when
`wasm-port/` was a sibling fork. They lack arcane-card handling entirely
and predate the void/arcane work. None of them are used by anything
live (the web app uses TypeScript; the desktop uses outer `src/`). Will
be deleted during the unification follow-up.

---

## When you change scoring code, also update this file

Touch points to keep in sync, by surface:

- **Add a new card type** → update the **Card types** table + (if greedy)
  the **Greed cards** table.
- **Add or rename a core** → **Cores** table + relevant gating sections.
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
