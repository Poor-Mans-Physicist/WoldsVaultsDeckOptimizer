# Inventory-Based Optimizer — Implementation Plan

## Goal

Add a **new, parallel optimizer** that takes a concrete card inventory (per type + color) and a concrete core inventory (with optional per-core multiplier overrides), runs a single-deck SA against it, and returns the best assignment. Designed to power an interactive GUI (NiceGUI, separate work item) where the user picks a deck, types in their card counts, and gets a result in ~10–20 seconds.

The existing batch optimizer (`src/simulate.py`, `src/main.py`, `ndm_core::run_sa_optimize`) stays untouched. The new optimizer lives alongside it, reusing `Deck`, `CardType`, and the SA scaffolding pattern but with a fresh scoring kernel that understands colors.

## What's new vs. the existing optimizer

| Concept           | Existing                                        | New                                                                   |
| ----------------- | ----------------------------------------------- | --------------------------------------------------------------------- |
| Card identity     | `CardType` only                               | `(CardType, Color)`                                                 |
| Card supply       | Unlimited per type                              | Bounded by inventory counts (per `(type, color)`)                   |
| Positional counts | Count all filled slots                          | Count only same-color filled slots                                    |
| Color core        | Single global multiplier on all non-greed cards | Per-color core; only boosts cards of matching color; only one in deck |
| Core multipliers  | Always from `config.yaml`                     | Per-core override allowed; falls back to config                       |
| Card-class flag   | Per-deck-config                                 | Per-run argument                                                      |
| Empty slots       | Stay empty (sentinel)                           | Filled by transparent "dead cards" once inventory is exhausted        |
| Parallelism       | Multiprocessing across decks                    | Parallel restarts inside Rust (`rayon`); one deck per call          |

## Data model

### `Color` enum

```python
class Color(Enum):
    RED    = "red"
    GREEN  = "green"
    BLUE   = "blue"
    YELLOW = "yellow"
```

### Card identity

A card is a specific `(CardType, Color)` pair. **The inventory dict is a hard manifest**: each entry is a specific stack the user owns, and only the exact `(type, color)` combinations present in the dict can ever appear on the deck. A `(ROW, BLUE)` card cannot be placed unless `(ROW, BLUE)` is in the inventory with remaining count > 0 — there is no mechanism to "change a card's color" because that would conjure a card the user doesn't own. The full inventory key space:

- 4 positional types × 4 colors = 16 positional stacks (ROW/COL/SURR/DIAG × red/green/blue/yellow)
- 10 greed types × 4 colors = 40 greed stacks (8 dir greeds + EVO_GREED + SURR_GREED, each × 4 colors)
- 1 deluxe type × 4 colors = 4 deluxe stacks
- 1 typeless type × 4 colors = 4 typeless stacks

**Greed color matters for scoring**, not just for bookkeeping. A positional card counts every same-color card in its scan range regardless of that card's type — including greed cards. A green ROW card in a row with 4 other green greed cards has a positional value of 5 (itself + 4 same-color peers). The greed card's own mechanics still don't care about color (its target buff is colorless), but its color contributes to neighbors' positional counts.

**Typeless cards are budgeted like any other card.** Each `(TYPELESS, color)` stack lives in the inventory dict. Typeless cards behave like positional cards in every way *except* their positional multiplier is fixed at 1.0 (no per-card scaling from peers). They contribute NDM, they receive core multipliers (including the color core when colors match), they get greed boosts, and they count toward same-color positional totals for other cards.

The **dead card** is the *only* placeable thing not in the inventory dict. It's the filler used when inventory is exhausted before the deck is full. Dead cards are fully transparent: no NDM contribution, no greed receipt, no participation in any positional same-color count.

### `CardInventory`

```python
@dataclass
class CardInventory:
    counts:     Dict[Tuple[CardType, Color], int]   # "I have up to N of this stack"
    card_class: CardClass                           # SHINY or EVO (flag — cards don't mix)
    cores:      "CoreInventory"
```

Semantics: "up to N." No minimums (per user spec). If sum(counts) < deck slot count, dead cards fill the rest.

### `CoreInventory` and `CoreSpec`

