# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# Project: Vault Hunters Deck Optimizer

## Background

This repo is a system designed to optimize the layout of a piece of gear in a minecraft modpack known as a "deck". A "deck" is an object that provides the player stats, and functions by having an array of slots that "cards" can be placed into. Only one card can be in a slot at a time, and cards provide various kinds of boosts to the player. The thing we care about are the multipliers cards can get from other cards, and upgrades a deck can get called "cores". Ultimately, we care about maximizing a number called Net Deck Multiplier (NDM), which is the sum of every card times all of their multipliers, representing the total power of the deck. for example, if a card that gave +1 attack was slotted into every slot in a deck with an NDM of 100, the deck would give the player +100 attack.

## Multipliers

To start, there are two main kinds of cards; shiny, and evo, and any layout can use only one of the two types of cards. Both cards have three subclasses: row, column, and surrounding, and these provide a multiplier to the card based on how many other cards are on the same row, column, etc. as that card, based on it's type. These positional multiplers are additive, and each individual card gets it's own. Both shiny and evo cards have these types. There are also "T" typeless cards that are always 1x regardless.

There are also "greed" cards that can boost directly adjacent cards by giving them an additional multiplier, but provide no stats themselves. The main ones used are for the 4 cardinal directions, but surrounding and other types also exist in the code.

In addition, we also have "cores" which can be applied to a deck based on the number of core slots it has to upgrade them. Cores provide an additonal multiplier to all (non-greed) cards that is multiplicative with all others, and can be multiplicative or additive with itself, with additive working the same way as greed does. Cores might only apply to some card types, with the specifics being in the implementation itself.

Finally, we have "deluxe" type cards, which are similar to T cards but have a flat 3x multiplier instead of a 1x. deluxe cards are special because they fuel the deluxe core, which scales based on the number of deluxe cards in the deck, but the deluxe core does not boost deluxe cards.

## Conventions

- Make comments concise and designed to explain the essence of the most important functions and flows only
- Computationally heavy files should be written in rust, while wrappers and control flow/output handling files should be written in python
- Prefer to use separate easily modifiable config files for scripts instead of having all of the variable declarations in the scripts directly
- Create easily maintainable workflows by splitting files up effectively, avoiding hard to approach monoliths
- **Maintain [MODELING_CHOICES.md](MODELING_CHOICES.md) as the source-of-truth for scoring behavior.** Any code change that touches scoring logic, multiplier values, the `n_ns` formula, core gating, greed mechanics, card categorization, slot rules, stacking modes, or constraint handling must update `MODELING_CHOICES.md` in the **same commit**. The file itself lists which sections to touch under each kind of change. The point is to never have to re-derive intended behavior from the code — if it's not documented there, the change isn't complete.

## Two channels

After the channel-consolidation refactor, this repo ships exactly two user-facing channels — both backed by the same `config.yaml`, `decks/`, and `modifiers.json` at the repo root:

1. **Spreadsheet CLI** (`uv run optimize`) — outer Python orchestrator + outer `ndm_core/` Rust kernel (PyO3). Runs every deck in parallel via multiprocessing (one process per deck; kernel restarts stay serial) and emits `Panel_*.xlsx`.
2. **WASM web app** (`wasm-port/web/`, deployed to GitHub Pages) — Svelte 5 SPA + `wasm-port/ndm_core/` Rust kernel (wasm-bindgen). Interactive; three run modes (Max / Targeted / Exact) with restart-chunk fan-out across a worker pool.

**Optimizer 2.0:** both channels run the SAME tag-aware kernel —
`ndm_core/src/tagsim.rs` is the canonical source, `#[path]`-included by the
wasm crate so the math can't drift. The spreadsheet drives it in Max
configuration (`config.yaml engine: tagged`; `classic` keeps the 1.x kernel
for A/B). Deck implicits live in `decks/wolds_implicits.json` (extracted from
the woldsvaults datagen) and are attached to the web bundle by
`build_data.py`. `scripts/parity_2_0.py` is the validation gate (scoring
equivalence + SA-optimum convergence vs classic) — run it after any kernel
change. The legacy 1.x kernels (`lib.rs`, `inventory.rs` both crates) remain
as the parity baseline. `MODELING_CHOICES.md` is the cross-platform spec
(see its **Optimizer 2.0 addendum**); `src/simulate.py::simulate()` stays the
runnable Python reference for the classic math.

## Running

Everything is driven by `uv`. **Never invoke `python` directly** — always go through `uv run`.

| Command | Notes |
| --- | --- |
| `uv run optimize` | Spreadsheet CLI, default mode `wolds`. First run compiles the Rust extension. |
| `uv run optimize --mode vanilla` | Vanilla preset (multiplicative cores, no positional shiny, no deluxe, no void/archive). |
| `uv run optimize --help` | CLI help (only flag is `--mode`). |
| `uv run python wasm-port/scripts/build_data.py` | Regenerates `wasm-port/web/public/{config,decks,modifiers}.json`. CI does this automatically on push. |

The Rust extension is rebuilt automatically by `uv` whenever `ndm_core/Cargo.{toml,lock}` or `ndm_core/src/**/*.rs` change (see `[tool.uv].cache-keys` in `pyproject.toml`).

There are no tests, no lint config. CI is one workflow (`.github/workflows/deploy.yml`) that builds + deploys the WASM web app to GitHub Pages.

> The CLI skips spreadsheet export if `Panel_WV_Decks_ndm_simulation.xlsx` already exists. Delete or rename the previous run before re-running.

