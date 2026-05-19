"""Entry point: top-level ``optimize()`` orchestrator + parallel runner."""
from __future__ import annotations

import multiprocessing
import os
import random
import time
from typing import Dict, List, Tuple

from .types import CardClass
from .config import (
    DECKS,
    EXPORT_SPREADSHEET,
    MODE,
    SPREADSHEET_PREFIX,
    Deck,
    _CFG,
    _get_test_configs,
)
from .simulate import candidate_cores, sa_optimize
from .report import _report, generate_spreadsheet


# ──────────────────────────────────────────────────────────────────────────────
# Top-level optimizer
# ──────────────────────────────────────────────────────────────────────────────

def optimize(
    deck:     Deck,
    n_iter:   int,
    restarts: int,
    verbose:  bool = True,
) -> Dict[CardClass, dict]:
    """Run SA across every candidate core set for both card classes."""
    results: Dict[CardClass, dict] = {}

    for card_class in CardClass:
        candidates = candidate_cores(card_class, deck)
        best: dict = {"score": -1.0}
        run        = 0
        total_runs = len(candidates) * restarts

        if verbose:
            print(f"\n{'═'*66}")
            print(f"  Class : {card_class.value.upper()}   Deck : {deck.name}"
                  f"   Constraint : {deck.constraint_str()}")
            print(f"  Runs  : {len(candidates)} candidate(s) × {restarts} restarts = {total_runs}")
            print(f"{'═'*66}")

        for cores in candidates:
            cores_str = "+".join(ct.value[:4].upper() for ct in sorted(cores, key=lambda x: x.value))
            if verbose:
                print(f"\n  Candidate cores: {cores_str}")
            for _ in range(restarts):
                run += 1
                asgn, score = sa_optimize(deck, card_class, cores, n_iter=n_iter)
                improved = score > best["score"]
                if improved:
                    best = {"score": score, "cores": cores, "assignment": dict(asgn)}
                if verbose:
                    flag = "★" if improved else " "
                    print(f"  {flag}[{run:3d}/{total_runs}]  score={score:10.3f}  best={best['score']:10.3f}")

        results[card_class] = best
        if verbose:
            _report(card_class, best, deck)

    return results


# ──────────────────────────────────────────────────────────────────────────────
# Parallel worker + entry point
# ──────────────────────────────────────────────────────────────────────────────

def _run_deck_worker(args):
    deck, n_iter, restarts = args
    random.seed()
    t_start = time.perf_counter()
    result  = {}
    for label, min_reg, max_greed in _get_test_configs(deck):
        cdeck         = deck.with_constraints(min_reg, max_greed)
        result[label] = optimize(cdeck, n_iter=n_iter, restarts=restarts, verbose=False)
    elapsed = time.perf_counter() - t_start
    print(f"  ✓ Done: {deck.name}  ({elapsed:.1f}s)", flush=True)
    return deck.name, result


def main() -> None:
    n_iter   = int(_CFG["testing"]["n_iter"])
    restarts = int(_CFG["testing"]["restarts"])
    n_cores  = min(len(DECKS), multiprocessing.cpu_count())

    print(f"[ndm] Mode: {MODE}")
    print(f"Running {len(DECKS)} deck(s) across {n_cores} process(es)...")

    args = [(deck, n_iter, restarts) for deck in DECKS]
    with multiprocessing.Pool(processes=n_cores) as pool:
        raw = pool.map(_run_deck_worker, args)

    all_results = {name: result for name, result in raw}

    print("\n" + "█" * 66)
    print("  FINAL SUMMARY — ALL DECKS")
    print("█" * 66)
    for deck in DECKS:
        print(f"\n  {deck.name}")
        for label, _, __ in _get_test_configs(deck):
            test_res = all_results[deck.name].get(label, {})
            parts    = []
            for card_class in CardClass:
                ndm = test_res[card_class]["score"] if card_class in test_res else float("nan")
                parts.append(f"{card_class.value.upper()}={ndm:.2f}")
            print(f"    [{label:>13s}]  {'   '.join(parts)}")

    if EXPORT_SPREADSHEET:
        filename = f"{SPREADSHEET_PREFIX}ndm_simulation.xlsx"
        if os.path.exists(filename):
            print(f"\n  Spreadsheet '{filename}' already exists — skipping export.")
        else:
            generate_spreadsheet(all_results, DECKS, filename)
