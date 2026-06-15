"""Simulation kernel: per-assignment scoring, candidate cores, SA, optimize().

This module is the heart of the optimizer. It owns:
  * ``simulate()``     — score one (assignment, cores) combination
  * ``candidate_cores()`` — enumerate viable core sets per (class, deck)
  * ``sa_optimize()``  — simulated annealing (Rust + pure-Python fallback)
  * ``optimize()``     — top-level orchestrator
"""
from __future__ import annotations

import math
import random
import time
from itertools import combinations
from typing import Dict, FrozenSet, List, Optional, Tuple

from .types import (
    CardClass, CardType, CoreType, Position,
    GREED_TYPES, REGULAR_TYPES, DELUXE_TYPES, TYPELESS_TYPES,
    PLACEABLE,
)
from .config import (
    ADDITIVE_CORES,
    ALLOW_DELUXE,
    ALLOW_VOID,
    Deck,
    DELUXE_COUNTED_AS_REGULAR,
    ENABLE_EXPERIMENTAL_EXPONENT,
    EXPERIMENTAL_BOOST,
    EXPERIMENTAL_EXPONENT,
    GREED_ADDITIVE,
    MULT_COLOR,
    MULT_DELUXE_CORE_BASE,
    MULT_DELUXE_CORE_SCALE,
    MULT_DELUXE_FLAT,
    MULT_DIR_GREED_DIAG_DOWN,
    MULT_DIR_GREED_DIAG_UP,
    MULT_DIR_GREED_HORIZ,
    MULT_DIR_GREED_VERT,
    MULT_EQUILIBRIUM,
    MULT_EVO_GREED,
    MULT_FOIL,
    MULT_PURE_BASE,
    MULT_PURE_SCALE,
    MULT_STEADFAST,
    MULT_SURR_GREED,
    MULT_VOID_CORE_BASE,
    MULT_VOID_CORE_SCALE,
    SHINY_POSITIONAL,
)


def _get_placeable(card_class: CardClass) -> List[CardType]:
    """Return the list of placeable card types for a given class and settings."""
    if card_class == CardClass.SHINY and not SHINY_POSITIONAL:
        result = [t for t in PLACEABLE if t not in REGULAR_TYPES]
        return [CardType.TYPELESS] + result
    return list(PLACEABLE)

try:
    import ndm_core as _ndm_core
    _RUST_AVAILABLE = True
except ImportError:
    _ndm_core = None
    _RUST_AVAILABLE = False
    print("[ndm] Rust extension not found — using pure Python SA "
          "(run with 'uv run --extra rust optimize' to enable the Rust core).")

def _apply_greed(boost: Dict[Position, float], pos: Position, amount: float) -> None:
    # New additive rule: boost is a raw sum of the greed multipliers pointing
    # at this slot. The use-site `max(boost, 1.0)` floor handles the no-greed
    # case (boost stays 0 → b = 1). Multiplicative is unchanged.
    if pos in boost:
        if GREED_ADDITIVE: boost[pos] += amount
        else:              boost[pos] *= amount


