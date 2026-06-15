"""Inventory-based optimizer (color-aware, single-deck, single-run).

This module is a fork of ``src.simulate`` designed to power the interactive
GUI: instead of running every deck with unlimited card supply, it takes a
concrete ``CardInventory`` (per ``(CardType, Color)`` stack) and produces the
single best assignment for one deck.

Differences from the classic optimizer:

* Cards are identified by ``(CardType, Color)``; only stacks present in the
  inventory dict can be placed (no color flipping).
* Positional bonuses count only same-color cards in scan range.
* The COLOR core is per-color and only boosts matching-color cards.
* Core multipliers may be overridden per inventory run.
* Empty slots after inventory exhaustion are filled with transparent
  ``DEAD`` cards (no NDM, no greed receipt, no participation in counts).
"""
from __future__ import annotations

import math
import random
from dataclasses import dataclass, field
from itertools import combinations
from typing import (
    Any,
    Dict,
    FrozenSet,
    List,
    Optional,
    Sequence,
    Set,
    Tuple,
)

from .types import (
    CardClass,
    CardType,
    Color,
    CoreType,
    Position,
)
from . import config as _cfg  # for live mode-dependent reads (e.g. ADDITIVE_CORES)
from .config import (
    Deck,
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
)


# ──────────────────────────────────────────────────────────────────────────────
# Data model
# ──────────────────────────────────────────────────────────────────────────────

# Sentinel placed-card value used for transparent fillers.
DEAD_CARD: Tuple[CardType, Optional[Color]] = (CardType.DEAD, None)

# A placed card is (CardType, color). Color is None only for DEAD.
Placed = Tuple[CardType, Optional[Color]]

# Card categories — mirrors GREED_TYPES / REGULAR_TYPES / etc. in types.py but
# split for the per-card scoring branch.
POSITIONAL_TYPES: FrozenSet[CardType] = frozenset({
    CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
})
GREED_TYPES_NEW: FrozenSet[CardType] = frozenset({
    CardType.DIR_GREED_UP,    CardType.DIR_GREED_DOWN,
    CardType.DIR_GREED_LEFT,  CardType.DIR_GREED_RIGHT,
    CardType.DIR_GREED_NE,    CardType.DIR_GREED_NW,
    CardType.DIR_GREED_SE,    CardType.DIR_GREED_SW,
    CardType.EVO_GREED,       CardType.SURR_GREED,
})

# Card types that may legally occupy an arcane (`A`) slot. Everything else is
# rejected by the SA swap kernel and by initial_fill placement logic.
ARCANE_SLOT_LEGAL: FrozenSet[CardType] = frozenset({
    CardType.ARCANE, CardType.DEAD,
})


@dataclass(frozen=True)
class CoreSpec:
    """One core in the user's inventory.

    ``color`` is set only for ``CoreType.COLOR`` entries. ``override`` lets the
    user supply a non-default multiplier for this specific run:

    * Static cores (EQUILIBRIUM, STEADFAST, COLOR, FOIL): replaces the multiplier.
    * PURE / DELUXE_CORE: replaces the *scale* term only; base + runtime
      ``n_ns`` / ``n_deluxe`` are unchanged.
    """
    core_type: CoreType
    color:     Optional[Color] = None
    override:  Optional[float] = None

    def __post_init__(self) -> None:
        if (self.core_type == CoreType.COLOR) != (self.color is not None):
            raise ValueError(
                "CoreSpec.color must be set iff core_type == CoreType.COLOR"
            )


@dataclass
class CoreInventory:
    """Set of cores the user owns.

    At most one of each core_type, except COLOR which can have one per color
    (so a user can own COLOR_RED + COLOR_BLUE simultaneously). The candidate
    enumerator still only ever *places* one color core per deck.
    """
    cores: Set[CoreSpec] = field(default_factory=set)


@dataclass
class CardInventory:
    """All the cards + cores available for one optimization run.

    Two-bucket inventory model:
      * ``counts`` — regular cards: optimizer may use 0 to N of each (type, color).
      * ``forced_counts`` — must-place cards: optimizer must place AT LEAST N.

    The total cap per (type, color) is the sum of both buckets:
      ``placement_count[(t, c)] in [forced_counts[(t, c)], counts[(t, c)] + forced_counts[(t, c)]]``
    """
    counts:        Dict[Tuple[CardType, Color], int]
    card_class:    CardClass
    cores:         CoreInventory = field(default_factory=CoreInventory)
    # Forced cards — empty dict by default. Existing call-sites that don't pass
    # this stay backwards-compatible (zero forced = current behavior).
    forced_counts: Dict[Tuple[CardType, Color], int] = field(default_factory=dict)

    def cap(self, t: CardType, c: Color) -> int:
        """Combined upper bound for one (type, color)."""
        return self.counts.get((t, c), 0) + self.forced_counts.get((t, c), 0)

    def min_required(self, t: CardType, c: Color) -> int:
        """Lower bound (forced minimum) for one (type, color)."""
        return self.forced_counts.get((t, c), 0)

    def total_cards(self) -> int:
        return sum(self.counts.values()) + sum(self.forced_counts.values())

    def total_forced(self) -> int:
        return sum(self.forced_counts.values())


@dataclass
class GreedSource:
    """One greed card contributing to a slot's boost."""
    from_position: Position
    greed_type:    CardType
    multiplier:    float  # raw multiplier from the greed card (before additive collapse)


@dataclass
class CoreComponent:
    """One core's contribution to the slot-level core_mult."""
    core_type:  CoreType
    color:      Optional[Color]  # only set for COLOR cores
    value:      float            # the numeric multiplier this core contributed
    override:   bool             # True if the user supplied a non-default value


@dataclass
class ExcludedCore:
    """One core that is in the deck but doesn't apply to this particular card."""
    core_type: CoreType
    color:     Optional[Color]
    reason:    str   # e.g. "card is blue, color core is red" / "deluxe core never boosts deluxes"


@dataclass
class SlotBreakdown:
    """Full multiplicative decomposition of one slot's NDM contribution.

    Under additive_cores mode every core that applies to this card is folded
    into ONE additive sum (``core_mult``). Cores that don't apply (wrong color
    for the color core, deluxe core on a deluxe card, EQUI/STEAD when not
    SHINY) are recorded in ``excluded_cores`` with a reason.
    """
    card_type:         CardType
    color:             Optional[Color]
    base_value:        float
    base_explain:      str
    applied_cores:     List[CoreComponent]
    excluded_cores:    List[ExcludedCore]
    core_mult:         float
    core_mult_formula: str           # e.g. "1 + (1.7-1) + (2.5-1) + (1.75-1)"
    boost:             float
    boost_sources:     List[GreedSource]
    final_ndm:         float


@dataclass
class InventoryResult:
    """Returned by ``optimize_inventory``."""
    assignment:          Dict[Position, Placed]
    score:               float                  # canonical: rust if available, else python
    cores_used:          FrozenSet[CoreSpec]
    per_slot_ndm:        Dict[Position, float] = field(default_factory=dict)
    # New fields for the GUI's verification + hover tooltips.
    python_score:        float                  = 0.0
    rust_score:          Optional[float]        = None
    per_slot_breakdown:  Dict[Position, SlotBreakdown] = field(default_factory=dict)


# ──────────────────────────────────────────────────────────────────────────────
# Core-multiplier lookup (override-aware)
# ──────────────────────────────────────────────────────────────────────────────

