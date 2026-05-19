# Wold's Vaults Deck Optimizer — Web App

Browser-only port of the inventory optimizer. The simulated-annealing core
runs entirely client-side as WASM inside a Web Worker — no server, no API
calls, no data leaves your machine. Deployed as a static site to GitHub Pages.

---

## Prerequisites

| Tool | Why | Install |
|------|-----|---------|
| [`uv`](https://docs.astral.sh/uv/) | runs the Python data emitter | `curl -LsSf https://astral.sh/uv/install.sh \| sh` |
| `cargo` (Rust stable) | compiles the WASM core | https://rustup.rs |
| `wasm-pack` | wraps the WASM into a JS module | `cargo install wasm-pack` |
| `node` (≥ 20) + `npm` | Vite dev server / production build | https://nodejs.org |

---

## Local dev

From the repo root:

```bash
# 1. Emit static data bundle → web/public/{config,decks,modifiers}.json
uv run python scripts/build_data.py

# 2. Compile the WASM core → web/src/wasm/
(cd ndm_core && wasm-pack build --target web --release \
   --out-dir ../web/src/wasm -- --no-default-features --features wasm)

# 3. Start Vite dev server
cd web && npm install && npm run dev
```

Open the URL Vite prints — by default
`http://localhost:5173/woldsvaultsdeckoptimizer/`.

### When to re-run each step

| You changed                              | Re-run                |
|------------------------------------------|-----------------------|
| `config.yaml` or `decks/*.{yaml,json}`   | step 1                |
| `ndm_core/src/**/*.rs`                   | step 2 (Vite HMRs)    |
| anything under `web/src/`                | nothing — Vite HMR    |

### Production preview

This serves the exact output GitHub Pages will host:

```bash
cd web && npm run build && npm run preview
```

### Type-check only

```bash
cd web && npm run check
```

---

## Deployment

`.github/workflows/deploy.yml` runs the three local-dev steps on every push to
`main` and uploads `web/dist/` to GitHub Pages. The deployed URL is
`https://<owner>.github.io/<repo>/`. The `base` URL in
[vite.config.ts](./vite.config.ts) is overridden by the workflow with
`VITE_BASE=/<repo>/` so asset paths line up.

---

## How it works

```diagram
                       BUILD TIME                                  RUNTIME (browser)
              ─────────────────────────────                ─────────────────────────────
                                                                       ┌──────────────┐
   config.yaml ─┐                                          ┌──────────▶│ App.svelte   │
   decks/*  ───┤  scripts/build_data.py ──▶ public/*.json ─┤           │   (Svelte 5) │
   modifiers.json                                          │           └──────┬───────┘
                                                           │                  │ postMessage
   ndm_core/src/*.rs ──▶ wasm-pack ──▶ src/wasm/*.{js,wasm}┘                  ▼
                                                                       ┌──────────────┐
                                                                       │ Web Worker   │
                                                                       │   ↓ wasm-bindgen
                                                                       │  ndm_core    │
                                                                       │  SA kernel   │
                                                                       └──────────────┘
```

### 1. Static data bundle (`scripts/build_data.py`)

The Python script reads the same authoritative inputs as the CLI optimizer:

- `config.yaml` → resolved per mode (defaults deep-merged with each
  `modes.<name>` block) and emitted as `web/public/config.json`.
- `decks/*.{yaml,json}` → flattened to `web/public/decks.json` with slot
  geometry, `base_core_slots` (pre-deckmod), arcane count, and constraints.
- `modifiers.json` → copied verbatim to `web/public/modifiers.json`.

The browser fetches these three JSON files at boot. The build script is
deliberately self-contained (no `import src.config`) so it doesn't trip the
single-mode loader's argv parsing.

### 2. Rust → WASM (`ndm_core`)

The crate dual-builds with feature flags:

| Build                                                                            | Where it's used                |
|----------------------------------------------------------------------------------|--------------------------------|
| `cargo build --release` (default `python` feature)                               | PyO3 extension for the CLI     |
| `wasm-pack build --target web -- --no-default-features --features wasm`          | Browser bundle                 |

The WASM build skips `rayon` (no threads in plain wasm32) and runs SA restarts
serially. On the Moon-deck fixture this is ~10× slower than the threaded
native build (~570 ms vs ~60 ms), which is acceptable for an interactive UI.

### 3. Svelte + TS glue

The TS modules under `src/lib/` mirror the Python optimizer one-for-one so the
Rust core's wasm entrypoint and the breakdown re-score agree to ≈1e-6:

| Module                                                | Purpose                                                                 |
|-------------------------------------------------------|-------------------------------------------------------------------------|
| `lib/config.ts`, `lib/deck.ts`                        | typed loaders for the build-time JSON bundle                            |
| `lib/types.ts`                                        | `CardType` / `Color` / `CardClass` / `CoreType` — must match Python+Rust|
| `lib/cores.ts`                                        | candidate-core enumeration (port of `candidate_cores_inventory`)         |
| `lib/breakdown.ts`                                    | per-slot NDM re-score (port of `simulate_inventory_breakdown`)          |
| `lib/modifiers.ts`                                    | stat-card classifier (port of `src/modifiers.py`)                       |
| `lib/preview.ts`                                      | slot-family resolution + Preview-mode aggregation                       |
| `lib/optimize.ts`                                     | wasm dispatch + per-combo SA loop + cross-check re-score                |
| `lib/workerClient.ts`, `worker/optimize.worker.ts`    | Promise wrapper around the optimize worker                              |
| `lib/state.svelte.ts`                                 | central runes-based app state store                                     |

### 4. Optimization flow

```diagram
  user clicks Run
       │
       ▼
  selectedCores() + inventory + deck + class
       │
       ▼
  optimizeInventoryAsync (workerClient.ts)
       │   postMessage(OptimizeInput)
       ▼
  worker: candidateCoresInventory(...) ─── one wasm call per core combo
       │                                          │
       │                                          ▼
       │                                   runSaInventory(payload) → (assignment, score)
       │
       ▼
  pick best (assignment, score)
       │
       ▼
  simulateInventoryBreakdown(...) (TS re-score)
       │
       ▼
  return { assignment, wasmScore, tsScore, coresUsed, breakdown }
```

The UI shows a green ✓ badge when `|wasmScore − tsScore| ≤ 1e-6`, which
verifies that the Rust SA kernel and the TS scorer agree.

### 5. Two tabs

- **Optimize** — pick deck/class/cores/inventory, hit Run, see per-slot NDM
  on the deck grid; click any slot for the full math breakdown.
- **Preview** — assign concrete stat cards (and tier) to each scoring slot;
  the right-side panel sums contributions as
  `Σ tier_value × per_slot_NDM` and splits flat vs percent attributes.

Assignments survive an Optimize re-run as long as the slot in the new layout
holds a card of the same family (Shiny / Evo / Deluxe / Typeless); otherwise
they're dropped.

---

## Layout

```
web/
├── README.md           ← you are here
├── package.json
├── vite.config.ts      ← wasm + top-level-await plugins, GH Pages base path
├── tsconfig.json
├── svelte.config.js
├── index.html
├── public/             ← generated by scripts/build_data.py (gitignored)
│   ├── config.json
│   ├── decks.json
│   └── modifiers.json
└── src/
    ├── main.ts
    ├── App.svelte                  ← tabs, layout, dialog dispatch
    ├── components/                 ← presentational Svelte components
    │   ├── DeckGrid.svelte
    │   ├── InventoryTable.svelte
    │   ├── CorePicker.svelte
    │   ├── ModeToggle.svelte
    │   ├── Legend.svelte
    │   ├── BreakdownDialog.svelte  ← Optimize-tab popup
    │   ├── AssignDialog.svelte     ← Preview-tab popup
    │   └── StatsPanel.svelte       ← Preview-tab aggregate
    ├── lib/                        ← pure logic / data layer
    ├── worker/
    │   └── optimize.worker.ts
    └── wasm/                       ← generated by wasm-pack (gitignored)
```

---

## Troubleshooting

- **`wasm-pack: command not found`** — `cargo install wasm-pack`.
- **`Failed to load config.json`** in dev — you forgot step 1 (the JSON
  bundle isn't checked in; it's generated from the source-of-truth Python
  config).
- **`Failed to fetch ndm_core_bg.wasm`** — you forgot step 2, or `web/src/wasm/`
  was wiped. Re-run the `wasm-pack build …` command.
- **WASM/TS verification badge is red** — the Rust SA produced a layout that
  the TS scorer disagrees with. This means a real divergence between
  `ndm_core/src/inventory.rs` and `web/src/lib/breakdown.ts` (or the
  candidate-cores enumerator). File an issue with the deck name + selected
  cores + inventory so it can be reproduced.
- **Stuck on "Optimizing…"** — open DevTools → Console to see the worker's
  error; common cause is an empty inventory.