def simulate(
    deck:       Deck,
    assignment: Dict[Position, CardType],
    card_class: CardClass,
    cores:      FrozenSet[CoreType],
) -> float:
    greed:    Dict[Position, CardType] = {}
    regular:  Dict[Position, CardType] = {}
    deluxe:   Dict[Position, CardType] = {}
    typeless: Dict[Position, CardType] = {}
    arcane:   Dict[Position, CardType] = {}
    n_dead = 0

    for p, t in assignment.items():
        if   t in GREED_TYPES:    greed[p]    = t
        elif t in REGULAR_TYPES:  regular[p]  = t
        elif t in DELUXE_TYPES:   deluxe[p]   = t
        elif t in TYPELESS_TYPES: typeless[p] = t
        elif t == CardType.ARCANE: arcane[p]  = t
        elif t == CardType.DEAD:  n_dead     += 1
        # CardType.EMPTY etc. are ignored

    # ARCANE counts as "filled" for row/col peer counts (so neighbors see it),
    # but is NOT scorable (no direct NDM, no greed boost, no cores apply).
    filled   = (frozenset(greed) | frozenset(regular) | frozenset(deluxe)
                | frozenset(typeless) | frozenset(arcane))
    scorable = {**regular, **deluxe, **typeless}

    # n_ns for PURE. ARCANE placements always count (preserving the pre-arcane
    # `+ deck.n_arcane` fudge as real placements). On top of that:
    #   EVO-no-FOIL → regular + greed (+arcane)   ─ regulars contribute too
    #   EVO+FOIL    → greed (+arcane)
    #   SHINY       → greed (+arcane)
    # TYPELESS / DELUXE are intentionally excluded here (this is the classic
    # kernel's design — only the inventory optimizer counts them).
    foil_active = CoreType.FOIL in cores
    if card_class == CardClass.EVO and not foil_active:
        n_ns = len(regular) + len(arcane) + len(greed)
    else:
        n_ns = len(greed) + len(arcane)
    n_deluxe = len(deluxe)

    # All cores fold into a single core_mult. Two cores are *per-card gated*:
    #   DELUXE_CORE — applies to regular/typeless cards, NOT to deluxe cards.
    #   VOID_CORE   — applies to regular/typeless/deluxe, NOT to dead cards
    #                 (dead cards have 0 base, so this only matters for symmetry).
    # We compute a baseline (everything else), plus separate addends/factors for
    # deluxe / void, then build per-class core_mult variants below.
    baseline_contribs = []
    deluxe_core_value = None  # raw multiplier value for DELUXE_CORE if present
    void_core_value   = None  # raw multiplier value for VOID_CORE if present
    for core in cores:
        if   core == CoreType.PURE:
            # n_ns now includes placed arcane cards directly.
            baseline_contribs.append(MULT_PURE_BASE + MULT_PURE_SCALE * n_ns)
        elif core == CoreType.EQUILIBRIUM and card_class == CardClass.SHINY:
            baseline_contribs.append(MULT_EQUILIBRIUM)
        elif core == CoreType.STEADFAST   and card_class == CardClass.SHINY:
            baseline_contribs.append(MULT_STEADFAST)
        elif core == CoreType.COLOR:
            baseline_contribs.append(MULT_COLOR)
        elif core == CoreType.FOIL:
            baseline_contribs.append(MULT_FOIL)
        elif core == CoreType.DELUXE_CORE:
            deluxe_core_value = MULT_DELUXE_CORE_BASE + MULT_DELUXE_CORE_SCALE * n_deluxe
        elif core == CoreType.VOID_CORE:
            void_core_value   = MULT_VOID_CORE_BASE   + MULT_VOID_CORE_SCALE   * n_dead

    if ADDITIVE_CORES:
        baseline_sum  = sum(v - 1.0 for v in baseline_contribs)
        deluxe_addend = (deluxe_core_value - 1.0) if deluxe_core_value is not None else 0.0
        void_addend   = (void_core_value   - 1.0) if void_core_value   is not None else 0.0
        regular_core_mult     = 1.0 + baseline_sum + deluxe_addend + void_addend
        deluxe_card_core_mult = 1.0 + baseline_sum + void_addend
        typeless_core_mult    = 1.0 + baseline_sum + deluxe_addend + void_addend
    else:
        baseline_prod = math.prod(baseline_contribs) if baseline_contribs else 1.0
        deluxe_factor = deluxe_core_value if deluxe_core_value is not None else 1.0
        void_factor   = void_core_value   if void_core_value   is not None else 1.0
        regular_core_mult     = baseline_prod * deluxe_factor * void_factor
        deluxe_card_core_mult = baseline_prod * void_factor
        typeless_core_mult    = baseline_prod * deluxe_factor * void_factor

    row_count: Dict[int, int] = {}
    col_count: Dict[int, int] = {}
    for r, c in filled:
        row_count[r] = row_count.get(r, 0) + 1
        col_count[c] = col_count.get(c, 0) + 1

    # Additive: start at 0 and accumulate raw multipliers; the `max(b, 1.0)`
    # floor at the use site promotes the no-greed case to a neutral 1× boost.
    # Multiplicative: start at 1 and multiply (unchanged).
    init  = 0.0 if GREED_ADDITIVE else 1.0
    boost = {p: init for p in scorable}

    for g, gt in greed.items():
        gr, gc = g
        if   gt == CardType.DIR_GREED_UP:
            t = (gr - 1, gc)
            if t in scorable:  _apply_greed(boost, t, MULT_DIR_GREED_VERT)
        elif gt == CardType.DIR_GREED_DOWN:
            t = (gr + 1, gc)
            if t in scorable:  _apply_greed(boost, t, MULT_DIR_GREED_VERT)
        elif gt == CardType.DIR_GREED_LEFT:
            t = (gr, gc - 1)
            if t in scorable:  _apply_greed(boost, t, MULT_DIR_GREED_HORIZ)
        elif gt == CardType.DIR_GREED_RIGHT:
            t = (gr, gc + 1)
            if t in scorable:  _apply_greed(boost, t, MULT_DIR_GREED_HORIZ)
        elif gt == CardType.DIR_GREED_NE:
            t = (gr - 1, gc + 1)
            if t in scorable:  _apply_greed(boost, t, MULT_DIR_GREED_DIAG_UP)
        elif gt == CardType.DIR_GREED_NW:
            t = (gr - 1, gc - 1)
            if t in scorable:  _apply_greed(boost, t, MULT_DIR_GREED_DIAG_UP)
        elif gt == CardType.DIR_GREED_SE:
            t = (gr + 1, gc + 1)
            if t in scorable:  _apply_greed(boost, t, MULT_DIR_GREED_DIAG_DOWN)
        elif gt == CardType.DIR_GREED_SW:
            t = (gr + 1, gc - 1)
            if t in scorable:  _apply_greed(boost, t, MULT_DIR_GREED_DIAG_DOWN)
        elif gt == CardType.EVO_GREED:
            if card_class == CardClass.EVO:
                t = (gr + 1, gc)
                if t in regular:   # only buffs EVO regular cards; not typeless or deluxe
                    _apply_greed(boost, t, MULT_EVO_GREED)
        elif gt == CardType.SURR_GREED:
            for t in deck._surr_peers[g]:
                if t in scorable:  _apply_greed(boost, t, MULT_SURR_GREED)

    ndm = 0.0
    def _contrib(val: float) -> float:
        return (val * EXPERIMENTAL_BOOST) ** EXPERIMENTAL_EXPONENT if ENABLE_EXPERIMENTAL_EXPONENT else val

    for p, t in regular.items():
        r, c = p
        if   t == CardType.ROW:  pos = row_count.get(r, 0)
        elif t == CardType.COL:  pos = col_count.get(c, 0)
        elif t == CardType.DIAG: pos = sum(1 for q in deck._diag_peers[p] if q in filled) + 1
        else:                    pos = sum(1 for q in deck._surr_peers[p] if q in filled)
        b    = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        ndm += _contrib(pos * regular_core_mult * b)

    for p in deluxe:
        b    = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        ndm += _contrib(MULT_DELUXE_FLAT * deluxe_card_core_mult * b)

    for p in typeless:
        b    = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        ndm += _contrib(1.0 * typeless_core_mult * b)

    return ndm



