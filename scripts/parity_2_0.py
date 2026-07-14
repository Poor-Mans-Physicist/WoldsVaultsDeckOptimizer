"""Optimizer 2.0 validation gate (spec §12).

Two checks against the classic 1.x kernel, run in the active --mode:

  A. SCORING equivalence — for sampled (deck, class, cores) combos, score K
     random assignments (drawn from the shared 2.0 vocabulary) with BOTH the
     Python reference ``simulate()`` (the classic spec) and the Rust
     ``score_tagged`` in Max configuration. Must match to 1e-9. This proves
     the scoring math is identical where the models overlap.

  B. OPTIMUM equivalence — run the classic SA and the tagged-Max SA
     (implicits stripped, §6 final pass off) on every deck × class × panel
     config and compare best-of-restarts scores within a relative tolerance.
     SA is stochastic, so B is a convergence check, not a proof; A is the
     proof.

Run:  uv run python scripts/parity_2_0.py [--mode vanilla] [--fast]
"""
from __future__ import annotations

import argparse
import random
import sys
from pathlib import Path

# Windows consoles default to cp1252 — force UTF-8 so the report glyphs print.
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

# --mode is consumed by src.config at import; keep argparse in sync.
_ap = argparse.ArgumentParser()
_ap.add_argument("--mode", default="wolds")
_ap.add_argument("--fast", action="store_true", help="fewer decks / iterations")
_ARGS, _ = _ap.parse_known_args()

from src.config import DECKS, MODE, _get_test_configs  # noqa: E402
from src.types import CardClass, CardType  # noqa: E402
from src.simulate import (  # noqa: E402
    candidate_cores, sa_optimize, sa_optimize_tagged, simulate,
    _peers_as_indices, _get_placeable,
)
from src import simulate as _sim  # noqa: E402
import ndm_core  # noqa: E402

# Shared 2.0 vocabulary (classic minus the 0-mult greeds, minus wild).
_SHARED = [
    CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
    CardType.TYPELESS, CardType.DELUXE,
    CardType.DIR_GREED_UP, CardType.DIR_GREED_DOWN,
    CardType.DIR_GREED_LEFT, CardType.DIR_GREED_RIGHT,
    CardType.DEAD,
]

TOL_SCORE = 1e-9      # part A: identical math
TOL_OPT   = 5e-3      # part B: relative SA-convergence tolerance


def _random_assignment(deck, card_class, rng):
    placeable = [t for t in _get_placeable(card_class) if t in _SHARED]
    if not placeable:
        placeable = [CardType.TYPELESS, CardType.DEAD]
    asgn = {}
    for p in deck.slots:
        if p in deck.arcane_slots:
            asgn[p] = rng.choice([CardType.ARCANE, CardType.DEAD])
        else:
            asgn[p] = rng.choice(placeable)
    return asgn


def _score_tagged(deck, asgn, card_class, cores):
    slots_list = list(deck.slots)
    slot_order = {p: i for i, p in enumerate(slots_list)}
    mono = "red"
    assignment = []
    for p in slots_list:
        t = asgn[p]
        color = "" if t == CardType.DEAD else mono
        groups = []
        # Run-level foil rule, mirrored from the kernel's materialize():
        # shiny (Wold's) or evo+FOIL-core ⇒ scorable/arcane cards are foil.
        foil_core = any(c.value == "foil" for c in cores)
        shiny = card_class == CardClass.SHINY
        wv = MODE != "vanilla"
        foil = (wv if shiny else foil_core)
        scorable_or_arcane = t in (
            CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
            CardType.DELUXE, CardType.TYPELESS, CardType.ARCANE,
        )
        if foil and scorable_or_arcane:
            groups.append("Foil")
        assignment.append((t.value, color, "", groups))

    return ndm_core.score_tagged(
        slots               = slots_list,
        row_peers           = _peers_as_indices(slot_order, deck._row_peers, slots_list),
        col_peers           = _peers_as_indices(slot_order, deck._col_peers, slots_list),
        surr_peers          = _peers_as_indices(slot_order, deck._surr_peers, slots_list),
        diag_peers          = _peers_as_indices(slot_order, deck._diag_peers, slots_list),
        arcane_slot_indices = [slot_order[p] for p in deck.arcane_slots],
        assignment          = assignment,
        implicits           = [],
        cores               = [(c.value, "", -1.0) for c in cores],
        mult_dir_vert          = _sim.MULT_DIR_GREED_VERT,
        mult_dir_horiz         = _sim.MULT_DIR_GREED_HORIZ,
        mult_pure_base         = _sim.MULT_PURE_BASE,
        mult_pure_scale        = _sim.MULT_PURE_SCALE,
        mult_equilibrium       = _sim.MULT_EQUILIBRIUM,
        mult_foil              = _sim.MULT_FOIL,
        mult_steadfast         = _sim.MULT_STEADFAST,
        mult_sparkling         = _sim.MULT_SPARKLING,
        mult_color             = _sim.MULT_COLOR,
        mult_deluxe_flat       = _sim.MULT_DELUXE_FLAT,
        mult_deluxe_core_base  = _sim.MULT_DELUXE_CORE_BASE,
        mult_deluxe_core_scale = _sim.MULT_DELUXE_CORE_SCALE,
        mult_void_core_base    = _sim.MULT_VOID_CORE_BASE,
        mult_void_core_scale   = _sim.MULT_VOID_CORE_SCALE,
        mult_archive_core      = _sim.MULT_ARCHIVE_CORE,
        greed_additive         = _sim.GREED_ADDITIVE,
        additive_cores         = _sim.ADDITIVE_CORES,
        is_shiny               = card_class == CardClass.SHINY,
        colors_real            = False,
        complex_cards          = False,
        wv_foil_rules          = MODE != "vanilla",
    )