def _static_mult(spec: CoreSpec) -> float:
    """Multiplier for a non-variable core, honoring user override."""
    if spec.override is not None:
        return spec.override
    if spec.core_type == CoreType.EQUILIBRIUM: return MULT_EQUILIBRIUM
    if spec.core_type == CoreType.STEADFAST:   return MULT_STEADFAST
    if spec.core_type == CoreType.COLOR:       return MULT_COLOR
    if spec.core_type == CoreType.FOIL:        return MULT_FOIL
    raise ValueError(f"_static_mult called with non-static core {spec.core_type}")


def _pure_mult(spec: CoreSpec, n_ns: int) -> float:
    """PURE multiplier given runtime ``n_ns``. ``override`` replaces the scale."""
    scale = spec.override if spec.override is not None else MULT_PURE_SCALE
    return MULT_PURE_BASE + scale * n_ns


def _deluxe_core_mult(spec: CoreSpec, n_deluxe: int) -> float:
    """DELUXE_CORE multiplier given runtime ``n_deluxe``. ``override`` is scale."""
    scale = spec.override if spec.override is not None else MULT_DELUXE_CORE_SCALE
    return MULT_DELUXE_CORE_BASE + scale * n_deluxe


def _void_core_mult(spec: CoreSpec, n_dead: int) -> float:
    """VOID_CORE multiplier given runtime ``n_dead``. ``override`` is scale."""
    scale = spec.override if spec.override is not None else MULT_VOID_CORE_SCALE
    return MULT_VOID_CORE_BASE + scale * n_dead


def _classify_cores(
    cores:      FrozenSet[CoreSpec],
    card_class: CardClass,
    n_ns:       int,
    n_deluxe:   int,
    n_dead:     int,
) -> Tuple[
    List["CoreComponent"],
    Optional["CoreComponent"],
    Optional["CoreComponent"],
    Optional["CoreComponent"],
    List["ExcludedCore"],
]:
    """Sort cores into baseline / color / deluxe / void buckets.

    Returns ``(baseline, color_comp, deluxe_comp, void_comp, class_excluded)``:
      * ``baseline`` — cores that apply to every non-greed scoring card
        regardless of color (PURE, FOIL, plus EQUI/STEAD when class is SHINY).
      * ``color_comp`` — the active COLOR core, if any. Gates per-card by color.
      * ``deluxe_comp`` — the active DELUXE_CORE, if any. Excludes deluxe cards.
      * ``void_comp`` — the active VOID_CORE, if any. Excludes dead cards
        (dead cards score 0 anyway; this is only used for breakdown symmetry).
      * ``class_excluded`` — cores excluded by the card-class rule (EQUI/STEAD
        on EVO). Precomputed once for the run.
    """
    baseline:   List[CoreComponent]      = []
    color_comp: Optional[CoreComponent]  = None
    deluxe_comp: Optional[CoreComponent] = None
    void_comp:  Optional[CoreComponent]  = None
    class_excluded: List[ExcludedCore]   = []

    for spec in cores:
        is_override = spec.override is not None
        if spec.core_type == CoreType.PURE:
            # n_ns already includes placed ARCANE cards (computed by the caller
            # from the runtime assignment) — no separate fudge addend needed.
            v = _pure_mult(spec, n_ns)
            baseline.append(CoreComponent(CoreType.PURE, None, v, is_override))
        elif spec.core_type == CoreType.EQUILIBRIUM:
            if card_class == CardClass.SHINY:
                baseline.append(CoreComponent(CoreType.EQUILIBRIUM, None, _static_mult(spec), is_override))
            else:
                class_excluded.append(ExcludedCore(
                    CoreType.EQUILIBRIUM, None,
                    "equilibrium only applies to SHINY decks (this run is EVO)",
                ))
        elif spec.core_type == CoreType.STEADFAST:
            if card_class == CardClass.SHINY:
                baseline.append(CoreComponent(CoreType.STEADFAST, None, _static_mult(spec), is_override))
            else:
                class_excluded.append(ExcludedCore(
                    CoreType.STEADFAST, None,
                    "steadfast only applies to SHINY decks (this run is EVO)",
                ))
        elif spec.core_type == CoreType.FOIL:
            baseline.append(CoreComponent(CoreType.FOIL, None, _static_mult(spec), is_override))
        elif spec.core_type == CoreType.COLOR:
            color_comp = CoreComponent(CoreType.COLOR, spec.color, _static_mult(spec), is_override)
        elif spec.core_type == CoreType.DELUXE_CORE:
            v = _deluxe_core_mult(spec, n_deluxe)
            deluxe_comp = CoreComponent(CoreType.DELUXE_CORE, None, v, is_override)
        elif spec.core_type == CoreType.VOID_CORE:
            v = _void_core_mult(spec, n_dead)
            void_comp = CoreComponent(CoreType.VOID_CORE, None, v, is_override)

    return baseline, color_comp, deluxe_comp, void_comp, class_excluded


# ──────────────────────────────────────────────────────────────────────────────
# Scoring kernel (color-aware)
# ──────────────────────────────────────────────────────────────────────────────

def _apply_greed(boost: Dict[Position, float], pos: Position, amount: float) -> None:
    """Same semantics as simulate.py's _apply_greed — additive or multiplicative.

    Additive rule: boost is a raw sum of greed multipliers pointing at this
    slot. The use-site `max(b, 1.0)` floor promotes the no-greed case back
    to a neutral 1× boost. Multiplicative is unchanged.
    """
    if pos in boost:
        if GREED_ADDITIVE: boost[pos] += amount
        else:              boost[pos] *= amount


