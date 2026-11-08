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

## Scoring change 2026-11-07

Greed stacking moved to the live 3.21.6 raw sum and the DIAG lone-card floor was removed. Both change
scores, so `bench_web_best.json` was reset; the tables above were measured under the old scoring.

## Deeper presets and lane-independent search (2026-11-08)

Old deep took ~1 s in the browser on typical decks, so the presets moved down one level and deep was retuned
at ~4x the old deep's work. Partial rounds (10 decks: the 8 with the largest run-to-run spread plus queen and large,
x evo/shiny x 2 reps, 12 lanes; gap to the best score of the round):

| candidate (vs old deep) | mean gap | p90 | max | wall s |
|---|---|---|---|---|
| old deep | 1.53% | 3.92% | 7.21% | 3.40 |
| sel: screen 6x60k, beam 16, halve 24 x 10x80k, full 96x30k | 1.11% | 3.84% | 10.35% | 12.14 |
| full: screen 4x60k, beam 12, halve 16 x 8x60k, full 192x40k top 16 | 0.76% | 2.71% | 6.91% | 9.24 |
| **long: screen 4x80k, beam 12, halve 18 x 8x80k, full 96x60k top 12, p2 300k** | **0.56%** | 2.63% | 3.48% | 10.54 |
| longfull: old deep screen, full 144x60k top 16 | 0.96% | 2.72% | 6.91% | 5.72 |
| longer: lighter screen, full 96x90k, p2 450k | 1.48% | 5.00% | 10.35% | 7.24 |

A rerun of "long" reproduced its first-round scores exactly. Extra work pays in long full-run walks backed by a
deeper screen; a lighter screen misses core sets.

Lane independence: the full stage used to split restarts into `max(topK, lanes / survivors)` jobs, so the result
depended on the worker count. Jobs are now `max(topK, ceil(restarts / 6))` per set. Same seeds at 12 and 24
lanes: 18/18 runs identical (score and core set). The site's worker cap went from 12 to 32. On the 24-core
Ultra 9 275HX (8 P + 16 E cores) 24 lanes run the 4x deep 1.5x faster than 12.

Full panel, new presets (37 decks x evo/shiny x 2 reps, every core, 24 lanes):

| preset | mean gap | median | p90 | max | hits | wall s/run | CPU s/run |
|---|---|---|---|---|---|---|---|
| fast (= old default) | 1.437% | 0 | 4.846% | 10.345% | 51% | 0.98 | 19.0 |
| default (= old deep) | 1.166% | 0 | 3.886% | 15.799% | 63% | 2.01 | 41.7 |
| deep (new) | 0.377% | 0 | 0.955% | 7.779% | 77% | 6.16 | 133.0 |

## Structural optimizer (2026-11-08)

Changes: target schedules everywhere (cold walks phase 1; deep scores 12x100k + top 3 x 200k phase 2 at the old
step total), chain steps warm-started from the parent layout's best cards (8k Standard / 12k Deep, phase-2
schedule), band and resolve re-scores warm from the class's best kept layout at half the old steps, fixed job
counts (results independent of the worker count), one shared queue and least-loaded worker dispatch.

Bench: `bench_struct.ts` (see its header) on 6 decks (queen, wold, bishop, merchant, fairy, large), evo, a seeded
random roll per k = 3 and 5, combo from the core search, 24 lanes, one run each. k = 3 has exhaustive truth:
every reachable set scored cold 6 x 100k + phase 2. "Ideal gap" is to the best ideal from any run or the truth;
"T short" is how far the player's layout score falls below the best known; rarity error is |P(better) - truth|.

| config | wall s | ideal gap | ideal hits | k=3 T short | k=3 err at own T | k=3 err vs truth T |
|---|---|---|---|---|---|---|
| old Standard | 15.5 | 0.291% | 10/12 | 0.477% | 0.119 | 0.088 |
| new Standard | 10.1 | 0.411% | 9/12 | 0.020% | 0.096 | 0.094 |
| old Deep | 57.9 | 0.004% | 11/12 | 3.063% | 0.069 | 0.106 |
| new Deep | 39.0 | 0.000% | 12/12 | 0.085% | 0.030 | 0.024 |

Both efforts are about a third faster. Deep is better everywhere. The old Deep scored one player layout (wold k=3)
16.6% low and reported P(better) 0.74 against a true 0.23. Standard's rarity error is dominated by the 10k x 1
screen in both versions (merchant k=3: 0.33 vs 0.68 true); its ideal misses moved between problems (old: wold k=5
2.9%, merchant k=5 0.6%; new: wold k=5 1.5%, bishop k=5 2.0%, bishop k=3 1.4%). One run per cell, 12 problems.

### Structural follow-ups (2026-11-08)

Single-seed rows (12 problems) for successive halving in place of the chains: Standard 6.9 s, 6/12 exact best,
0.60% mean gap even with arrangement transfer; Deep 24.8 s, 9/12. Per-layout re-scores do not find the card
arrangements the chains carry between layouts, so halving was dropped. The warm screen alone cut Standard's
rarity error from 0.094 to ~0.06.

Single seeds could not separate Standard variants (8 of 12 problems are solved by every version; bishop k=3 has
three discrete outcomes), so `bench_struct.ts` gained `--salt` (remaps every job seed in the shim) and `--only`.
5-seed convergence test, Standard, bishop/wold k=3/5 (20 runs per version):

| version | mean gap | worst | exact best | player score low | k=3 rarity err | wall s |
|---|---|---|---|---|---|---|
| pre-port (live) | 0.762% | 2.63% | 10/20 | 4.53% | 0.096 | 25.4 |
| mix 1x (warm screen + seeded warm chains) | 1.447% | 10.27% | 5/20 | 1.81% | 0.056 | 14.1 |
| mix 1.5x | 2.250% | 10.27% | 5/20 | 1.91% | 0.062 | 22.9 |
| mix + 8 hot chains | 0.889% | 2.13% | 7/20 | 1.81% | 0.056 | 17.9 |
| **mix + 12 hot chains (shipped Standard)** | **0.745%** | **2.13%** | 8/20 | **1.81%** | **0.056** | **17.8** |

Bishop k=3 exact best: live 5/5, mix 1x 1/5, 1.5x 0/5, hot 8 2/5, hot 12 3/5. Warm chains settle into one card
arrangement and longer anneals settle harder; hot chains (random start, a fresh legacy-schedule walk every step)
restore the wide search. Deep keeps the cold screen and warm random chains; its switch-gated build reproduced the
earlier Deep rows exactly (12/12).

Deep, same 5-seed test (20 runs per version): the cold-screen / warm-chain Deep 0.903% mean gap, 6.85% worst,
8/20 exact best (bishop k=3 2/5), rarity err 0.042, 58.9 s; with warm screen + seeded chains + 24 of 48 hot
chains (shipped) 0.320%, 1.07%, 12/20 (bishop k=3 5/5), 0.028, 63.4 s; the same at 350 steps / 20k cold walks /
150k + 300k finalists 0.504%, 1.56%, 10/20, 0.034, 56.8 s.