```python
@dataclass(frozen=True)
class CoreSpec:
    core_type: CoreType        # PURE, EQUILIBRIUM, STEADFAST, FOIL, DELUXE_CORE, COLOR
    color:     Optional[Color] # only set when core_type == COLOR
    override:  Optional[float] # user-supplied multiplier (see below)

@dataclass
class CoreInventory:
    cores: Set[CoreSpec]
```

`cores` is a **set** of available `CoreSpec`s. At most one of each core_type, except COLOR which can have one per color (so a user can own COLOR_RED + COLOR_BLUE simultaneously). The candidate enumerator will still only ever **place** one color core per deck (game rule), but enumerates over which color to try.

#### Override semantics (per user spec)

| Core                                | What `override` replaces                                                                                                                              |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| EQUILIBRIUM, STEADFAST, COLOR, FOIL | The full multiplier value. If `None`, use config value.                                                                                               |
| PURE                                | The **scale** term only. The base term and `n_ns` (which depends on the live assignment) are unchanged. Formula stays `base + scale * n_ns`. |
| DELUXE_CORE                         | The**scale** term only. Formula stays `base + scale * n_deluxe`.                                                                                |

If `override` is `None` for any core, that core's relevant config value is used as the default.

## Scoring model changes

### Color-aware positional counts

Each position now tracks per-color filled counts. **The count includes every non-dead card of that color regardless of type** — positional cards, greed cards, deluxe cards, and typeless cards all participate in their color's count.

- `row_count[r][color]` — number of `color` cards in row `r`, of any type, including the card itself if applicable
- `col_count[c][color]` — same for column
- For SURR/DIAG, we walk `deck._surr_peers[p]` / `deck._diag_peers[p]` and count peers whose color matches the placed card's color (any type)

**Dead cards do not contribute to any of these counts.** They are transparent — they occupy a slot for the purpose of "this slot is filled," but the scoring math treats them as if the slot is empty.

**Concrete example.** A green ROW card at `(r, c)` in a row also containing: 2 green dir-greed-up cards, 1 green deluxe, 1 red row card, 1 blue greed card. Its positional value is `row_count[r][green] = 4` (itself + 2 green greeds + 1 green deluxe). The red and blue cards contribute nothing to the green count.

### Color core gating

The COLOR core is no longer a flat multiplier in `core_mult`. It splits into two terms:

```
core_mult           = product/sum of (PURE, EQUILIBRIUM, STEADFAST, FOIL)  # applies to all
deluxe_core_mult    = (DELUXE_CORE if present, else 1.0)                   # applies to non-deluxe
color_core_mult[c]  = (COLOR override or MULT_COLOR if active core's color == c, else 1.0)
```

Per-card contribution becomes:

```
red row card at p:       pos * core_mult * color_core_mult[red] * deluxe_core_mult * boost
red typeless card at p:  1.0 * core_mult * color_core_mult[red] * deluxe_core_mult * boost   (pos fixed at 1.0)
red deluxe at p:         MULT_DELUXE_FLAT * core_mult * color_core_mult[red] * boost   (no deluxe_core_mult — deluxe core never boosts deluxes)
```

`ADDITIVE_CORES` stacking still applies to `core_mult` and `deluxe_core_mult` exactly as today; the color core's contribution is a separate per-card factor.

### Greed and EVO_GREED

