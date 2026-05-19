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

## Running

Everything is driven by `uv`. **Never invoke `python` directly** — always go through `uv run`.

| Command | Notes |
| --- | --- |
| `uv run optimize-py` | Pure-Python SA (no Rust toolchain needed). Default mode is `wolds`. |
| `uv run optimize-py --mode vanilla` | Vanilla preset (multiplicative cores, no positional shiny, no deluxe). |
| `uv run --extra rust optimize` | Same optimizer, but uses the Rust core (`ndm_core`) — much faster. First run compiles the extension. |
| `uv run optimize-py --help` | CLI help (only flag is `--mode`). |

The Rust extension is rebuilt automatically by `uv` whenever `ndm_core/Cargo.{toml,lock}` or `ndm_core/src/**/*.rs` change (see `[tool.uv].cache-keys` in [pyproject.toml](pyproject.toml)).

There are no tests, no lint config, no CI. The optimizer is the only entry point.

> The optimizer skips spreadsheet export if `Panel_WV_Decks_ndm_simulation.xlsx` already exists. Delete or rename the previous run before re-running if you want a new file.

## Architecture

### Module layout (Python)

```
optimizer.py     thin CLI shims: optimize / optimize-py (the latter sets
                 sys.modules["ndm_core"] = None to force the pure-Python path)
src/
  config.py      parses --mode, loads config.yaml + decks/*, exposes UPPERCASE
                 module constants, defines the Deck class
  types.py       CardType / CardClass / CoreType enums + PLACEABLE list
                 (PLACEABLE is mutated at import time when ALLOW_DELUXE)
  simulate.py    simulate() scoring kernel, candidate_cores() enumerator,
                 sa_optimize() (Rust-or-Python dispatcher), _sa_optimize_python()
  main.py        optimize() orchestrator + multiprocessing entry point (main())
  report.py      terminal heatmaps, HNS metric, openpyxl spreadsheet export
ndm_core/        PyO3 Rust crate exposing run_sa_optimize(); single file lib.rs.
                 Built via maturin, declared as an optional extra in pyproject.
decks/           *.yaml (hand-curated) and/or *.json (game-data dumps).
                 See decks/README.md for schema and collision rules.
config.yaml      every tunable (greed/core multipliers, stacking modes,
                 test panel configs, etc.). `modes.<name>` deep-merges over
                 the defaults when --mode <name> is selected.
```

### Import-time side effects (important)

`src/config.py` does real work at import:

1. Parses `--mode` from `sys.argv` (via `parse_known_args`, so other flags pass through).
2. Loads `config.yaml`, deep-merges the selected `modes.<mode>` block over it.
3. Binds every tunable as an UPPERCASE module constant (`MULT_PURE_BASE`, `GREED_ADDITIVE`, etc.). **These are read once at import — never mutate them.**
4. If `ALLOW_DELUXE` is true, appends `CardType.DELUXE` to the shared `PLACEABLE` list in `types.py`.
5. Scans `decks/` and builds `DECKS: List[Deck]` (YAML first, then JSON — JSON entries are dropped if their `<key>` collides with a YAML filename stem stripped of any `NN_` prefix).

When adding a new tunable: add it to `config.yaml`, read it in `config.py` as a module constant, import it where needed. If the Rust core needs it, also thread it through the kwargs of `_ndm_core.run_sa_optimize(...)` in `src/simulate.py` and the matching signature in `ndm_core/src/lib.rs`.

### Python ↔ Rust dispatch

`src/simulate.py` tries `import ndm_core` at module load. On success `_RUST_AVAILABLE = True`; on `ImportError` it prints a fallback notice and uses `_sa_optimize_python`. The pure-Python path is the spec — when changing scoring rules, update both `simulate()` (Python) and the equivalent code in `ndm_core/src/lib.rs`. Card-type and core-type **string values must stay in sync** between `CardType`/`CoreType` enums and the `card_type_from_str` / `core_type_from_str` matchers in `lib.rs`.

`optimize-py` disables the Rust core by stuffing `sys.modules["ndm_core"] = None` **before** importing `src.main` (which transitively imports `src.simulate`). Preserve this ordering if you refactor the entry points.

### Execution model

`src/main.py::main` runs each deck on its own process via `multiprocessing.Pool` (one worker per deck, capped at CPU count). Each worker iterates `_get_test_configs(deck)` (panel configs from `config.yaml`, or the deck's own `min_regular`/`max_greed` if `testing.full_panel: false`) and calls `optimize()`, which itself runs `candidate_cores × restarts` SA invocations per `CardClass`. Per-worker `random.seed()` is called for randomized starts. Reporting/spreadsheet generation happens back in the parent after all workers return.

### Adding decks

Drop a `*.yaml` (or game-data `*.json`) into `decks/`. Layout grid: `O` = placeable, `A` = arcane (counted, not placed), anything else = empty. JSON dumps with `socketCount: null` (dungeon-only variants) are skipped. To skip a deck without deleting its file, add its dedup key to `excluded_decks` in `config.yaml`. Full schema and collision rules in [decks/README.md](decks/README.md).

### Adding a new card type

Touch points: `CardType` enum + categorize via `GREED_TYPES`/`REGULAR_TYPES`/`DELUXE_TYPES`/`TYPELESS_TYPES` and the `PLACEABLE` list in `src/types.py`; greed effect handling in `simulate()` in `src/simulate.py`; display char in `Deck._CHAR` in `src/config.py`; matching `u8` constant + `card_type_from_str` arm + greed/scoring arms in `ndm_core/src/lib.rs` (header comment lists the exact spots to touch).