def simulate_inventory(
    deck:       Deck,
    assignment: Dict[Position, Placed],
    card_class: CardClass,
    cores:      FrozenSet[CoreSpec],
) -> float:
    """Score one (assignment, cores) combo with color-aware multipliers.

    Mirrors ``src.simulate.simulate`` but:
      * cards carry color, positional counts split per color
      * COLOR core gates per-card by color match
      * DEAD cards are fully transparent
    """
    # Partition by category.
    positional: Dict[Position, Placed] = {}
    deluxe:     Dict[Position, Placed] = {}
    typeless:   Dict[Position, Placed] = {}
    greed:      Dict[Position, Placed] = {}
    arcane:     Dict[Position, Placed] = {}
    n_dead = 0

    for p, (t, _c) in assignment.items():
        if   t == CardType.DEAD:        n_dead += 1
        elif t == CardType.ARCANE:      arcane[p]     = (t, _c)
        elif t in POSITIONAL_TYPES:     positional[p] = (t, _c)
        elif t == CardType.DELUXE:      deluxe[p]     = (t, _c)
        elif t == CardType.TYPELESS:    typeless[p]   = (t, _c)
        elif t in GREED_TYPES_NEW:      greed[p]      = (t, _c)
        # FILLER_GREED, EMPTY etc. fall through and are ignored.

    # Same-color counts. `colored` is every non-dead placed card with its color
    # — ARCANE cards participate here so they boost neighbors' same-color peer
    # counts (their own 0-NDM contribution doesn't change either way).
    colored: Dict[Position, Color] = {}
    for p, (t, c) in assignment.items():
        if t == CardType.DEAD or c is None:
            continue
        colored[p] = c

    # Build per-row / per-col same-color counts.
    row_count: Dict[Tuple[int, Color], int] = {}
    col_count: Dict[Tuple[int, Color], int] = {}
    for (r, ccol), color in colored.items():
        row_count[(r, color)]    = row_count.get((r, color), 0)    + 1
        col_count[(ccol, color)] = col_count.get((ccol, color), 0) + 1

    foil_active = any(s.core_type == CoreType.FOIL for s in cores)

    # n_ns for PURE. ARCANE placements always count (preserving the pre-arcane
    # `+ deck.n_arcane` fudge as real placements). On top of that:
    #   EVO-no-FOIL → positional + deluxe + typeless + greed (+arcane)
    #   EVO+FOIL    → greed (+arcane)
    #   SHINY       → greed (+arcane)
    if card_class == CardClass.EVO and not foil_active:
        n_ns = len(positional) + len(deluxe) + len(typeless) + len(arcane) + len(greed)
    else:
        n_ns = len(greed) + len(arcane)
    n_deluxe = len(deluxe)

    # All cores fold into ONE per-card ``core_mult``. Precompute the baseline
    # (cores that apply to every non-greed card) and the color-/deluxe-/void-
    # gated addends so each card's multiplier is a cheap constant-time combo.
    baseline, color_comp, deluxe_comp, void_comp, _ex = _classify_cores(
        cores, card_class, n_ns, n_deluxe, n_dead,
    )
    baseline_sum  = sum(c.value - 1.0 for c in baseline)
    baseline_prod = math.prod(c.value for c in baseline) if baseline else 1.0
    color_core_color = color_comp.color if color_comp is not None else None
    color_addend  = (color_comp.value - 1.0) if color_comp is not None else 0.0
    color_factor  = color_comp.value         if color_comp is not None else 1.0
    deluxe_addend = (deluxe_comp.value - 1.0) if deluxe_comp is not None else 0.0
    deluxe_factor = deluxe_comp.value         if deluxe_comp is not None else 1.0
    void_addend   = (void_comp.value - 1.0)   if void_comp   is not None else 0.0
    void_factor   = void_comp.value           if void_comp   is not None else 1.0

    def _card_core_mult(card_type: CardType, card_color: Optional[Color]) -> float:
        """Combined per-card core multiplier — color, deluxe, and void cores
        fold in here, gated by:
          color  → card_color matches the color core's color
          deluxe → card is NOT a deluxe card
          void   → card is NOT a dead card (never reached in practice)
        """
        color_applies  = (
            color_comp is not None
            and card_color is not None
            and card_color == color_core_color
        )
        deluxe_applies = (deluxe_comp is not None and card_type != CardType.DELUXE)
        void_applies   = (void_comp is not None and card_type != CardType.DEAD)
        if _cfg.ADDITIVE_CORES:
            return (1.0
                    + baseline_sum
                    + (color_addend  if color_applies  else 0.0)
                    + (deluxe_addend if deluxe_applies else 0.0)
                    + (void_addend   if void_applies   else 0.0))
        m = baseline_prod
        if color_applies:  m *= color_factor
        if deluxe_applies: m *= deluxe_factor
        if void_applies:   m *= void_factor
        return m

    # Greed-boost map (per target slot).
    # Additive: start at 0 and accumulate raw multipliers; floored at 1 at
    # use. Multiplicative: start at 1 and multiply.
    scorable_positions = set(positional) | set(deluxe) | set(typeless)
    init = 0.0 if GREED_ADDITIVE else 1.0
    boost: Dict[Position, float] = {p: init for p in scorable_positions}

    for g, (gt, _gc) in greed.items():
        gr, gcc = g
        if gt == CardType.DIR_GREED_UP:
            t = (gr - 1, gcc)
            if t in scorable_positions: _apply_greed(boost, t, MULT_DIR_GREED_VERT)
        elif gt == CardType.DIR_GREED_DOWN:
            t = (gr + 1, gcc)
            if t in scorable_positions: _apply_greed(boost, t, MULT_DIR_GREED_VERT)
        elif gt == CardType.DIR_GREED_LEFT:
            t = (gr, gcc - 1)
            if t in scorable_positions: _apply_greed(boost, t, MULT_DIR_GREED_HORIZ)
        elif gt == CardType.DIR_GREED_RIGHT:
            t = (gr, gcc + 1)
            if t in scorable_positions: _apply_greed(boost, t, MULT_DIR_GREED_HORIZ)
        elif gt == CardType.DIR_GREED_NE:
            t = (gr - 1, gcc + 1)
            if t in scorable_positions: _apply_greed(boost, t, MULT_DIR_GREED_DIAG_UP)
        elif gt == CardType.DIR_GREED_NW:
            t = (gr - 1, gcc - 1)
            if t in scorable_positions: _apply_greed(boost, t, MULT_DIR_GREED_DIAG_UP)
        elif gt == CardType.DIR_GREED_SE:
            t = (gr + 1, gcc + 1)
            if t in scorable_positions: _apply_greed(boost, t, MULT_DIR_GREED_DIAG_DOWN)
        elif gt == CardType.DIR_GREED_SW:
            t = (gr + 1, gcc - 1)
            if t in scorable_positions: _apply_greed(boost, t, MULT_DIR_GREED_DIAG_DOWN)
        elif gt == CardType.EVO_GREED:
            if card_class == CardClass.EVO:
                t = (gr + 1, gcc)
                if t in positional:
                    _apply_greed(boost, t, MULT_EVO_GREED)
        elif gt == CardType.SURR_GREED:
            for tgt in deck._surr_peers[g]:
                if tgt in scorable_positions:
                    _apply_greed(boost, tgt, MULT_SURR_GREED)

    # NDM accumulation. Single combined core_mult per card via _card_core_mult.
    ndm = 0.0

    for p, (t, c) in positional.items():
        r, ccol = p
        if   t == CardType.ROW:  pos_val = row_count.get((r, c), 0) if c is not None else 0
        elif t == CardType.COL:  pos_val = col_count.get((ccol, c), 0) if c is not None else 0
        elif t == CardType.DIAG:
            pos_val = 1 + sum(1 for q in deck._diag_peers[p] if colored.get(q) == c)
        else:  # SURR
            pos_val = sum(1 for q in deck._surr_peers[p] if colored.get(q) == c)
        b    = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        ndm += pos_val * _card_core_mult(t, c) * b

    for p, (_t, c) in deluxe.items():
        b    = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        ndm += MULT_DELUXE_FLAT * _card_core_mult(CardType.DELUXE, c) * b

    for p, (_t, c) in typeless.items():
        b    = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        ndm += 1.0 * _card_core_mult(CardType.TYPELESS, c) * b

    return ndm


# ──────────────────────────────────────────────────────────────────────────────
# Per-slot NDM breakdown (for the GUI heatmap)
# ──────────────────────────────────────────────────────────────────────────────