def candidate_cores(card_class: CardClass, deck: Deck) -> List[FrozenSet[CoreType]]:
    k = deck.core_slots

    # ── Shared helper ─────────────────────────────────────────────────────────
    def add_candidate(candidates, seen, combo):
        fs = frozenset(combo)
        if fs not in seen:
            seen.add(fs); candidates.append(fs)

    # ── SHINY ─────────────────────────────────────────────────────────────────
    # FOIL is now just another fixed multiplier for shiny — no mutual exclusion
    # with PURE. PURE remains the only variable core (n_ns = greed count unknown).
    if card_class == CardClass.SHINY:
        non_var_shiny = [CoreType.EQUILIBRIUM, CoreType.STEADFAST,
                         CoreType.COLOR,       CoreType.FOIL]

        def shiny_static(combo) -> float:
            m = 1.0
            for core in combo:
                if   core == CoreType.EQUILIBRIUM: m *= MULT_EQUILIBRIUM
                elif core == CoreType.STEADFAST:   m *= MULT_STEADFAST
                elif core == CoreType.COLOR:       m *= MULT_COLOR
                elif core == CoreType.FOIL:        m *= MULT_FOIL
            return m

        def best_non_var_shiny(slots: int) -> FrozenSet[CoreType]:
            cap    = min(slots, len(non_var_shiny))
            best_m = 0.0
            best_c: FrozenSet[CoreType] = frozenset()
            for size in range(0, cap + 1):
                for combo in (combinations(non_var_shiny, size) if size > 0 else [()]):
                    m = shiny_static(combo)
                    if m > best_m:
                        best_m = m; best_c = frozenset(combo)
            return best_c

        # VOID_CORE is always variable (n_dead is unknown pre-SA), so it joins
        # the variable pool like PURE and (when allowed) DELUXE_CORE.
        var_pool = [CoreType.PURE]
        if ALLOW_VOID:
            var_pool.append(CoreType.VOID_CORE)
        if ALLOW_DELUXE:
            var_pool.append(CoreType.DELUXE_CORE)

        candidates: List[FrozenSet[CoreType]] = []
        seen: set = set()
        for size in range(0, len(var_pool) + 1):
            for var_combo in (combinations(var_pool, size) if size > 0 else [()]):
                var = frozenset(var_combo) if size > 0 else frozenset()
                if len(var) > k: continue
                filler = best_non_var_shiny(k - len(var))
                add_candidate(candidates, seen, var | filler)
        return candidates

    # ── EVO ───────────────────────────────────────────────────────────────────
    # Two groups based on whether FOIL is present:
    #
    # Group A (no FOIL): n_ns = all filled cards ≈ deck size → PURE analytically known.
    #   Variable cores: DELUXE_CORE only.
    #   Fixed filler: best from {PURE, COLOR}.
    #
    # Group B (with FOIL): n_ns = greed only → PURE is now unknown pre-SA.
    #   Variable cores: PURE + DELUXE_CORE.
    #   Fixed filler: best from {COLOR} only (PURE is variable, FOIL already included).

    # EVO no-FOIL n_ns estimate. deck.slots already includes arcane slots under
    # the new model (the old `+ deck.n_arcane` fudge added them as a phantom).
    n_ns_full = len(deck.slots)

    def evo_no_foil_mult(combo) -> float:
        m = 1.0
        for core in combo:
            if   core == CoreType.PURE:  m *= MULT_PURE_BASE + MULT_PURE_SCALE * n_ns_full
            elif core == CoreType.COLOR: m *= MULT_COLOR
        return m

    def best_fixed_evo_no_foil(slots: int) -> FrozenSet[CoreType]:
        pool   = [CoreType.PURE, CoreType.COLOR]
        cap    = min(slots, len(pool))
        best_m = -1.0
        best_c: FrozenSet[CoreType] = frozenset({CoreType.PURE})
        for size in range(1, cap + 1):
            for combo in combinations(pool, size):
                m = evo_no_foil_mult(frozenset(combo))
                if m > best_m:
                    best_m = m; best_c = frozenset(combo)
        return best_c

    def best_fixed_evo_with_foil(slots: int) -> FrozenSet[CoreType]:
        # PURE is variable here; only COLOR is analytically evaluable filler
        if slots >= 1 and MULT_COLOR > 1.0:
            return frozenset({CoreType.COLOR})
        return frozenset()

    deluxe_var = [CoreType.DELUXE_CORE] if ALLOW_DELUXE else []
    # VOID_CORE joins the variable pool in both EVO groups (n_dead is unknown)
    # — but only when the mode allows it.
    void_var = [CoreType.VOID_CORE] if ALLOW_VOID else []
    candidates = []
    seen       = set()

    # Group A: no FOIL
    var_pool_a = list(deluxe_var) + list(void_var)
    for size in range(0, len(var_pool_a) + 1):
        for var_combo in (combinations(var_pool_a, size) if size > 0 else [()]):
            var = frozenset(var_combo) if size > 0 else frozenset()
            if len(var) > k: continue
            filler = best_fixed_evo_no_foil(k - len(var))
            combo = var | filler
            add_candidate(candidates, seen, combo)

    # Group B: with FOIL — PURE is variable
    var_pool_b = [CoreType.PURE] + deluxe_var + list(void_var)
    for size in range(0, len(var_pool_b) + 1):
        for var_combo in (combinations(var_pool_b, size) if size > 0 else [()]):
            var   = frozenset(var_combo) if size > 0 else frozenset()
            total = var | {CoreType.FOIL}
            if len(total) > k: continue
            filler = best_fixed_evo_with_foil(k - len(total))
            combo = total | filler
            add_candidate(candidates, seen, combo)

    return candidates


