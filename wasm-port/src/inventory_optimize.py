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
    """All the cards + cores available for one optimization run."""
    counts:     Dict[Tuple[CardType, Color], int]
    card_class: CardClass
    cores:      CoreInventory = field(default_factory=CoreInventory)

    def total_cards(self) -> int:
        return sum(self.counts.values())


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


def _classify_cores(
    cores:      FrozenSet[CoreSpec],
    card_class: CardClass,
    n_ns:       int,
    n_deluxe:   int,
    n_arcane:   int,
) -> Tuple[List["CoreComponent"], Optional["CoreComponent"], Optional["CoreComponent"], List["ExcludedCore"]]:
    """Sort cores into baseline / color-core / deluxe-core buckets.

    Returns ``(baseline, color_comp, deluxe_comp, class_excluded)``:
      * ``baseline`` — cores that apply to every non-greed scoring card
        regardless of color (PURE, FOIL, plus EQUI/STEAD when class is SHINY).
      * ``color_comp`` — the active COLOR core, if any. Gates per-card by color.
      * ``deluxe_comp`` — the active DELUXE_CORE, if any. Gates per-card by
        "is this a deluxe card."
      * ``class_excluded`` — cores excluded by the card-class rule (EQUI/STEAD
        when class is EVO). Same for every slot in the run, so we precompute
        these once.
    """
    baseline:   List[CoreComponent]      = []
    color_comp: Optional[CoreComponent]  = None
    deluxe_comp: Optional[CoreComponent] = None
    class_excluded: List[ExcludedCore]   = []

    for spec in cores:
        is_override = spec.override is not None
        if spec.core_type == CoreType.PURE:
            v = _pure_mult(spec, n_ns + n_arcane)
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

    return baseline, color_comp, deluxe_comp, class_excluded


# ──────────────────────────────────────────────────────────────────────────────
# Scoring kernel (color-aware)
# ──────────────────────────────────────────────────────────────────────────────

