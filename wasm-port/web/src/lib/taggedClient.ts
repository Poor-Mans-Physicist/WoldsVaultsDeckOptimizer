// Main-thread orchestrator for Optimizer 2.0 runs: the core-set search (coreSearch.ts) on the worker pool, then
// the TS-mirror breakdown of the winning layout.

import {
  CardClass, OptimizerMode,
  type CoreSpec, type Placed, type TaggedPlaced,
} from "./types";
import { dispatchTagged, workerPoolSize } from "./workerPool";
import {
  activeImplicits, colorsRealFor, parseKernelAssignment, type TaggedRunInput,
} from "./tagged";
import { searchCores, type KernelOut, type RankedSet } from "./coreSearch";
import { SEARCH_DEPTHS } from "./searchConfig";
import {
  simulateTaggedBreakdown, type TaggedBreakdownResult,
} from "./taggedBreakdown";
import type { RawTaggedPlaced } from "./optimize";

export interface TaggedOptimizeResult {
  /** The deck the SA actually scored (post-structural-cores). `cards` is
   *  parallel to ITS slots — use this, not app.deck, for slot lookups. */
  deck:       import("./deck").Deck;
  /** Per-slot tagged cards, parallel to deck.slots. */
  cards:      TaggedPlaced[];
  /** Legacy map (type, color) keyed by `${r},${c}` — Preview tab + grid. */
  assignment: Map<string, Placed>;
  wasmScore:  number;
  tsScore:    number;
  coresUsed:  CoreSpec[];
  breakdown:  TaggedBreakdownResult;
  /** The best full-size core sets the search compared (full-run score when it got one). */
  ranked?:    RankedSet[];
}

/** Pre-flight validation. Throws a user-readable error when the run can't
 *  produce anything sensible. */
function preflight(input: TaggedRunInput): void {
  if (input.mode === OptimizerMode.EXACT) {
    const usable = input.exactStacks.filter(
      (s) => s.count > 0 && (input.complexCards || s.scaleColor === s.color),
    );
    if (usable.length === 0) {
      throw new Error(
        "Exact inventory is empty — add cards with the builder (or enable " +
        "Complex Cards if your cards have mismatched scale colors).",
      );
    }
    const totalMust = usable.filter((s) => s.mustPlace)
      .reduce((a, s) => a + s.count, 0);
    if (totalMust > input.deck.slots.length) {
      throw new Error(
        `Must-place cards (${totalMust}) exceed deck capacity ` +
        `(${input.deck.slots.length} slots).`,
      );
    }
  }
  if (input.mode === OptimizerMode.TARGETED) {
    const foil = input.targetedRules.find((r) => r.axis === "group" && r.key === "Foil");
    if (foil && foil.max === 0
        && input.cardClass === CardClass.SHINY && input.appMode !== "vanilla") {
      throw new Error(
        "Foil is banned but Wold's shiny cards are always foil — nothing is " +
        "placeable, the deck would be empty (§5). Lift the Foil cap or " +
        "switch to Evolution.",
      );
    }
  }
}

export async function optimizeTaggedAsync(
  input: TaggedRunInput,
): Promise<TaggedOptimizeResult> {
  const t0 = performance.now();
  preflight(input);

  const poolSize = workerPoolSize();
  let lane = 0;
  const runner = (payload: Record<string, unknown>) =>
    dispatchTagged<KernelOut>(lane++ % poolSize, payload);
  const best = await searchCores(input, input.cores, input.deck.core_slots, {
    runner, lanes: poolSize, depth: SEARCH_DEPTHS[input.depth], seed: input.seed,
    onProgress: input.onProgress,
  });
  const tWorkers = performance.now();

  const out = finalizeTaggedResult(input, best.combo, best.assignment, best.score);
  out.ranked = best.ranked;
  const breakdown = out.breakdown;

  console.log(
    `[optimize 2.0] total=${(performance.now() - t0).toFixed(0)}ms  ` +
    `mode=${input.mode}  depth=${input.depth}  seed=${input.seed}  pool=[${best.stats.pool.join(" ")}]  ` +
    `screened=${best.stats.screened}  halved=${best.stats.halved}  full=${best.stats.full}  ` +
    `jobs=${best.stats.jobs} (lanes ${poolSize})  workers=${(tWorkers - t0).toFixed(0)}ms  ` +
    `wasm=${best.score.toFixed(3)} ts=${breakdown.total.toFixed(3)}`,
  );
  if (Math.abs(best.score - breakdown.total) > 1e-6) {
    // Loud on purpose: a mismatch means the TS mirror drifted from the
    // kernel — the verify badge will show it, but log the detail too.
    console.error(
      `[optimize 2.0] TS mirror mismatch: wasm=${best.score} ts=${breakdown.total}`,
    );
  }

  return out;
}

/** Kernel output for `input.deck` + `combo` → the full result object (typed
 *  cards, grid map, TS-mirror breakdown for the verify badge). */
export function finalizeTaggedResult(
  input: TaggedRunInput,
  combo: CoreSpec[],
  raw: RawTaggedPlaced[],
  score: number,
): TaggedOptimizeResult {
  const cards = parseKernelAssignment(raw);
  const assignment = new Map<string, Placed>();
  for (let i = 0; i < input.deck.slots.length; i++) {
    const [r, c] = input.deck.slots[i];
    assignment.set(`${r},${c}`, [cards[i].t, cards[i].color]);
  }
  const breakdown = simulateTaggedBreakdown(
    input.deck, cards, input.cardClass, combo,
    activeImplicits(input),
    {
      colorsReal: colorsRealFor(input, combo),
      complex: input.complexCards,
      wvFoilRules: input.appMode !== "vanilla",
    },
    input.cfg,
  );
  return {
    deck: input.deck,
    cards, assignment,
    wasmScore: score,
    tsScore: breakdown.total,
    coresUsed: combo,
    breakdown,
  };
}