def simulate_inventory_breakdown(
    deck:       Deck,
    assignment: Dict[Position, Placed],
    card_class: CardClass,
    cores:      FrozenSet[CoreSpec],
) -> Tuple[float, Dict[Position, "SlotBreakdown"]]:
    """Re-score one assignment and emit a ``SlotBreakdown`` for every slot.

    Mirrors ``simulate_inventory`` but factors out each multiplier with a
    human-readable source string so the GUI tooltip can show exactly what
    contributed to each slot's NDM. Intended for one-shot use on a final
    assignment — do not call from the SA hot loop.
    """
    positional: Dict[Position, Placed] = {}
    deluxe:     Dict[Position, Placed] = {}
    typeless:   Dict[Position, Placed] = {}
    greed:      Dict[Position, Placed] = {}
    arcane:     Dict[Position, Placed] = {}
    n_dead = 0

    for p, (t, c) in assignment.items():
        if   t == CardType.DEAD:        n_dead += 1
        elif t == CardType.ARCANE:      arcane[p]     = (t, c)
        elif t in POSITIONAL_TYPES:     positional[p] = (t, c)
        elif t == CardType.DELUXE:      deluxe[p]     = (t, c)
        elif t == CardType.TYPELESS:    typeless[p]   = (t, c)
        elif t in GREED_TYPES_NEW:      greed[p]      = (t, c)

    colored: Dict[Position, Color] = {}
    for p, (t, c) in assignment.items():
        # ARCANE participates in same-color counts to boost neighbors.
        if t == CardType.DEAD or c is None:
            continue
        colored[p] = c

    row_count: Dict[Tuple[int, Color], int] = {}
    col_count: Dict[Tuple[int, Color], int] = {}
    for (r, ccol), color in colored.items():
        row_count[(r, color)]    = row_count.get((r, color), 0)    + 1
        col_count[(ccol, color)] = col_count.get((ccol, color), 0) + 1

    # ARCANE always counts in n_ns (preserves pre-arcane Pure-fudge behavior).
    foil_active = any(s.core_type == CoreType.FOIL for s in cores)
    if card_class == CardClass.EVO and not foil_active:
        n_ns = len(positional) + len(deluxe) + len(typeless) + len(arcane) + len(greed)
    else:
        n_ns = len(greed) + len(arcane)
    n_deluxe = len(deluxe)

    # Classify cores once. The breakdown for each slot picks from these buckets
    # depending on the slot's color and whether it's a deluxe / dead card.
    baseline, color_comp, deluxe_comp, void_comp, class_excluded = _classify_cores(
        cores, card_class, n_ns, n_deluxe, n_dead,
    )

    scorable_positions = set(positional) | set(deluxe) | set(typeless)
    # Mirror simulate_inventory: additive starts at 0 (floored to 1 at use),
    # multiplicative starts at 1.
    init_boost = 0.0 if GREED_ADDITIVE else 1.0
    boost: Dict[Position, float] = {p: init_boost for p in scorable_positions}
    # Parallel record of which greeds contributed to each scorable slot.
    boost_sources: Dict[Position, List[GreedSource]] = {p: [] for p in scorable_positions}

    def _record_and_apply(src_pos: Position, src_type: CardType, target: Position, amount: float) -> None:
        if target not in scorable_positions:
            return
        _apply_greed(boost, target, amount)
        boost_sources[target].append(GreedSource(src_pos, src_type, amount))

    for g, (gt, _gc) in greed.items():
        gr, gcc = g
        if gt == CardType.DIR_GREED_UP:
            _record_and_apply(g, gt, (gr - 1, gcc), MULT_DIR_GREED_VERT)
        elif gt == CardType.DIR_GREED_DOWN:
            _record_and_apply(g, gt, (gr + 1, gcc), MULT_DIR_GREED_VERT)
        elif gt == CardType.DIR_GREED_LEFT:
            _record_and_apply(g, gt, (gr, gcc - 1), MULT_DIR_GREED_HORIZ)
        elif gt == CardType.DIR_GREED_RIGHT:
            _record_and_apply(g, gt, (gr, gcc + 1), MULT_DIR_GREED_HORIZ)
        elif gt == CardType.DIR_GREED_NE:
            _record_and_apply(g, gt, (gr - 1, gcc + 1), MULT_DIR_GREED_DIAG_UP)
        elif gt == CardType.DIR_GREED_NW:
            _record_and_apply(g, gt, (gr - 1, gcc - 1), MULT_DIR_GREED_DIAG_UP)
        elif gt == CardType.DIR_GREED_SE:
            _record_and_apply(g, gt, (gr + 1, gcc + 1), MULT_DIR_GREED_DIAG_DOWN)
        elif gt == CardType.DIR_GREED_SW:
            _record_and_apply(g, gt, (gr + 1, gcc - 1), MULT_DIR_GREED_DIAG_DOWN)
        elif gt == CardType.EVO_GREED:
            if card_class == CardClass.EVO:
                t = (gr + 1, gcc)
                if t in positional:
                    _record_and_apply(g, gt, t, MULT_EVO_GREED)
        elif gt == CardType.SURR_GREED:
            for tgt in deck._surr_peers[g]:
                _record_and_apply(g, gt, tgt, MULT_SURR_GREED)

    def _card_breakdown(
        card_type: CardType, card_color: Optional[Color],
    ) -> Tuple[List[CoreComponent], List[ExcludedCore], float, str]:
        """Build the per-card (applied_cores, excluded_cores, core_mult, formula)."""
        applied:  List[CoreComponent] = list(baseline)
        excluded: List[ExcludedCore]  = list(class_excluded)

        # Color core gating
        if color_comp is not None:
            if card_color is None:
                excluded.append(ExcludedCore(
                    CoreType.COLOR, color_comp.color,
                    f"card has no color (color core is {color_comp.color.value if color_comp.color else '?'})",
                ))
            elif card_color == color_comp.color:
                applied.append(color_comp)
            else:
                excluded.append(ExcludedCore(
                    CoreType.COLOR, color_comp.color,
                    f"card color is {card_color.value} (color core is {color_comp.color.value if color_comp.color else '?'})",
                ))

        # Deluxe core gating
        if deluxe_comp is not None:
            if card_type == CardType.DELUXE:
                excluded.append(ExcludedCore(
                    CoreType.DELUXE_CORE, None,
                    "deluxe core never boosts deluxe cards (they fuel it instead)",
                ))
            else:
                applied.append(deluxe_comp)

        # Void core gating — applies to every non-dead card. Dead cards have
        # base value 0 (we never call this for them), but the exclusion is
        # tracked here for symmetry with deluxe.
        if void_comp is not None:
            if card_type == CardType.DEAD:
                excluded.append(ExcludedCore(
                    CoreType.VOID_CORE, None,
                    "void core never boosts dead cards (they fuel it instead)",
                ))
            else:
                applied.append(void_comp)

        vals = [c.value for c in applied]
        if _cfg.ADDITIVE_CORES:
            mult = 1.0 + sum(v - 1.0 for v in vals)
            formula = (
                "1 + " + " + ".join(f"({v:.3f}-1)" for v in vals)
                if vals else "1.0 (no cores apply)"
            )
        else:
            mult = math.prod(vals) if vals else 1.0
            formula = (
                " × ".join(f"{v:.3f}" for v in vals) if vals else "1.0 (no cores apply)"
            )
        return applied, excluded, mult, formula

    per_slot_breakdown: Dict[Position, SlotBreakdown] = {}
    total = 0.0

    def _zero_breakdown(p: Position, t: CardType, c: Optional[Color], base_explain: str) -> SlotBreakdown:
        """Empty breakdown for greed/dead/empty slots — final_ndm is always 0."""
        return SlotBreakdown(
            card_type=t, color=c,
            base_value=0.0, base_explain=base_explain,
            applied_cores=[], excluded_cores=[],
            core_mult=1.0, core_mult_formula="(not scored)",
            boost=1.0, boost_sources=[],
            final_ndm=0.0,
        )

    # Greed/dead/arcane/empty slots — non-scoring.
    for p in deck.slots:
        if p in scorable_positions:
            continue
        if p in greed:
            gt, gc = greed[p]
            per_slot_breakdown[p] = _zero_breakdown(p, gt, gc, "greed card — provides boost to neighbors, no own NDM")
        elif p in arcane:
            at, ac = arcane[p]
            color_str = ac.value if ac is not None else "—"
            per_slot_breakdown[p] = _zero_breakdown(
                p, at, ac,
                f"arcane card (color {color_str}) — fixed 0 NDM, no cores applied; "
                f"counts in n_ns for Pure and in same-color peer counts for neighbors"
            )
        elif p in assignment:
            t, c = assignment[p]
            per_slot_breakdown[p] = _zero_breakdown(p, t, c, "dead card — transparent, contributes nothing")
        else:
            per_slot_breakdown[p] = _zero_breakdown(p, CardType.EMPTY, None, "empty slot")

    # Scorable slots.
    for p, (t, c) in positional.items():
        r, ccol = p
        if t == CardType.ROW:
            pos_val = row_count.get((r, c), 0) if c is not None else 0
            base_explain = f"row {r}, color {c.value if c else '—'} → row_count = {pos_val}"
        elif t == CardType.COL:
            pos_val = col_count.get((ccol, c), 0) if c is not None else 0
            base_explain = f"col {ccol}, color {c.value if c else '—'} → col_count = {pos_val}"
        elif t == CardType.DIAG:
            pos_val = 1 + sum(1 for q in deck._diag_peers[p] if colored.get(q) == c)
            base_explain = f"diag (self + same-color peers, color {c.value if c else '—'}) = {pos_val}"
        else:  # SURR
            pos_val = sum(1 for q in deck._surr_peers[p] if colored.get(q) == c)
            base_explain = f"surrounding same-color peers (color {c.value if c else '—'}) = {pos_val}"
        applied, excluded, cm, formula = _card_breakdown(t, c)
        b   = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        v   = pos_val * cm * b
        per_slot_breakdown[p] = SlotBreakdown(
            card_type=t, color=c,
            base_value=float(pos_val), base_explain=base_explain,
            applied_cores=applied, excluded_cores=excluded,
            core_mult=cm, core_mult_formula=formula,
            boost=b, boost_sources=list(boost_sources[p]),
            final_ndm=v,
        )
        total += v

    for p, (t, c) in deluxe.items():
        applied, excluded, cm, formula = _card_breakdown(CardType.DELUXE, c)
        b  = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        v  = MULT_DELUXE_FLAT * cm * b
        per_slot_breakdown[p] = SlotBreakdown(
            card_type=t, color=c,
            base_value=float(MULT_DELUXE_FLAT), base_explain=f"deluxe flat value = {MULT_DELUXE_FLAT}",
            applied_cores=applied, excluded_cores=excluded,
            core_mult=cm, core_mult_formula=formula,
            boost=b, boost_sources=list(boost_sources[p]),
            final_ndm=v,
        )
        total += v

    for p, (t, c) in typeless.items():
        applied, excluded, cm, formula = _card_breakdown(CardType.TYPELESS, c)
        b  = max(boost[p], 1.0) if GREED_ADDITIVE else boost[p]
        v  = 1.0 * cm * b
        per_slot_breakdown[p] = SlotBreakdown(
            card_type=t, color=c,
            base_value=1.0, base_explain="typeless flat value = 1.0",
            applied_cores=applied, excluded_cores=excluded,
            core_mult=cm, core_mult_formula=formula,
            boost=b, boost_sources=list(boost_sources[p]),
            final_ndm=v,
        )
        total += v

    return total, per_slot_breakdown