Greed targeting math is unchanged. Greed cards have a color in inventory but the boost they apply is colorless and acts on whatever card it points at (subject to existing rules — EVO_GREED only buffs EVO regular cards in the slot below it). Greed cards never receive any multiplier themselves (they don't contribute NDM at all).

## Initial fill heuristic

Per user spec: **Surr → Row → Col → Diag → Deluxe**, with typeless filling after deluxe and greed cards seeded only by SA proposals (not in initial fill).

### Algorithm

1. **Precompute "best slot" rankings** per card type, using deck geometry only (same approach as existing `_precompute_best_positional`). For each `CardType in (SURR, ROW, COL, DIAG)`, rank deck slots by the *maximum possible* peer count for that type (treats all peers as same-color, since geometry doesn't know colors yet).
2. For each type in order `SURR, ROW, COL, DIAG`:
   - Determine which color has the most cards of that type in inventory; place those first.
   - Continue to the next color (also by count of this type), and so on.
   - Cards are placed into the type's slot ranking, top-down, skipping any slot already filled.
   - Decrement inventory as each card is placed.
3. After all positional types are placed (either inventory exhausted or no slots left), place **deluxe** cards into the remaining open slots, color order by stack size.
4. Then place **typeless** cards into any still-open slots, color order by stack size. Typeless contributes only 1.0 × multipliers, but still adds to same-color positional counts for neighbors and earns the color-core bonus, so it's a non-trivial filler.
5. Any slot still empty after step 4 receives a transparent **dead card**.

The result is a feasible, geometrically-plausible starting state for SA. SA then proposes single-cell replacements and pair-swaps from there.

### Why not include greed in the initial fill?

Greed cards' value depends entirely on what they point at, which depends on the placement of every other card. Seeding them up front is harder to do correctly than letting SA discover their best positions through swap moves. Trading a slightly worse starting NDM for simpler initial-fill code is a good trade.

## SA changes

The simulated annealing loop pattern stays the same as today (cooling schedule, accept/reject). Only two proposal moves exist — there is no color-flip move, because inventory entries are concrete and you cannot "change a card's color" without conjuring a card you don't own. The differences from today's SA:

1. **Inventory counter**: a `Dict[(CardType, Color), int]` tracking how many of each stack are currently placed. Mirror in Rust as a flat 2D `[N_TYPES][N_COLORS]` array of `u32`.
2. **Proposal validation**:
   - **Single-cell replace** (current `(T_old, C_old)` → new `(T_new, C_new)`):
     - The new `(T_new, C_new)` must be either the dead card OR have `placed[(T_new, C_new)] < inventory[(T_new, C_new)]`. Otherwise reject without scoring.
     - Candidate `(T_new, C_new)` choices are drawn from `{ keys of inventory } ∪ { DEAD }`, so the proposal step never even considers a `(type, color)` the user doesn't own.
   - **Pair-swap**: always inventory-neutral; no extra check needed.
3. **Move-type biasing**: 80/20 swap/replace, same as today. Revisit only if SA convergence suffers under tight inventory.
4. **Min-regular / max-greed constraints**: dropped. The inventory implicitly bounds card counts; there are no aggregate constraints.

## Candidate cores

**Reuse the existing `candidate_cores()` logic** in `src/simulate.py` as the foundation. Copy/adapt rather than reinvent — the Shiny/EVO grouping logic and the variable-vs-static-core handling are correct and battle-tested. Three things change:

- **Color core enumeration**: in addition to today's "no color core / one color core" branches, enumerate over *which* color core to try. For each color present as a `COLOR_X` spec in `inventory.cores`, that becomes its own candidate option. "No color core" is also a candidate, as today.
- **Inventory restriction**: a core only enters a candidate if it's in `inventory.cores`. The existing function assumes all cores are always available; the new version filters by inventory.
- **Per-core multiplier overrides**: when computing the static-core ranking inside the candidate-cores helper, use the override value (if present in the `CoreSpec`) instead of the config constant. PURE and DELUXE_CORE remain *variable* cores — their override only affects the `scale` term, the rest of the formula (`base + scale * n_ns` / `base + scale * n_deluxe`) is unchanged, and `n_ns` / `n_deluxe` are still measured at runtime from the live assignment.

Cap is still `≤ deck.core_slots`.

## Rust core

### File structure

- New file `ndm_core/src/inventory.rs` containing:
  - `run_sa_inventory(...)` — the new entry point exposed to Python
  - Internal scoring kernel parallel to `simulate()` but color-aware
- `ndm_core/src/lib.rs` adds the new function to its `#[pymodule]` block and `mod inventory;`. Existing `run_sa_optimize` is untouched.

### Parallelism

Each call to `run_sa_inventory` runs N restarts in parallel using `rayon`. Each restart:

1. Builds its own initial assignment (with its own RNG seed) via the initial-fill heuristic.
2. Runs SA for `n_iter` steps.
3. Returns its best `(assignment, score)`.

The Rust entry returns the best result across all restarts (and optionally the top K, if we want the GUI to show alternates).

`rayon` is the natural choice — `Cargo.toml` change: add `rayon = "1"` to `[dependencies]`. Restart count exposed as a kwarg from Python; defaults to `num_cpus::get()` or a config value.

### Data layout

Color and type counts use flat arrays for cache-friendliness:

```rust
const N_TYPES: usize = 16;  // or however many we end up with after stripping unused ones
const N_COLORS: usize = 4;

type ColorCounts = [u32; N_COLORS];        // for row_count[r], col_count[c]
type Inventory    = [[u32; N_COLORS]; N_TYPES];
```

No hash maps in the hot loop.

## Python wrapper

New module `src/inventory_optimize.py` exposes:

```python
def optimize_inventory(
    deck:      Deck,
    inventory: CardInventory,
    n_iter:    int = 60_000,
    restarts:  int | None = None,   # None → use all cores
) -> InventoryResult:
    """Run inventory-constrained SA and return the best assignment + score + diagnostics."""
```

`InventoryResult` carries the assignment, NDM, the cores used (after enumeration picked the winner), and a per-slot breakdown for the heatmap. Pure-Python fallback path mirrors the structure of the existing `_sa_optimize_python` so it's testable without Rust.

The wrapper handles candidate-core enumeration and calls Rust once per candidate (or once with all candidates pre-passed, TBD — depends on how cheap the per-call overhead is).

## What's explicitly NOT in this plan

- **GUI**: NiceGUI work is separate. This plan covers only the optimizer that the GUI will eventually call.
- **Delta evaluation / incremental scoring**: the SA kernel re-evaluates the full assignment each iteration, same as the existing one. If 10–20s ends up being too slow after benchmarking, that's the optimization to add next.
- **Pinned slots**: "force this slot to be a deluxe" isn't included; can be added later by extending the proposal step.
- **Min-regular / max-greed**: dropped from this optimizer (inventory replaces them).
- **Spreadsheet export**: this optimizer's output goes to the GUI, not xlsx.

## Implementation phases

Listed in build order; each phase is independently testable.

1. **Data model + initial fill (Python only)**
   - `Color` enum
   - `CardInventory`, `CoreSpec`, `CoreInventory` dataclasses
   - `_initial_fill()` function with the surr→row→col→diag→deluxe heuristic
   - Unit test: feed an inventory, assert the initial assignment respects counts and uses the right slots
2. **Python scoring kernel** (`simulate_inventory()`)
   - Color-aware row/col/surr/diag counts
   - Color core gating
   - Deluxe cards count toward positional same-color totals
   - Dead-card transparency
   - Unit test: hand-crafted assignments with known NDM
3. **Python candidate-core enumeration**
   - Color core dimension
   - Inventory restriction
   - Override-aware ranking
   - Unit test: enumeration produces expected sets for sample inventories
4. **Python SA loop**
   - Inventory counter
   - Replace / swap moves with inventory validation (no color-flip — inventory is concrete)
   - Unit test: SA on a tiny deck converges to a known-optimal answer (or close)
5. **Rust port** (`inventory.rs`)
   - Mirror the scoring kernel with flat arrays
   - Add `rayon` for parallel restarts
   - Expose `run_sa_inventory` via PyO3
   - Parity test: same seed, same inputs → identical NDM in Python and Rust paths
6. **Python wrapper + Rust dispatch**
   - `optimize_inventory()` orchestrates candidate enumeration and dispatches to Rust (or pure-Python fallback)
   - Returns `InventoryResult` with diagnostics
7. **Benchmarking**
   - Realistic deck × full inventory, measure wall time
   - If well under 20s: done. If over: add delta evaluation as Phase 8.

Phases 1–4 are roughly a day each. Phase 5 is the slowest at 2–3 days. Phases 6–7 round out the week. Total ballpark: **5–10 working days** for an unoptimized but correct end-to-end pipeline, before any GUI work.

## Open questions to revisit before implementation

- Whether to return top-K candidates from Rust (for "show me alternate near-optimal layouts" in the GUI). Decide before Phase 5 wrap-up.
