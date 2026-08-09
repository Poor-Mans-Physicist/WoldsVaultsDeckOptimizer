// Main-thread orchestrator for Optimizer 2.0 runs. Fan-out unit is
// (candidate core combo × restart chunk) — "one core per restart" — so the
// whole worker pool stays busy even when a run has a single candidate.

import {
  CardClass, OptimizerMode,
  type CoreSpec, type Placed, type TaggedPlaced,
} from "./types";
import { candidateCoresInventory } from "./cores";
import { dispatchTagged, workerPoolSize } from "./workerPool";
import {
  activeImplicits, buildTaggedPayload, buildTasks, colorsRealFor,
  parseKernelAssignment, type TaggedRunInput,
} from "./tagged";
import { preferredMonoColor } from "./implicits";
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

  let candidates = candidateCoresInventory(
    input.cores, input.cardClass, input.deck.core_slots,
    input.deck.slots.length, input.deck.arcaneSlots.length, input.cfg,
  );
  if (candidates.length === 0) candidates = [[]];

  // Color-blind runs score every COLOR-core color identically; stable-sort
  // combos matching a color-keyed implicit first so score ties resolve
  // toward the color the player should actually build (velara → green).
  if (!colorsRealFor(input)) {
    const pref = preferredMonoColor(activeImplicits(input));
    if (pref) {
      const matches = (combo: CoreSpec[]) =>
        combo.some((s) => s.core_type === "color" && s.color === pref) ? 0 : 1;
      candidates = [...candidates].sort((a, b) => matches(a) - matches(b));
    }
  }

  const poolSize = workerPoolSize();
  const tasks = buildTasks(candidates, input.restarts, poolSize);

  const results = await Promise.all(tasks.map((task, i) =>
    dispatchTagged<{ assignment: RawTaggedPlaced[]; score: number }>(
      i, buildTaggedPayload(input, task.combo, task.restarts),
    ).then((r) => ({ ...r, combo: task.combo })),
  ));

  let best = results[0];
  for (let i = 1; i < results.length; i++) {
    if (results[i].score > best.score) best = results[i];
  }
  const tWorkers = performance.now();

  const cards = parseKernelAssignment(best.assignment);
  const assignment = new Map<string, Placed>();
  for (let i = 0; i < input.deck.slots.length; i++) {
    const [r, c] = input.deck.slots[i];
    assignment.set(`${r},${c}`, [cards[i].t, cards[i].color]);
  }

  const breakdown = simulateTaggedBreakdown(
    input.deck, cards, input.cardClass, best.combo,
    activeImplicits(input),
    {
      colorsReal: colorsRealFor(input, best.combo),
      complex: input.complexCards,
      wvFoilRules: input.appMode !== "vanilla",
    },
    input.cfg,
  );

  console.log(
    `[optimize 2.0] total=${(performance.now() - t0).toFixed(0)}ms  ` +
    `mode=${input.mode}  candidates=${candidates.length}  tasks=${tasks.length} ` +
    `(pool ${poolSize})  workers=${(tWorkers - t0).toFixed(0)}ms  ` +
    `wasm=${best.score.toFixed(3)} ts=${breakdown.total.toFixed(3)}`,
  );
  if (Math.abs(best.score - breakdown.total) > 1e-6) {
    // Loud on purpose: a mismatch means the TS mirror drifted from the
    // kernel — the verify badge will show it, but log the detail too.
    console.error(
      `[optimize 2.0] TS mirror mismatch: wasm=${best.score} ts=${breakdown.total}`,
    );
  }

  return {
    deck: input.deck,
    cards, assignment,
    wasmScore: best.score,
    tsScore: breakdown.total,
    coresUsed: best.combo,
    breakdown,
  };
}