# ──────────────────────────────────────────────────────────────────────────────
# Initial fill
# ──────────────────────────────────────────────────────────────────────────────

def _slot_rankings(deck: Deck) -> Dict[CardType, List[Position]]:
    """Per-positional-type slot rankings (best-first), by max possible peer count."""
    slots = list(deck.slots)
    return {
        CardType.ROW:  sorted(slots, key=lambda p: len(deck._row_peers[p]),  reverse=True),
        CardType.COL:  sorted(slots, key=lambda p: len(deck._col_peers[p]),  reverse=True),
        CardType.SURR: sorted(slots, key=lambda p: len(deck._surr_peers[p]), reverse=True),
        CardType.DIAG: sorted(slots, key=lambda p: len(deck._diag_peers[p]), reverse=True),
    }


_FILL_ORDER: Tuple[CardType, ...] = (
    CardType.SURR,
    CardType.ROW,
    CardType.COL,
    CardType.DIAG,
    CardType.DELUXE,
    CardType.TYPELESS,
)


def initial_fill(
    deck:      Deck,
    inventory: CardInventory,
) -> Dict[Position, Placed]:
    """Build a feasible starting assignment per the doc's heuristic.

    Phased fill:
      Phase 0: arcane slots. Pre-fill from inventory's ARCANE cards (largest
               color first). Slots exceeding supply get DEAD as the fallback —
               this is the only legal non-arcane placement in arcane slots.
      Phase 1: forced regulars. Confined to regular_slots — forced ARCANEs
               from inventory.forced_counts were already placed in Phase 0
               (caller's pre-flight ensures their total fits).
      Phase 2: regular cards. Also confined to regular_slots.
      Phase 3: remaining regular slots → DEAD.

    Arcane-slot rule: only ARCANE or DEAD ever lands in an arcane slot.
    """
    rankings  = _slot_rankings(deck)
    assignment: Dict[Position, Placed] = {}
    regular_slots = list(deck.regular_slots)
    arcane_slots  = list(deck.arcane_slots)

    # Working counters. ``remaining`` is the cap-aware budget the regular phase
    # will draw from; we decrement it as forced/arcane cards consume their share.
    remaining: Dict[Tuple[CardType, Color], int] = {}
    for k, n in inventory.counts.items():
        remaining[k] = n
    for k, n in inventory.forced_counts.items():
        remaining[k] = remaining.get(k, 0) + n
    forced_remaining: Dict[Tuple[CardType, Color], int] = dict(inventory.forced_counts)

    def _next_open(slot_order: Sequence[Position]) -> Optional[Position]:
        for s in slot_order:
            if s not in assignment:
                return s
        return None

    # ── Phase 0: arcane slots ─────────────────────────────────────────────────
    # Fill arcane slots with ARCANE cards drawn from inventory, biggest color
    # bucket first (heuristic — SA will optimize colors further if allowed).
    # Slots remaining after arcane supply is exhausted get DEAD.
    if arcane_slots:
        for slot in arcane_slots:
            # Pick the color with the most arcane cards still available.
            best_color: Optional[Color] = None
            best_avail = 0
            for c in Color:
                avail = remaining.get((CardType.ARCANE, c), 0)
                if avail > best_avail:
                    best_avail = avail
                    best_color = c
            if best_color is not None:
                assignment[slot] = (CardType.ARCANE, best_color)
                remaining[(CardType.ARCANE, best_color)] -= 1
                # Honor any forced ARCANE counts we just consumed.
                fkey = (CardType.ARCANE, best_color)
                if forced_remaining.get(fkey, 0) > 0:
                    forced_remaining[fkey] -= 1
            else:
                # No ARCANE inventory left — fall back to DEAD (still legal in
                # arcane slots).
                assignment[slot] = DEAD_CARD

    # ── Phase 1: forced cards (regular_slots only) ────────────────────────────
    # Positional / deluxe / typeless first via ranked slots.
    for card_type in _FILL_ORDER:
        # Skip ARCANE — already handled in Phase 0.
        if card_type == CardType.ARCANE:
            continue
        slot_order = [s for s in rankings.get(card_type, regular_slots) if s in deck.regular_slots]
        colors_sorted = sorted(
            Color,
            key=lambda c: forced_remaining.get((card_type, c), 0),
            reverse=True,
        )
        for color in colors_sorted:
            while forced_remaining.get((card_type, color), 0) > 0:
                slot = _next_open(slot_order)
                if slot is None:
                    break
                assignment[slot] = (card_type, color)
                forced_remaining[(card_type, color)] -= 1
                remaining[(card_type, color)] = remaining.get((card_type, color), 0) - 1
    # Catch any forced types NOT in _FILL_ORDER (greeds, etc.) — plain iteration
    # over regular slots only (ARCANE slots are off-limits to greeds).
    for (card_type, color), n in list(forced_remaining.items()):
        if card_type == CardType.ARCANE:
            continue
        while n > 0:
            slot = _next_open(regular_slots)
            if slot is None:
                break
            assignment[slot] = (card_type, color)
            n -= 1
            forced_remaining[(card_type, color)] -= 1
            remaining[(card_type, color)] = remaining.get((card_type, color), 0) - 1

    # ── Phase 2: regular cards (regular_slots only) ───────────────────────────
    for card_type in _FILL_ORDER:
        if card_type == CardType.ARCANE:
            continue
        slot_order = [s for s in rankings.get(card_type, regular_slots) if s in deck.regular_slots]
        colors_sorted = sorted(
            Color,
            key=lambda c: remaining.get((card_type, c), 0),
            reverse=True,
        )
        for color in colors_sorted:
            while remaining.get((card_type, color), 0) > 0:
                slot = _next_open(slot_order)
                if slot is None:
                    break
                assignment[slot] = (card_type, color)
                remaining[(card_type, color)] -= 1
        # Stop iterating types if every regular slot is filled.
        if all(s in assignment for s in regular_slots):
            break

    # ── Phase 3: pad with DEAD (regular slots only; arcane slots already filled)
    for s in deck.slots:
        if s not in assignment:
            assignment[s] = DEAD_CARD

    return assignment