# ──────────────────────────────────────────────────────────────────────────────
# Simulated annealing
# ──────────────────────────────────────────────────────────────────────────────

def _precompute_best_positional(deck: Deck) -> Dict[Position, CardType]:
    """
    For each slot, determine which positional card type yields the highest
    multiplier based purely on deck geometry (peer set sizes).
    This is fixed for a given deck shape and never needs recomputing.
    DIAG counts self (+1); ROW/COL count all filled including self via row_count;
    SURR does not count self. For a fair comparison we use maximum possible peer
    counts (i.e. assume all slots filled), since relative ordering is geometry-only.
    """
    result: Dict[Position, CardType] = {}
    for p in deck.slots:
        r, c   = p
        counts = {
            CardType.ROW:  len(deck._row_peers[p]) + 1,   # +1 for self
            CardType.COL:  len(deck._col_peers[p]) + 1,   # +1 for self
            CardType.SURR: len(deck._surr_peers[p]),       # does not count self
            CardType.DIAG: len(deck._diag_peers[p]) + 1,  # +1 for self
        }
        result[p] = max(counts, key=counts.__getitem__)
    return result

# ── Peer-set converter: frozenset of (row,col) → list of slot indices ─────────
def _peers_as_indices(
    slot_order: Dict[Position, int],
    peer_sets: Dict[Position, FrozenSet[Position]],
    slots_list: List[Position],
) -> List[List[int]]:
    return [
        [slot_order[q] for q in peer_sets[p]]
        for p in slots_list
    ]