def _apply_greed(boost: Dict[Position, float], pos: Position, amount: float) -> None:
    """Same semantics as simulate.py's _apply_greed — additive or multiplicative."""
    if pos in boost:
        if GREED_ADDITIVE: boost[pos] += amount - 1
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

    for p, (t, _c) in assignment.items():
        if   t == CardType.DEAD:        continue
        elif t in POSITIONAL_TYPES:     positional[p] = (t, _c)
        elif t == CardType.DELUXE:      deluxe[p]     = (t, _c)
        elif t == CardType.TYPELESS:    typeless[p]   = (t, _c)
        elif t in GREED_TYPES_NEW:      greed[p]      = (t, _c)
        # FILLER_GREED, EMPTY etc. fall through and are ignored.

    # Same-color counts. `colored` is every non-dead placed card with its color.
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

    # n_ns for PURE — same rule as the existing simulate(): SHINY and
    # EVO-with-FOIL count greed only; EVO-without-FOIL counts non-greed too.
    if card_class == CardClass.EVO:
        n_ns = len(greed) if foil_active else (len(positional) + len(deluxe) + len(typeless) + len(greed))
    else:
        n_ns = len(greed)
    n_deluxe = len(deluxe)

    # All cores fold into ONE per-card ``core_mult``. Precompute the baseline
    # (cores that apply to every non-greed card) and the color- and deluxe-gated
    # addends so each card's multiplier is a cheap constant-time combination.
    baseline, color_comp, deluxe_comp, _ex = _classify_cores(
        cores, card_class, n_ns, n_deluxe, deck.n_arcane,
    )
    baseline_sum  = sum(c.value - 1.0 for c in baseline)
    baseline_prod = math.prod(c.value for c in baseline) if baseline else 1.0
    color_core_color = color_comp.color if color_comp is not None else None
    color_addend  = (color_comp.value - 1.0) if color_comp is not None else 0.0
    color_factor  = color_comp.value         if color_comp is not None else 1.0
    deluxe_addend = (deluxe_comp.value - 1.0) if deluxe_comp is not None else 0.0
    deluxe_factor = deluxe_comp.value         if deluxe_comp is not None else 1.0

    def _card_core_mult(card_type: CardType, card_color: Optional[Color]) -> float:
        """Combined per-card core multiplier — color & deluxe cores fold in here
        too (gated by color match and is-not-deluxe respectively)."""
        color_applies  = (
            color_comp is not None
            and card_color is not None
            and card_color == color_core_color
        )
        deluxe_applies = (deluxe_comp is not None and card_type != CardType.DELUXE)
        if _cfg.ADDITIVE_CORES:
            return (1.0
                    + baseline_sum
                    + (color_addend if color_applies else 0.0)
                    + (deluxe_addend if deluxe_applies else 0.0))
        m = baseline_prod
        if color_applies:  m *= color_factor
        if deluxe_applies: m *= deluxe_factor
        return m

    # Greed-boost map (per target slot).
    scorable_positions = set(positional) | set(deluxe) | set(typeless)
    init = 1.0
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

    for p, (t, c) in assignment.items():
        if   t == CardType.DEAD:        continue
        elif t in POSITIONAL_TYPES:     positional[p] = (t, c)
        elif t == CardType.DELUXE:      deluxe[p]     = (t, c)
        elif t == CardType.TYPELESS:    typeless[p]   = (t, c)
        elif t in GREED_TYPES_NEW:      greed[p]      = (t, c)

    colored: Dict[Position, Color] = {}
    for p, (t, c) in assignment.items():
        if t == CardType.DEAD or c is None:
            continue
        colored[p] = c

    row_count: Dict[Tuple[int, Color], int] = {}
    col_count: Dict[Tuple[int, Color], int] = {}
    for (r, ccol), color in colored.items():
        row_count[(r, color)]    = row_count.get((r, color), 0)    + 1
        col_count[(ccol, color)] = col_count.get((ccol, color), 0) + 1

    foil_active = any(s.core_type == CoreType.FOIL for s in cores)
    if card_class == CardClass.EVO:
        n_ns = len(greed) if foil_active else (len(positional) + len(deluxe) + len(typeless) + len(greed))
    else:
        n_ns = len(greed)
    n_deluxe = len(deluxe)

    # Classify cores once. The breakdown for each slot picks from these buckets
    # depending on the slot's color and whether it's a deluxe card.
    baseline, color_comp, deluxe_comp, class_excluded = _classify_cores(
        cores, card_class, n_ns, n_deluxe, deck.n_arcane,
    )

    scorable_positions = set(positional) | set(deluxe) | set(typeless)
    boost: Dict[Position, float] = {p: 1.0 for p in scorable_positions}
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

    # Greed/dead/empty slots — non-scoring.
    for p in deck.slots:
        if p in scorable_positions:
            continue
        if p in greed:
            gt, gc = greed[p]
            per_slot_breakdown[p] = _zero_breakdown(p, gt, gc, "greed card — provides boost to neighbors, no own NDM")
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
    """Build a feasible starting assignment per the doc's heuristic."""
    rankings  = _slot_rankings(deck)
    assignment: Dict[Position, Placed] = {}
    remaining: Dict[Tuple[CardType, Color], int] = dict(inventory.counts)
    all_slots = set(deck.slots)

    def _next_open(slot_order: Sequence[Position]) -> Optional[Position]:
        for s in slot_order:
            if s not in assignment:
                return s
        return None

    for card_type in _FILL_ORDER:
        slot_order = rankings.get(card_type, list(deck.slots))
        # Colors of this type sorted by remaining stack size, descending.
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
        if len(assignment) == len(all_slots):
            break

    # Fill remaining slots with DEAD.
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
    # n_ns estimate for the "no FOIL" case: positional + deluxe + typeless + greed
    # ≈ deck size. With FOIL, PURE becomes variable.
    n_ns_full = len(deck.slots) + deck.n_arcane

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

    # Group A: no FOIL (PURE is static)
    var_pool_a = list(deluxe_var)
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
                add(frozenset(pre | set(fillers)))

    # Group B: with FOIL (PURE is variable)
    if foil_spec is not None:
        var_pool_b: List[CoreSpec] = []
        if pure_spec        is not None: var_pool_b.append(pure_spec)
        if deluxe_core_spec is not None: var_pool_b.append(deluxe_core_spec)

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
                    add(frozenset(pre | set(fillers)))

    return candidates


# ──────────────────────────────────────────────────────────────────────────────
# SA loop (pure Python — used as fallback and parity check)
# ──────────────────────────────────────────────────────────────────────────────