# ──────────────────────────────────────────────────────────────────────────────
# Candidate cores (inventory-aware, adapted from src.simulate.candidate_cores)
# ──────────────────────────────────────────────────────────────────────────────

def candidate_cores_inventory(
    inventory: CardInventory,
    deck:      Deck,
) -> List[FrozenSet[CoreSpec]]:
    """Enumerate core combos to try, restricted to ``inventory.cores``.

    Reuses the grouping logic from ``src.simulate.candidate_cores``:
      * SHINY: PURE / DELUXE_CORE are *variable*; EQUILIBRIUM / STEADFAST /
        COLOR / FOIL are *static fillers*.
      * EVO without FOIL: PURE is analytically known → fillers; DELUXE_CORE is
        variable.
      * EVO with FOIL: PURE is variable again.

    Color cores are enumerated independently — for each candidate base, we
    consider "no color core" plus one option per available color in inventory.
    """
    k          = deck.core_slots
    available  = list(inventory.cores.cores)
    card_class = inventory.card_class

    by_type: Dict[CoreType, List[CoreSpec]] = {}
    for spec in available:
        by_type.setdefault(spec.core_type, []).append(spec)

    pure_spec        = (by_type.get(CoreType.PURE)        or [None])[0]
    deluxe_core_spec = (by_type.get(CoreType.DELUXE_CORE) or [None])[0]
    # Void is gated by mode (vanilla disables it). Read via _cfg so a runtime
    # set_mode() flip is honored without re-importing.
    void_core_spec   = (by_type.get(CoreType.VOID_CORE)   or [None])[0]
    if not _cfg.ALLOW_VOID:
        void_core_spec = None
    foil_spec        = (by_type.get(CoreType.FOIL)        or [None])[0]
    equi_spec        = (by_type.get(CoreType.EQUILIBRIUM) or [None])[0]
    stead_spec       = (by_type.get(CoreType.STEADFAST)   or [None])[0]
    color_specs      = list(by_type.get(CoreType.COLOR)   or [])

    color_choices: List[Optional[CoreSpec]] = [None] + color_specs

    candidates: List[FrozenSet[CoreSpec]] = []
    seen: set = set()

    def add(combo: FrozenSet[CoreSpec]) -> None:
        if combo not in seen:
            seen.add(combo); candidates.append(combo)

    # ── SHINY ────────────────────────────────────────────────────────────────
    if card_class == CardClass.SHINY:
        non_var_static = [s for s in (equi_spec, stead_spec, foil_spec) if s is not None]

        def best_shiny_fillers(slots_left: int, color_pick: Optional[CoreSpec]) -> FrozenSet[CoreSpec]:
            pool: List[CoreSpec] = list(non_var_static)
            if color_pick is not None:
                pool.append(color_pick)
            cap    = min(slots_left, len(pool))
            best_m = 0.0
            best_c: FrozenSet[CoreSpec] = frozenset()
            for size in range(0, cap + 1):
                for combo in (combinations(pool, size) if size > 0 else [()]):
                    m = 1.0
                    for c in combo:
                        m *= _static_mult(c)
                    if m > best_m:
                        best_m = m; best_c = frozenset(combo)
            return best_c

        var_pool: List[CoreSpec] = []
        if pure_spec        is not None: var_pool.append(pure_spec)
        if deluxe_core_spec is not None: var_pool.append(deluxe_core_spec)
        if void_core_spec   is not None: var_pool.append(void_core_spec)

        for color_pick in color_choices:
            for size in range(0, len(var_pool) + 1):
                for var_combo in (combinations(var_pool, size) if size > 0 else [()]):
                    var = frozenset(var_combo)
                    pre = set(var)
                    if color_pick is not None:
                        pre.add(color_pick)
                    if len(pre) > k:
                        continue
                    fillers = best_shiny_fillers(k - len(pre), color_pick)
                    # Fillers helper may re-pick color_pick; merge as a set so
                    # we don't double-count.
                    add(frozenset(pre | set(fillers)))
        return candidates

    # ── EVO ─────────────────────────────────────────────────────────────────
    # n_ns estimate for the "no FOIL" case: positional + deluxe + typeless +
    # arcane + greed ≈ total slot count (arcane slots are already in deck.slots
    # under the new model). With FOIL, PURE becomes variable.
    n_ns_full = len(deck.slots)

    def evo_no_foil_static_mult(spec: CoreSpec) -> float:
        if spec.core_type == CoreType.PURE:
            return _pure_mult(spec, n_ns_full)
        return _static_mult(spec)

    def best_fixed_evo_no_foil(slots_left: int, color_pick: Optional[CoreSpec]) -> FrozenSet[CoreSpec]:
        pool: List[CoreSpec] = []
        if pure_spec is not None: pool.append(pure_spec)
        if color_pick is not None: pool.append(color_pick)
        cap    = min(slots_left, len(pool))
        best_m = -1.0
        best_c: FrozenSet[CoreSpec] = frozenset()
        for size in range(0, cap + 1):
            for combo in (combinations(pool, size) if size > 0 else [()]):
                m = 1.0
                for c in combo:
                    m *= evo_no_foil_static_mult(c)
                if m > best_m:
                    best_m = m; best_c = frozenset(combo)
        return best_c

    def best_fixed_evo_with_foil(slots_left: int, color_pick: Optional[CoreSpec]) -> FrozenSet[CoreSpec]:
        # PURE is variable here; color core (if any) is the only static filler.
        if slots_left >= 1 and color_pick is not None and _static_mult(color_pick) > 1.0:
            return frozenset({color_pick})
        return frozenset()

    deluxe_var: List[CoreSpec] = [deluxe_core_spec] if deluxe_core_spec is not None else []
    # VOID joins the variable pool in both EVO groups (n_dead unknown pre-SA).
    void_var: List[CoreSpec] = [void_core_spec] if void_core_spec is not None else []

    # Group A: no FOIL (PURE is static)
    var_pool_a = list(deluxe_var) + list(void_var)
    for color_pick in color_choices:
        for size in range(0, len(var_pool_a) + 1):
            for var_combo in (combinations(var_pool_a, size) if size > 0 else [()]):
                var = frozenset(var_combo)
                pre = set(var)
                if color_pick is not None:
                    pre.add(color_pick)
                if len(pre) > k:
                    continue
                fillers = best_fixed_evo_no_foil(k - len(pre), color_pick)
                combo = frozenset(pre | set(fillers))
                add(combo)

    # Group B: with FOIL (PURE is variable)
    if foil_spec is not None:
        var_pool_b: List[CoreSpec] = []
        if pure_spec        is not None: var_pool_b.append(pure_spec)
        if deluxe_core_spec is not None: var_pool_b.append(deluxe_core_spec)
        if void_core_spec   is not None: var_pool_b.append(void_core_spec)

        for color_pick in color_choices:
            for size in range(0, len(var_pool_b) + 1):
                for var_combo in (combinations(var_pool_b, size) if size > 0 else [()]):
                    var = frozenset(var_combo)
                    pre = set(var) | {foil_spec}
                    if color_pick is not None:
                        pre.add(color_pick)
                    if len(pre) > k:
                        continue
                    fillers = best_fixed_evo_with_foil(k - len(pre), color_pick)
                    combo = frozenset(pre | set(fillers))
                    add(combo)

    return candidates


