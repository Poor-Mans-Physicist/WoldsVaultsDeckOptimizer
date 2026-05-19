# WoldsVaultsDeckOptimizer

Script that computes the best deck layouts for Wold's Vaults based on a simulated-annealing optimizer. The main simulation logic is written in Rust, and data handling is done in Python. The output is a .xlsx file containing a full breakdown of the decks and their optimal solutions. The main Python code in NDM_Optimizer_Rust.py contains instructions on how to configure the parameters in the script for testing and performance.

In general, this optimizer uses the "NDM", a score that represents the total multiplier a single card type would recieve if every card bearing slot (non greed) were filled with it. For example, a deck with NDM = 100 filled with +1% HP cards would provide the player with +100% HP when equiped. Make sure to check the heatmaps provided with every layout to ensure that the multipliers have the distribution you want; with the way greed works the multiplier often concentrates in just a few cards.

This optimizer checks two classes of decks: "Shiny" stat card based decks, and "Evo" evolution card based decks. Both Shiny and Evolution cards have the same behavior where they scale based off the cards around them (row, column, or surrounding), but Shiny cards can have stat cores applied to them (but have lower base stats on average), and Evo cards cannot have stat cores applied (but have higher base stats to compensate). The NDMs do not reflect this base stat difference, so an Evo deck with NDM = 5000 might be stronger than a Shiny with NDM = 6000 depending on the specific ratio of stat:evo card bases for that card type.

In general, the optimizer takes into account vanilla cores, deluxe cards and the fancy core, and greed cards when computing the optimal layout. Other Wold's cores are not supported. By default, everything is assumed to be ideal and maxed out (all greed cards are 5x, greater cores, etc.). It also has support for a few experimental cores such as the "balance core", which aims to lower overall NDM while improving the even-ness of the spread.

There is a built-in **Vanilla** preset (`--mode vanilla`) that flips the three required toggles for you (multiplicative core scaling on, positional shiny off, deluxe card system off). The default mode is `wolds`.

---

## Quick start (Windows / macOS / Linux)

This project is driven by [`uv`](https://docs.astral.sh/uv/). `uv` installs the right Python version, creates a virtual environment, and installs the Python dependencies — you don't need to install Python yourself.

The Rust core is **optional**. If you have the Rust toolchain installed, you can opt in for a large speedup. If you don't, the pure-Python fallback runs out of the box with no extra setup.

### 1. Install `uv`

**Windows (PowerShell):**
```powershell
powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
```

**macOS / Linux (bash, zsh, fish):**
```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

After installing, restart your terminal so the `uv` command is on your `PATH`. Verify with:
```
uv --version
```

> Alternative installers (Homebrew `brew install uv`, `pipx install uv`, `winget install --id=astral-sh.uv`, etc.) work just as well — anything that gets `uv` on your `PATH` is fine.

### 2. Get the code

```
git clone https://github.com/poor-mans-physicist/woldsvaultsdeckoptimizer.git
cd woldsvaultsdeckoptimizer
```

### 3. Run the optimizer

The same commands work identically on Windows, macOS, and Linux.

#### Without Rust (works out of the box, slower)

No extra setup required. Just run:

```
uv run optimize-py
```

Or with the Vanilla preset:

```
uv run optimize-py --mode vanilla
```

#### With Rust (much faster, requires the Rust toolchain)

First install Rust **once** by visiting [https://rustup.rs](https://rustup.rs) and following the instructions for your OS (accept all defaults). On macOS / Linux this is usually:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On Windows, download and run `rustup-init.exe` from the same page. Restart your terminal afterwards and verify with `cargo --version`.

Then opt into the Rust extension by passing `--extra rust` to `uv`:

```
uv run --extra rust optimize
```

Or with the Vanilla preset:

```
uv run --extra rust optimize --mode vanilla
```

The first run with `--extra rust` will compile `ndm_core` (takes a minute or two); subsequent runs are instant. Re-running without `--extra rust` afterwards is fine — `uv` keeps the Rust core in your environment.

#### Command summary

| What you want                                      | Command                                       | Needs Rust? |
|----------------------------------------------------|-----------------------------------------------|-------------|
| Pure Python, Wold's mode (default)                 | `uv run optimize-py`                          | No          |
| Pure Python, Vanilla mode                          | `uv run optimize-py --mode vanilla`           | No          |
| Rust-accelerated, Wold's mode                      | `uv run --extra rust optimize`                | Yes         |
| Rust-accelerated, Vanilla mode                     | `uv run --extra rust optimize --mode vanilla` | Yes         |
| Show CLI help for the optimizer                    | `uv run optimize-py --help`                   | No          |

Anything after the command name is passed straight through to the optimizer, so no `--` separator is needed.

The optimizer writes its output spreadsheet (`Panel_WV_Decks_ndm_simulation.xlsx` by default) into the current working directory.

> If you run `uv run optimize` *without* `--extra rust`, the script will print a warning and fall back to pure Python — the same behavior as `uv run optimize-py`.

### 4. (Optional) Edit and re-run

The script's tunable constants live near the top of `NDM_Optimizer_Rust.py`. After editing, just re-run the same command — `uv` detects source changes and rebuilds the Rust extension when needed (only when you actually change `ndm_core/src/lib.rs` or its `Cargo.toml`/`Cargo.lock`).

---

## Modes

| Mode      | When to use it           | What it changes                                                                                       |
|-----------|--------------------------|-------------------------------------------------------------------------------------------------------|
| `wolds`   | Default. Wold's Vaults.  | Additive core stacking, positional shiny enabled, deluxe card system enabled.                         |
| `vanilla` | Vanilla VH cards.        | Multiplicative core scaling, positional shiny disabled (Shiny becomes a pure stat deck), no deluxe.   |

---

## Interactive web app (browser, no install)

The interactive single-deck inventory optimizer lives at
**https://poor-mans-physicist.github.io/woldsvaultsdeckoptimizer/**.
It runs entirely in the browser — the simulated annealer is compiled to WASM and
runs in a Web Worker, so nothing is sent to a server. Pick a deck, toggle cores
and overrides, fill in the inventory table, hit Run; the deck repaints with the
optimizer's chosen placement and per-slot NDM. Click any tile for the full math
breakdown. Switch to the **Preview** tab to assign concrete stat cards to each
scoring slot and see the resulting player-stat totals.

### Running the web app locally

You need `uv` (for the data bundle), `cargo` + `wasm-pack` (for the WASM core),
and `node` (for the Svelte/Vite app).

```bash
# 1. Generate the static data bundle (config/decks/modifiers JSON)
uv run python scripts/build_data.py

# 2. Build the WASM core into web/src/wasm/
(cd ndm_core && wasm-pack build --target web --release \
   --out-dir ../web/src/wasm -- --no-default-features --features wasm)

# 3. Run the dev server
cd web && npm install && npm run dev
```

### Deployment

Pushing to `main` triggers `.github/workflows/deploy.yml`, which runs the three
steps above and publishes the resulting `web/dist/` to GitHub Pages. No
server-side runtime is needed.