# ── Pure-Python SA (renamed from old sa_optimize) ─────────────────────────────
def _sa_optimize_python(
    deck:       Deck,
    card_class: CardClass,
    cores:      FrozenSet[CoreType],
    n_iter:     int,
    T_start:    float = 100.0,
    T_end:      float = 0.5,
) -> Tuple[Dict[Position, CardType], float]:
    # Live read so set_mode() flips on the GUI side propagate. Classic CLI
    # never flips at runtime so this is essentially a constant there.
    from . import config as _cfg
    auto_place_arcane = _cfg.AUTO_PLACE_ARCANE

    slots       = list(deck.slots)
    arcane_set  = deck.arcane_slots
    regular_slots = [p for p in slots if p not in arcane_set]
    placeable   = _get_placeable(card_class)
    default_t   = (CardType.TYPELESS
                   if card_class == CardClass.SHINY and not SHINY_POSITIONAL
                   else CardType.SURR)

    best_positional = _precompute_best_positional(deck)

    # Seed REGULAR slots only; arcane slots are pre-filled with ARCANE below.
    if deck.min_regular > 0 and deck.min_regular < len(regular_slots):
        shuffled = list(regular_slots)
        random.shuffle(shuffled)
        asgn: Dict[Position, CardType] = {
            p: (default_t if i < deck.min_regular else CardType.SURR_GREED)
            for i, p in enumerate(shuffled)
        }
    else:
        asgn = {p: default_t for p in regular_slots}

    # Pre-fill every arcane slot with ARCANE — strictly dominant in classic
    # except when SA later flips to DEAD for void-core (auto_place_arcane=false).
    for p in arcane_set:
        asgn[p] = CardType.ARCANE

    score      = simulate(deck, asgn, card_class, cores)
    best_score = score
    best_asgn  = dict(asgn)
    log_cool   = math.log(T_end / T_start)

    SCORING_TYPES = REGULAR_TYPES | TYPELESS_TYPES
    if DELUXE_COUNTED_AS_REGULAR:
        SCORING_TYPES = SCORING_TYPES | DELUXE_TYPES

    n_greed_cur = sum(1 for t in asgn.values() if t in GREED_TYPES)
    n_reg_cur   = sum(1 for t in asgn.values() if t in SCORING_TYPES)

    min_reg_active = (
        deck.min_regular >= 0
        and not (deck.max_greed >= 0
                 and deck.min_regular + deck.max_greed > len(deck.slots))
    )

    def _valid() -> bool:
        if deck.max_greed >= 0 and n_greed_cur > deck.max_greed:
            return False
        if min_reg_active and n_reg_cur < deck.min_regular:
            return False
        return True

    def _counter_update(old: CardType, new: CardType) -> None:
        nonlocal n_greed_cur, n_reg_cur
        if old in GREED_TYPES:   n_greed_cur -= 1
        if old in SCORING_TYPES: n_reg_cur   -= 1
        if new in GREED_TYPES:   n_greed_cur += 1
        if new in SCORING_TYPES: n_reg_cur   += 1

    def _resolve(p: Position, t: CardType) -> CardType:
        return best_positional[p] if t in REGULAR_TYPES else t

    for i in range(n_iter):
        T = T_start * math.exp(log_cool * i / n_iter)

        if len(slots) < 2 or random.random() < 0.80:
            p   = random.choice(slots)
            # Arcane-slot rule: under auto-place ON, arcane slots are locked.
            # Under OFF, the only legal swap is ARCANE↔DEAD.
            if p in arcane_set:
                if auto_place_arcane:
                    continue
                old = asgn[p]
                new = CardType.DEAD if old == CardType.ARCANE else CardType.ARCANE
                # ARCANE/DEAD are neither greed nor scoring; constraint deltas
                # are zero so we skip the counter dance.
                asgn[p] = new
                new_score = simulate(deck, asgn, card_class, cores)
                delta = new_score - score
                if delta >= 0 or random.random() < math.exp(delta / T):
                    score = new_score
                    if score > best_score:
                        best_score = score; best_asgn = dict(asgn)
                else:
                    asgn[p] = old
                continue

            old = asgn[p]
            new = _resolve(p, random.choice(placeable))
            if new == old:
                continue
            # Regular slot may never receive ARCANE.
            if new == CardType.ARCANE:
                continue
            _counter_update(old, new)
            asgn[p] = new
            if not _valid():
                _counter_update(new, old)
                asgn[p] = old
                continue
            new_score = simulate(deck, asgn, card_class, cores)
            delta     = new_score - score
            if delta >= 0 or random.random() < math.exp(delta / T):
                score = new_score
                if score > best_score:
                    best_score = score; best_asgn = dict(asgn)
            else:
                _counter_update(new, old)
                asgn[p] = old
        else:
            p1, p2 = random.sample(slots, 2)
            if asgn[p1] == asgn[p2]:
                continue
            # Reject swaps that would violate the arcane-slot rule. Also reject
            # any swap involving an arcane slot under auto_place_arcane.
            a1 = p1 in arcane_set
            a2 = p2 in arcane_set
            if auto_place_arcane and (a1 or a2):
                continue
            # `legal_at(p, v)`: ARCANE only allowed in arcane slots; arcane
            # slots accept only ARCANE or DEAD.
            def _legal(is_arc: bool, v: CardType) -> bool:
                if is_arc: return v in (CardType.ARCANE, CardType.DEAD)
                return v != CardType.ARCANE
            if not _legal(a1, asgn[p2]) or not _legal(a2, asgn[p1]):
                continue

            old1, old2 = asgn[p1], asgn[p2]
            _counter_update(old1, old2)
            _counter_update(old2, old1)
            asgn[p1], asgn[p2] = old2, old1
            if not _valid():
                _counter_update(old2, old1)
                _counter_update(old1, old2)
                asgn[p1], asgn[p2] = old1, old2
                continue
            new_score = simulate(deck, asgn, card_class, cores)
            delta     = new_score - score
            if delta >= 0 or random.random() < math.exp(delta / T):
                score = new_score
                if score > best_score:
                    best_score = score; best_asgn = dict(asgn)
            else:
                _counter_update(old2, old1)
                _counter_update(old1, old2)
                asgn[p1], asgn[p2] = old1, old2

    return best_asgn, best_score