# ──────────────────────────────────────────────────────────────────────────────
# SA loop (pure Python — used as fallback and parity check)
# ──────────────────────────────────────────────────────────────────────────────

def _placeable_options(inventory: CardInventory) -> List[Placed]:
    """Concrete proposal alphabet: every (type, color) the user owns (regular
    OR forced) plus DEAD as a sentinel."""
    keys: set = set()
    for (t, c), n in inventory.counts.items():
        if n > 0:
            keys.add((t, c))
    for (t, c), n in inventory.forced_counts.items():
        if n > 0:
            keys.add((t, c))
    opts: List[Placed] = list(keys)
    opts.append(DEAD_CARD)
    return opts


def _initial_placed_counts(
    assignment: Dict[Position, Placed],
) -> Dict[Tuple[CardType, Color], int]:
    counts: Dict[Tuple[CardType, Color], int] = {}
    for (t, c) in assignment.values():
        if t == CardType.DEAD or c is None:
            continue
        key = (t, c)
        counts[key] = counts.get(key, 0) + 1
    return counts


def _sa_inventory_python(
    deck:              Deck,
    inventory:         CardInventory,
    cores:             FrozenSet[CoreSpec],
    n_iter:            int,
    auto_place_arcane: bool,
    T_start:           float = 100.0,
    T_end:             float = 0.5,
) -> Tuple[Dict[Position, Placed], float]:
    """Pure-Python SA for one (deck, inventory, cores) combo.

    Arcane-slot rules enforced here:
      * Arcane slots: only ARCANE or DEAD ever placed there.
      * Regular slots: never ARCANE.
      * Auto-place ON: arcane slots cannot be swapped to DEAD (locked to ARCANE
        unless inventory ran out, in which case they were pre-filled with DEAD
        and stay there). SA may still swap COLORS within ARCANE-locked slots.
    """
    assignment      = initial_fill(deck, inventory)
    score           = simulate_inventory(deck, assignment, inventory.card_class, cores)
    best_score      = score
    best_assignment = dict(assignment)

    placed_counts = _initial_placed_counts(assignment)
    options       = _placeable_options(inventory)
    # Pre-partition slots: arcane vs regular. Arcane slots have their own
    # restricted proposal alphabet; regular slots use the full option list.
    arcane_slots_set = deck.arcane_slots
    regular_slots    = [s for s in deck.slots if s not in arcane_slots_set]
    arcane_slots     = list(arcane_slots_set)
    all_slots        = list(deck.slots)

    # Restricted proposal alphabet for arcane slots: ARCANE colors the user has
    # in inventory (regular OR forced), plus DEAD. Auto-place ON drops DEAD —
    # arcane slots must stay ARCANE (or stay DEAD if inventory was exhausted
    # at initial_fill time; that state then cannot be changed either).
    arcane_options: List[Placed] = []
    for c in Color:
        if inventory.cap(CardType.ARCANE, c) > 0:
            arcane_options.append((CardType.ARCANE, c))
    if not auto_place_arcane:
        arcane_options.append(DEAD_CARD)

    # Auto-place ON: lock down arcane slots that were filled with DEAD at
    # initial_fill (inventory was exhausted) — they cannot be swapped either.
    # When ON, only ARCANE-occupied arcane slots are mutable, and only to other
    # ARCANE colors (subject to inventory).
    locked_arcane_slots: set = set()
    if auto_place_arcane:
        for s in arcane_slots:
            if assignment[s] == DEAD_CARD:
                locked_arcane_slots.add(s)

    log_cool      = math.log(T_end / T_start)

    def _within_inventory(p: Placed) -> bool:
        """Can we still place one more of `p` given current placed_counts?
        Cap = regular + forced for this (type, color)."""
        if p == DEAD_CARD:
            return True
        t, c = p
        if c is None:
            return True
        return placed_counts.get((t, c), 0) < inventory.cap(t, c)

    def _can_remove(p: Placed) -> bool:
        """Removing one of `p` must not drop placed_counts below the forced min."""
        if p == DEAD_CARD:
            return True
        t, c = p
        if c is None:
            return True
        return placed_counts.get((t, c), 0) > inventory.min_required(t, c)

    def _slot_legal(p: Position, placed: Placed) -> bool:
        """Per-slot legality check enforcing the ARCANE-slot rule."""
        t = placed[0]
        if p in arcane_slots_set:
            return t in ARCANE_SLOT_LEGAL
        return t != CardType.ARCANE

    def _apply(p: Position, old: Placed, new: Placed) -> None:
        if old != DEAD_CARD and old[1] is not None:
            placed_counts[(old[0], old[1])] -= 1
        if new != DEAD_CARD and new[1] is not None:
            placed_counts[(new[0], new[1])] = placed_counts.get((new[0], new[1]), 0) + 1
        assignment[p] = new

    # Slot proposal weights: avoid picking locked arcane slots in replace moves
    # (they can't be changed under auto-place ON). If every slot is locked or
    # there are no arcane slots, slot_pool = all_slots.
    if locked_arcane_slots:
        slot_pool = [s for s in all_slots if s not in locked_arcane_slots]
    else:
        slot_pool = all_slots

    for i in range(n_iter):
        T = T_start * math.exp(log_cool * i / n_iter)

        if len(all_slots) < 2 or random.random() < 0.80:
            # ── Replace move ─────────────────────────────────────────────────
            if not slot_pool:
                continue
            p   = random.choice(slot_pool)
            old = assignment[p]
            # Use the arcane proposal alphabet when targeting an arcane slot.
            if p in arcane_slots_set:
                if not arcane_options:
                    continue
                new = random.choice(arcane_options)
            else:
                new = random.choice(options)
                # Reject ARCANE-into-regular-slot proposals up front.
                if new[0] == CardType.ARCANE:
                    continue
            if new == old:
                continue
            if not _slot_legal(p, new):
                continue
            if not _within_inventory(new):
                continue
            if not _can_remove(old):     # forced-min constraint
                continue
            _apply(p, old, new)
            new_score = simulate_inventory(deck, assignment, inventory.card_class, cores)
            delta     = new_score - score
            if delta >= 0 or random.random() < math.exp(delta / T):
                score = new_score
                if score > best_score:
                    best_score = score; best_assignment = dict(assignment)
            else:
                _apply(p, new, old)
        else:
            # ── Pair-swap move ─────────────────────────────────────────────────
            p1, p2 = random.sample(all_slots, 2)
            if assignment[p1] == assignment[p2]:
                continue
            # Reject swaps that would violate the arcane-slot rule on either
            # endpoint (e.g. swapping ARCANE↔regular card across the slot-type
            # boundary). Also reject any swap involving a locked arcane slot
            # under auto-place ON.
            if not _slot_legal(p1, assignment[p2]) or not _slot_legal(p2, assignment[p1]):
                continue
            if p1 in locked_arcane_slots or p2 in locked_arcane_slots:
                continue
            old1, old2 = assignment[p1], assignment[p2]
            assignment[p1], assignment[p2] = old2, old1
            new_score = simulate_inventory(deck, assignment, inventory.card_class, cores)
            delta     = new_score - score
            if delta >= 0 or random.random() < math.exp(delta / T):
                score = new_score
                if score > best_score:
                    best_score = score; best_assignment = dict(assignment)
            else:
                assignment[p1], assignment[p2] = old1, old2

    return best_assignment, best_score