def _placeable_options(inventory: CardInventory) -> List[Placed]:
    """Concrete proposal alphabet: every (type, color) the user owns + DEAD."""
    opts: List[Placed] = [(t, c) for (t, c), n in inventory.counts.items() if n > 0]
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
    deck:      Deck,
    inventory: CardInventory,
    cores:     FrozenSet[CoreSpec],
    n_iter:    int,
    T_start:   float = 100.0,
    T_end:     float = 0.5,
) -> Tuple[Dict[Position, Placed], float]:
    """Pure-Python SA for one (deck, inventory, cores) combo."""
    assignment      = initial_fill(deck, inventory)
    score           = simulate_inventory(deck, assignment, inventory.card_class, cores)
    best_score      = score
    best_assignment = dict(assignment)

    placed_counts = _initial_placed_counts(assignment)
    options       = _placeable_options(inventory)
    slots         = list(deck.slots)
    log_cool      = math.log(T_end / T_start)

    def _within_inventory(p: Placed) -> bool:
        """Can we still place one more of `p` given current placed_counts?"""
        if p == DEAD_CARD:
            return True
        t, c = p
        if c is None:
            return True
        key   = (t, c)
        return placed_counts.get(key, 0) < inventory.counts.get(key, 0)

    def _apply(p: Position, old: Placed, new: Placed) -> None:
        if old != DEAD_CARD and old[1] is not None:
            placed_counts[(old[0], old[1])] -= 1
        if new != DEAD_CARD and new[1] is not None:
            placed_counts[(new[0], new[1])] = placed_counts.get((new[0], new[1]), 0) + 1
        assignment[p] = new

    for i in range(n_iter):
        T = T_start * math.exp(log_cool * i / n_iter)

        if len(slots) < 2 or random.random() < 0.80:
            # ── Replace move ─────────────────────────────────────────────────
            p   = random.choice(slots)
            old = assignment[p]
            new = random.choice(options)
            if new == old:
                continue
            if not _within_inventory(new):
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
            p1, p2 = random.sample(slots, 2)
            if assignment[p1] == assignment[p2]:
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
    deck:      Deck,
    inventory: CardInventory,
    n_iter:    int = 60_000,
    restarts:  int = 12,
) -> InventoryResult:
    """Run inventory-constrained SA across every viable core combo.

    Returns the single best ``InventoryResult``. Tries the Rust core when
    available (parallel restarts via rayon); falls back to a pure-Python loop
    that runs restarts serially.
    """
    candidates = candidate_cores_inventory(inventory, deck)
    if not candidates:
        # No cores at all — still run with the empty set.
        candidates = [frozenset()]

    best: Optional[InventoryResult] = None

    for cores in candidates:
        asgn, score = _run_one_combo(deck, inventory, cores, n_iter, restarts)
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
    deck:      Deck,
    inventory: CardInventory,
    cores:     FrozenSet[CoreSpec],
    n_iter:    int,
    restarts:  int,
) -> Tuple[Dict[Position, Placed], float]:
    if _RUST_OK and _ndm_core is not None:
        return _run_one_combo_rust(deck, inventory, cores, n_iter, restarts)

    # Pure-Python: serial restarts, keep best.
    best_score = -1.0
    best_asgn: Dict[Position, Placed] = {}
    for _ in range(restarts):
        asgn, score = _sa_inventory_python(deck, inventory, cores, n_iter)
        if score > best_score:
            best_score = score; best_asgn = asgn
    return best_asgn, best_score


# ──────────────────────────────────────────────────────────────────────────────
# Rust marshalling
# ──────────────────────────────────────────────────────────────────────────────

def _run_one_combo_rust(
    deck:      Deck,
    inventory: CardInventory,
    cores:     FrozenSet[CoreSpec],
    n_iter:    int,
    restarts:  int,
) -> Tuple[Dict[Position, Placed], float]:
    slots_list = list(deck.slots)
    slot_order = {p: i for i, p in enumerate(slots_list)}

    def _peers(d: Dict[Position, FrozenSet[Position]]) -> List[List[int]]:
        return [[slot_order[q] for q in d[p]] for p in slots_list]

    # Inventory as a flat list of (type_str, color_str, count) tuples.
    inv_list = [
        (t.value, c.value, n)
        for (t, c), n in inventory.counts.items() if n > 0
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
        slots      = slots_list,
        row_peers  = _peers(deck._row_peers),
        col_peers  = _peers(deck._col_peers),
        surr_peers = _peers(deck._surr_peers),
        diag_peers = _peers(deck._diag_peers),
        n_arcane   = deck.n_arcane,
        is_shiny   = (inventory.card_class == CardClass.SHINY),
        inventory  = inv_list,
        cores      = cores_list,
        n_iter     = n_iter,
        restarts   = restarts,
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
