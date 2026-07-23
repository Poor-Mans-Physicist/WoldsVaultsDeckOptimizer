# Wold's Vaults Deck Optimizer

Computes optimal card-deck layouts for [Wold's Vaults](https://github.com/iwolfking/Wolds-Vaults) by maximizing **NDM** with a simulated-annealing kernel written in Rust. Two frontends share the same kernel, math, and config:

- **Web app (recommended):** <https://poor-mans-physicist.github.io/WoldsVaultsDeckOptimizer/> — runs entirely in your browser (WASM), nothing to install. Max / Targeted / Exact modes, deck implicits, structural cores, Complex Cards, per-slot math breakdowns, snapshots, and a deck builder.
- **CLI spreadsheet optimizer:** batch-runs *every* deck × both card classes × several greed-constraint configs and writes a full `.xlsx` panel with layouts and heatmaps. This is what the balance sheets are made with.

## What NDM means

NDM is the total multiplier a single card type would receive if every card-bearing (non-greed) slot were filled with it. A deck with NDM = 100 filled with +1% HP cards gives +100% HP. Check the per-slot heatmaps too — greed concentrates the multiplier into few cards.

Two card classes are optimized separately: **Shiny** (stat cards; stat cores apply, lower base stats) and **Evo** (evolution cards; no stat cores, higher base stats). NDM does not include that base-stat difference, so compare within a class, not across.

## CLI spreadsheet optimizer

### Requirements

- [`uv`](https://docs.astral.sh/uv/) — manages Python, the venv, and dependencies:
  - Windows (PowerShell): `powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"`
  - macOS / Linux: `curl -LsSf https://astral.sh/uv/install.sh | sh`
- **Rust toolchain** (<https://rustup.rs>) — the SA kernel is a compiled extension; `uv` builds it automatically. There is no pure-Python fallback.

### Run

```
git clone https://github.com/Poor-Mans-Physicist/WoldsVaultsDeckOptimizer.git
cd WoldsVaultsDeckOptimizer
uv run optimize
```

The first run compiles the Rust kernel (a minute or two); later runs start immediately. One process per deck runs in parallel, then a summary prints and the spreadsheet is written.

### Flags

| Flag | Effect |
|------|--------|
| `--mode vanilla` | Vanilla Vault Hunters rules and core values (default mode is `wolds`) |
| `--no-implicits` | Score bare layouts with deck implicits disabled (balance comparisons) |
| `--structural-cores` | Use the curated greater Construction/Arcane-core builds for the Wold / Fairy / Mystery decks (`decks/structural_layouts.json`); their core budgets shrink to pay for the placed structural cores, and Mystery runs its runic + bishop pair |

### Output

`Panel_WV_Decks_real_ndm_simulation.xlsx` in the repo root — per deck × constraint config × class: NDM, chosen cores, card counts, the layout grid, and a per-slot NDM heatmap. **An existing spreadsheet is never overwritten — delete or rename it before re-running.**

Layout-grid legend: `R`/`C`/`S`/`X` = row/col/surround/diagonal scaling cards, `D` deluxe, `T` typeless, `a` arcane, `_` dead, `^ v < >` directional greeds, `e` evo-greed, `o` surround-greed, `.` filler greed, `·` empty.

### Tuning

- `config.yaml` — search budget (`testing: n_iter / restarts`), spreadsheet export toggle, per-mode core values and gates, `engine: tagged` (default; `classic` keeps the 1.x color-blind kernel used as a validation baseline).
- `decks/` — deck shapes, constraints, implicit values (`wolds_implicits.json`), structural layouts.
- `modifiers.json` / `vh_modifiers.json` — the card rosters (Wold's / vanilla).

### Other scripts

- `uv run python scripts/implicit_impact.py` — one-pass balance leaderboard (screenshot-ready HTML + markdown); `--compare` runs the implicit on/off diff instead.
- `uv run python scripts/parity_2_0.py --fast` — validation gate: proves the Python reference and both Rust kernels score identically.

## Repo layout

| Path | What |
|------|------|
| `src/` | CLI pipeline (config, SA driver, report/xlsx) |
| `ndm_core/` | Rust kernels (PyO3 extension; the tagged 2.0 kernel is shared with the web build) |
| `wasm-port/` | The web app — Svelte + WASM; see [wasm-port/web/README.md](wasm-port/web/README.md) to run it locally |
| `config.yaml`, `decks/`, `modifiers*.json` | Single source of truth for both frontends |

## Docs

- [MODELING_CHOICES.md](MODELING_CHOICES.md) — how every game mechanic is modeled, with the receipts.
- [WV_DECK_OPTIMIZER_2.0.md](WV_DECK_OPTIMIZER_2.0.md) — the Optimizer 2.0 design spec.