# ──────────────────────────────────────────────────────────────────────────────
# Top-level wrapper (Rust dispatch + pure-Python fallback)
# ──────────────────────────────────────────────────────────────────────────────

try:
    import ndm_core as _ndm_core
    _RUST_OK = hasattr(_ndm_core, "run_sa_inventory")
except Exception:
    _ndm_core = None
    _RUST_OK = False


def optimize_inventory(
    deck:              Deck,
    inventory:         CardInventory,
    n_iter:            int = 60_000,
    restarts:          int = 12,
    auto_place_arcane: Optional[bool] = None,
) -> InventoryResult:
    """Run inventory-constrained SA across every viable core combo.

    Returns the single best ``InventoryResult``. Tries the Rust core when
    available (parallel restarts via rayon); falls back to a pure-Python loop
    that runs restarts serially.

    ``auto_place_arcane`` defaults to ``_cfg.AUTO_PLACE_ARCANE`` when None.
    When True (the historical behavior), arcane slots are filled with ARCANE
    and SA cannot swap them to DEAD; it may still reshuffle ARCANE colors.
    When False, SA may freely choose ARCANE/DEAD per arcane slot.

    Raises ``ValueError`` if the user's forced inventory exceeds deck capacity.
    """
    # Default to the config-level toggle when caller doesn't override.
    if auto_place_arcane is None:
        auto_place_arcane = _cfg.AUTO_PLACE_ARCANE

    # Pre-flight: forced cards must fit in the deck.
    total_forced = inventory.total_forced()
    if total_forced > len(deck.slots):
        raise ValueError(
            f"Forced inventory ({total_forced} cards) exceeds deck capacity "
            f"({len(deck.slots)} slots) — remove some forced entries."
        )
    # Forced ARCANE cards must fit in the arcane slots specifically (since
    # they can't be placed anywhere else).
    forced_arcane = sum(
        n for (t, _c), n in inventory.forced_counts.items()
        if t == CardType.ARCANE
    )
    if forced_arcane > len(deck.arcane_slots):
        raise ValueError(
            f"Forced ARCANE inventory ({forced_arcane} cards) exceeds arcane "
            f"slot count ({len(deck.arcane_slots)}) — arcane cards can only "
            f"go in arcane slots."
        )

    candidates = candidate_cores_inventory(inventory, deck)
    if not candidates:
        # No cores at all — still run with the empty set.
        candidates = [frozenset()]

    best: Optional[InventoryResult] = None

    for cores in candidates:
        asgn, score = _run_one_combo(deck, inventory, cores, n_iter, restarts, auto_place_arcane)
        if best is None or score > best.score:
            best = InventoryResult(assignment=asgn, score=score, cores_used=cores)

    assert best is not None

    # Re-score the final assignment in Python to populate the breakdown and
    # cross-check the Rust-reported total.
    python_total, breakdown = simulate_inventory_breakdown(
        deck, best.assignment, inventory.card_class, best.cores_used
    )
    best.per_slot_breakdown = breakdown
    best.per_slot_ndm = {p: bd.final_ndm for p, bd in breakdown.items()}
    best.python_score = python_total
    if _RUST_OK:
        # ``best.score`` came from the Rust SA loop (= Rust's simulate of the
        # final assignment, deterministic given the assignment). Keep it as
        # the cross-check reference.
        best.rust_score = best.score
    else:
        best.rust_score = None

    return best


def _run_one_combo(
    deck:              Deck,
    inventory:         CardInventory,
    cores:             FrozenSet[CoreSpec],
    n_iter:            int,
    restarts:          int,
    auto_place_arcane: bool,
) -> Tuple[Dict[Position, Placed], float]:
    if _RUST_OK and _ndm_core is not None:
        return _run_one_combo_rust(deck, inventory, cores, n_iter, restarts, auto_place_arcane)

    # Pure-Python: serial restarts, keep best.
    best_score = -1.0
    best_asgn: Dict[Position, Placed] = {}
    for _ in range(restarts):
        asgn, score = _sa_inventory_python(deck, inventory, cores, n_iter, auto_place_arcane)
        if score > best_score:
            best_score = score; best_asgn = asgn
    return best_asgn, best_score


# ──────────────────────────────────────────────────────────────────────────────
# Rust marshalling
# ──────────────────────────────────────────────────────────────────────────────

def _run_one_combo_rust(
    deck:              Deck,
    inventory:         CardInventory,
    cores:             FrozenSet[CoreSpec],
    n_iter:            int,
    restarts:          int,
    auto_place_arcane: bool,
) -> Tuple[Dict[Position, Placed], float]:
    slots_list = list(deck.slots)
    slot_order = {p: i for i, p in enumerate(slots_list)}
    arcane_slot_indices = [slot_order[p] for p in deck.arcane_slots]

    def _peers(d: Dict[Position, FrozenSet[Position]]) -> List[List[int]]:
        return [[slot_order[q] for q in d[p]] for p in slots_list]

    # Inventory as a flat list of (type_str, color_str, count) tuples.
    inv_list = [
        (t.value, c.value, n)
        for (t, c), n in inventory.counts.items() if n > 0
    ]
    forced_list = [
        (t.value, c.value, n)
        for (t, c), n in inventory.forced_counts.items() if n > 0
    ]

    # Cores as (core_type_str, color_str_or_empty, override_or_negative).
    cores_list = [
        (
            s.core_type.value,
            s.color.value if s.color is not None else "",
            -1.0 if s.override is None else float(s.override),
        )
        for s in cores
    ]

    result_strs, score = _ndm_core.run_sa_inventory(  # type: ignore[attr-defined]
        slots                = slots_list,
        row_peers            = _peers(deck._row_peers),
        col_peers            = _peers(deck._col_peers),
        surr_peers           = _peers(deck._surr_peers),
        diag_peers           = _peers(deck._diag_peers),
        arcane_slot_indices  = arcane_slot_indices,
        auto_place_arcane    = auto_place_arcane,
        is_shiny             = (inventory.card_class == CardClass.SHINY),
        inventory            = inv_list,
        forced_inventory     = forced_list,
        cores                = cores_list,
        n_iter               = n_iter,
        restarts             = restarts,
        # Multiplier constants — the Rust core needs config values for defaults.
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
        # Flags — read via _cfg so a runtime set_mode() takes effect on the
        # next Rust call without re-importing this module.
        greed_additive = GREED_ADDITIVE,
        additive_cores = _cfg.ADDITIVE_CORES,
    )

    assignment: Dict[Position, Placed] = {}
    for i, (t_s, c_s) in enumerate(result_strs):
        t = CardType(t_s)
        c = Color(c_s) if c_s else None
        assignment[slots_list[i]] = (t, c)
    return assignment, float(score)