# ── Public sa_optimize: tries Rust, falls back to Python ──────────────────────
def sa_optimize(
    deck:       Deck,
    card_class: CardClass,
    cores:      FrozenSet[CoreType],
    n_iter:     int,
    T_start:    float = 100.0,
    T_end:      float = 0.5,
) -> Tuple[Dict[Position, CardType], float]:

    if not _RUST_AVAILABLE:
        return _sa_optimize_python(deck, card_class, cores, n_iter, T_start, T_end)

    # ── Convert inputs for Rust ───────────────────────────────────────────────
    slots_list = list(deck.slots)                        # consistent ordering
    slot_order = {p: i for i, p in enumerate(slots_list)}

    row_peers_idx  = _peers_as_indices(slot_order, deck._row_peers,  slots_list)
    col_peers_idx  = _peers_as_indices(slot_order, deck._col_peers,  slots_list)
    surr_peers_idx = _peers_as_indices(slot_order, deck._surr_peers, slots_list)
    diag_peers_idx = _peers_as_indices(slot_order, deck._diag_peers, slots_list)

    cores_str    = [c.value for c in cores]
    placeable_str = [t.value for t in _get_placeable(card_class)]
    arcane_slot_indices = [slot_order[p] for p in deck.arcane_slots]

    # Read AUTO_PLACE_ARCANE live so a runtime set_mode() flip is honored. The
    # classic optimizer has no user toggle — config.yaml is the single source.
    from . import config as _cfg
    auto_place_arcane = _cfg.AUTO_PLACE_ARCANE

    # ── Call Rust ─────────────────────────────────────────────────────────────
    asgn_strs, best_score = _ndm_core.run_sa_optimize(
        slots               = slots_list,
        row_peers           = row_peers_idx,
        col_peers           = col_peers_idx,
        surr_peers          = surr_peers_idx,
        diag_peers          = diag_peers_idx,
        arcane_slot_indices = arcane_slot_indices,
        auto_place_arcane   = auto_place_arcane,
        min_regular         = deck.min_regular,
        max_greed           = deck.max_greed,
        is_shiny            = (card_class == CardClass.SHINY),
        cores               = cores_str,
        placeable           = placeable_str,
        n_iter     = n_iter,
        t_start    = T_start,
        t_end      = T_end,
        # Multiplier constants
        mult_dir_vert          = MULT_DIR_GREED_VERT,
        mult_dir_horiz         = MULT_DIR_GREED_HORIZ,
        mult_evo_greed         = MULT_EVO_GREED,
        mult_surr_greed        = MULT_SURR_GREED,
        mult_dir_diag_up       = MULT_DIR_GREED_DIAG_UP,
        mult_dir_diag_down     = MULT_DIR_GREED_DIAG_DOWN,
        mult_pure_base         = MULT_PURE_BASE,
        mult_pure_scale        = MULT_PURE_SCALE,
        mult_equilibrium       = MULT_EQUILIBRIUM,
        mult_foil              = MULT_FOIL,
        mult_steadfast         = MULT_STEADFAST,
        mult_color             = MULT_COLOR,
        mult_deluxe_flat       = MULT_DELUXE_FLAT,
        mult_deluxe_core_base  = MULT_DELUXE_CORE_BASE,
        mult_deluxe_core_scale = MULT_DELUXE_CORE_SCALE,
        mult_void_core_base    = MULT_VOID_CORE_BASE,
        mult_void_core_scale   = MULT_VOID_CORE_SCALE,
        # Flags
        greed_additive            = GREED_ADDITIVE,
        additive_cores            = ADDITIVE_CORES,
        shiny_positional          = SHINY_POSITIONAL,
        enable_experimental       = ENABLE_EXPERIMENTAL_EXPONENT,
        experimental_exponent     = EXPERIMENTAL_EXPONENT,
        experimental_boost        = EXPERIMENTAL_BOOST,
        deluxe_counted_as_regular = DELUXE_COUNTED_AS_REGULAR,
    )

    # ── Convert result back to Python format ──────────────────────────────────
    best_asgn = {
        slots_list[i]: CardType(asgn_strs[i])
        for i in range(len(slots_list))
    }
    return best_asgn, best_score
