# Web search bench (2026-11-07)

`bench_web.ts` drives the site's own TypeScript search (`coreSearch.ts`) and the WASM kernel on Node worker
threads (12 lanes, the site's pool cap). Problems: every non-Mystery Wold's deck x evo/shiny, Max mode. "old9"
owns the pre-port core list (Pure, Equilibrium, Steadfast, Sparkling, Shiny, Premium, Void, Archive, 4 colors);
"all" adds the 0.34.1 addon cores. Gap = % below the persistent best-known score for (deck, class, owned set)
(`bench_web_best.json`, only ever raised). Numbers from different runs are not comparable: every run can raise a
best-known.

Run:

```
wasm-pack build --target nodejs --release --out-dir ../web/src/wasm-node -- --no-default-features --features wasm
npx esbuild wasm-port/scripts/bench_web.ts --bundle --platform=node --format=esm --outfile=<dir>/bw.mjs
cp wasm-port/scripts/bench_worker.mjs <dir>/
node <dir>/bw.mjs <repo> --suite panel|schedule [--reps 2] [--configs a,b] [--dump rows.json]
```

## Scoring gate

`tagged_parity.ts`: WASM kernel vs the TypeScript mirror (`taggedBreakdown.ts`) on 3000 random layouts covering
all 20 scoring cores, random implicits and Mystery pairs: 0 mismatches, worst relative difference 4.0e-16. 40
anneals: reported score = fresh kernel score = mirror score. Kernel `cargo test --release` (`delta_full_equiv`,
new cores included) passes. CLI `parity_2_0.py --fast`: Part A 1472 layouts, 0 failures; Part B the 2 known
min-14-reg stragglers (Expanded, Extended evo), no new failures.

## Tuning log (all-cores problem)

1. First presets (beam over owned cores, 2x20k screens, 12x60k + phase 2): 0.72 s/run, mean gap 1.18%.
   61/148 runs picked a non-best core set (mean gap 2.5%); same-set runs 0.27%. Pluto sits in most best sets:
   its least-common-group bonus pays only once the layout has few stat cards and many greeds (the minority
   group is then a group every stat card carries), which a short screen rarely reaches.
2. Wider / longer screens (beam 8-10, 3x30k): 1.27-1.65% at 1.2-1.5 s vs 1.86% for (1) in that run.
3. Schedule suite, single calls on the best-known sets, equal steps (240k): 1x240k 5.70% mean gap, 4x60k
   3.55%, 8x30k 3.09%, 16x15k 3.33%, 4x30k + phase 2 (2x60k) 3.07% with the best median (0.76%) and hit share
   (43%). At 60k single runs: geometric 9.34%, target [0.5, 0.001, 0.02, 0.01] 7.76%, hotter / cooler / eta
   variants 7.17-8.07% (noise level). Kept the CLI's target schedules; moved the full stage to many short
   restarts.
4. Warm screens (an extended set's walks start from its parent's best layout) + short restarts: 1.40% at
   1.18 s vs deep 1.26% at 1.89 s (same run).
5. Beam diversity (the best set holding each coordination core stays in the beam and halving pool): with warm
   screens and short restarts 1.53% at 0.90 s vs deep 1.67% at 1.87 s; missed-Pluto runs 9 vs 31.
6. Scaled presets (same run): 0.81 s 1.55%, 1.28 s 1.26%, 1.72 s 1.26%, 2.63 s 0.84%. Adopted 0.81 s as fast,
   1.28 s as default, 2.63 s as deep.

## Confirmation (final presets)

One run, 37 decks x evo/shiny x 3 reps, 12 lanes:

| config | owned | mean gap | median | p90 | max | hits | wall s/run | CPU s/run |
|---|---|---|---|---|---|---|---|---|
| old site (structured enumeration, legacy schedule, 75k x 12 per candidate) | old9 | 0.532% | 0 | 1.599% | 4.374% | 67% | 1.74 | 19.5 |
| new default | old9 | 0.180% | 0 | 0.562% | 3.459% | 84% | 0.51 | 4.8 |
| new fast | all | 1.742% | 0 | 5.229% | 17.679% | 50% | 0.79 | 8.3 |
| new default | all | 1.399% | 0 | 4.179% | 10.345% | 53% | 1.26 | 13.4 |
| new deep | all | 0.871% | 0 | 3.316% | 10.345% | 64% | 2.61 | 28.9 |

On the old core list the new search is 3.4x faster at a third of the gap. With every core it stays under the old
site's run time, but the larger space (Pluto / Nitwit basins in particular) leaves a 1.4% mean gap at default
and 0.9% at deep.