def part_a(decks, rng, k_per_combo=25):
    print(f"── Part A: scoring equivalence ({len(decks)} decks × classes × cores × {k_per_combo} random assignments)")
    worst = 0.0
    fails = 0
    checked = 0
    for deck in decks:
        for card_class in CardClass:
            for cores in candidate_cores(card_class, deck):
                for _ in range(k_per_combo):
                    asgn = _random_assignment(deck, card_class, rng)
                    py = simulate(deck, asgn, card_class, cores)
                    rs = _score_tagged(deck, asgn, card_class, cores)
                    d = abs(py - rs)
                    rel = d / max(1.0, abs(py))
                    worst = max(worst, rel)
                    checked += 1
                    if rel > TOL_SCORE:
                        fails += 1
                        if fails <= 5:
                            cs = "+".join(sorted(c.value for c in cores))
                            print(f"  ✗ {deck.name} {card_class.value} [{cs}] py={py:.9f} rust={rs:.9f}")
    status = "PASS" if fails == 0 else "FAIL"
    print(f"  Part A: {status} — {checked} assignments, worst rel-Δ {worst:.2e}, {fails} failures")
    return fails == 0


def part_b(decks, n_iter, restarts):
    print(f"── Part B: SA optimum convergence ({len(decks)} decks, {n_iter} iters × {restarts} restarts)")
    fails = 0
    for deck in decks:
        for label, min_reg, max_greed in _get_test_configs(deck):
            cdeck = deck.with_constraints(min_reg, max_greed)
            for card_class in CardClass:
                best_c = -1.0
                best_t = -1.0
                for cores in candidate_cores(card_class, cdeck):
                    for _ in range(restarts):
                        _, sc = sa_optimize(cdeck, card_class, cores, n_iter=n_iter)
                        best_c = max(best_c, sc)
                        _, st = sa_optimize_tagged(
                            cdeck, card_class, cores, n_iter=n_iter,
                            implicits=[], final_pass=False,
                        )
                        best_t = max(best_t, st)
                rel = abs(best_c - best_t) / max(1.0, abs(best_c))
                ok = rel <= TOL_OPT
                flag = "✓" if ok else "✗"
                if not ok:
                    fails += 1
                print(f"  {flag} {deck.name:<28s} [{label:>13s}] {card_class.value:<5s} "
                      f"classic={best_c:10.3f} tagged={best_t:10.3f} relΔ={rel:.2e}")
    status = "PASS" if fails == 0 else "FAIL"
    print(f"  Part B: {status} — {fails} combos beyond tolerance {TOL_OPT}")
    return fails == 0


def main():
    rng = random.Random(0xDECC)
    decks = list(DECKS)
    if _ARGS.fast:
        decks = decks[:6]
        n_iter, restarts, k = 20_000, 4, 8
    else:
        n_iter, restarts, k = 40_000, 6, 25
    print(f"[parity 2.0] mode={MODE}  decks={len(decks)}")
    ok_a = part_a(decks, rng, k_per_combo=k)
    ok_b = part_b(decks, n_iter, restarts)
    if ok_a and ok_b:
        print("\nVALIDATION GATE: PASS")
        sys.exit(0)
    print("\nVALIDATION GATE: FAIL")
    sys.exit(1)


if __name__ == "__main__":
    main()