## Architecture

### Spreadsheet CLI (outer)

```
optimizer.py     thin CLI shim: optimize → src.main.main
src/
  config.py      parses --mode, loads ../config.yaml + ../decks/*,
                 exposes UPPERCASE module constants, defines Deck
  types.py       CardType / CardClass / CoreType enums + PLACEABLE
                 (PLACEABLE is mutated at import time when ALLOW_DELUXE)
  simulate.py    candidate_cores() enumerator, sa_optimize() (Rust call),
                 simulate() scoring kernel (runnable spec — unused at runtime)
  main.py        optimize() orchestrator + multiprocessing entry (main())
  report.py      terminal heatmaps, HNS metric, openpyxl spreadsheet export
ndm_core/        PyO3 Rust crate exposing run_sa_optimize().
                 Built via maturin, declared as a regular dep in pyproject.
decks/           *.yaml (hand-curated) and/or *.json (game-data dumps).
                 See decks/README.md for schema and collision rules.
config.yaml      every tunable (greed/core multipliers, stacking modes,
                 etc.). `modes.<name>` deep-merges over the defaults when
                 --mode <name> is selected.
modifiers.json   gear-modifier data, used by the WASM web app's Preview panel.
```

### WASM web app

```
wasm-port/
  scripts/
    build_data.py    reads ../config.yaml + ../decks/ + ../modifiers.json,
                     emits web/public/{config,decks,modifiers}.json
    wasm_*.mjs       perf / parity smoke tests (Node-target wasm build)
  ndm_core/          wasm-bindgen Rust crate; wasm32-unknown-unknown only.
                     inventory.rs is the pure-Rust kernel; wasm_api.rs is
                     the wasm-bindgen entry layer.
  web/               Svelte 5 + Vite SPA. Lib code under src/lib/ re-scores
                     the chosen assignment in TypeScript for the breakdown
                     popup (parity-check against the WASM kernel's score).
```

CI workflow (`.github/workflows/deploy.yml`) runs `build_data.py` from the repo root, builds the wasm crate, builds the SPA, and uploads to GitHub Pages.

### Import-time side effects (important)

`src/config.py` does real work at import:

1. Parses `--mode` from `sys.argv` (via `parse_known_args`, so other flags pass through).
2. Loads `config.yaml`, deep-merges the selected `modes.<mode>` block over it.
3. Binds every tunable as an UPPERCASE module constant (`MULT_PURE_BASE`, `GREED_ADDITIVE`, etc.). **These are read once at import — never mutate them.**
4. If `ALLOW_DELUXE` is true, appends `CardType.DELUXE` to the shared `PLACEABLE` list in `types.py`.
5. Scans `decks/` and builds `DECKS: List[Deck]` (YAML first, then JSON — JSON entries are dropped if their `<key>` collides with a YAML filename stem stripped of any `NN_` prefix).

When adding a new tunable: add it to `config.yaml`, read it in `config.py` as a module constant, import it where needed. If the Rust core needs it, also thread it through the kwargs of `_ndm_core.run_sa_optimize(...)` in `src/simulate.py` and the matching signature in `ndm_core/src/lib.rs`. For the web app you also need to add it to `wasm-port/web/src/lib/config.ts` (ResolvedConfig type) and wire it through the optimizer/breakdown paths + the WASM kernel in `wasm-port/ndm_core/`.

### Python ↔ Rust bridge

`src/simulate.py` imports `ndm_core` at module load (mandatory dep — no fallback). Card-type and core-type **string values must stay in sync** between `CardType`/`CoreType` enums and the `card_type_from_str` / `core_type_from_str` matchers in `ndm_core/src/lib.rs`. The same constraint applies between the outer and wasm Rust crates (their `u8` constants and matcher strings must match).

### Execution model (spreadsheet CLI)

`src/main.py::main` runs each deck on its own process via `multiprocessing.Pool` (one worker per deck, capped at CPU count). Each worker iterates `_get_test_configs(deck)` (panel configs from `config.yaml`, or the deck's own `min_regular`/`max_greed` if `testing.full_panel: false`) and calls `optimize()`, which itself runs `candidate_cores × restarts` SA invocations per `CardClass`. Per-worker `random.seed()` is called for randomized starts. Reporting/spreadsheet generation happens back in the parent after all workers return.

### Adding decks

Drop a `*.yaml` (or game-data `*.json`) into `decks/`. Layout grid: `O` = placeable, `A` = arcane (counted, not placed), anything else = empty. JSON dumps with `socketCount: null` (dungeon-only variants) are skipped. To skip a deck without deleting its file, add its dedup key to `excluded_decks` in `config.yaml`. Full schema and collision rules in [decks/README.md](decks/README.md).

### Adding a new card type

Touch points (in order): `CardType` enum + categorize via `GREED_TYPES`/`REGULAR_TYPES`/`DELUXE_TYPES`/`TYPELESS_TYPES` and the `PLACEABLE` list in `src/types.py`; display char in `Deck._CHAR` in `src/config.py`; greed effect handling in `simulate()` in `src/simulate.py` (the runnable spec); matching `u8` constant + `card_type_from_str` arm + greed/scoring arms in both `ndm_core/src/lib.rs` (outer) AND `wasm-port/ndm_core/src/inventory.rs` (wasm); TypeScript mirror in `wasm-port/web/src/lib/types.ts` + the type-dispatch sites in `cores.ts`, `breakdown.ts`, `optimize.ts`.
