"""Type aliases, card-type enums, and the shared PLACEABLE list.

Pure data-only module. Mutated at import time by ``src.config`` (which
appends ``CardType.DELUXE`` to ``PLACEABLE`` when ``ALLOW_DELUXE`` is true).
"""
from __future__ import annotations

from enum import Enum
from typing import FrozenSet, List, Tuple

# ──────────────────────────────────────────────────────────────────────────────
# Aliases
# ──────────────────────────────────────────────────────────────────────────────

Position = Tuple[int, int]


# ──────────────────────────────────────────────────────────────────────────────
# Enumerations
# ──────────────────────────────────────────────────────────────────────────────

class CardType(Enum):
    ROW             = "row"
    COL             = "col"
    SURR            = "surr"
    DIAG            = "diag"
    DELUXE          = "deluxe"
    TYPELESS        = "typeless"
    DIR_GREED_UP    = "dir_greed_up"
    DIR_GREED_DOWN  = "dir_greed_down"
    DIR_GREED_LEFT  = "dir_greed_left"
    DIR_GREED_RIGHT = "dir_greed_right"
    DIR_GREED_NE = "dir_greed_ne"
    DIR_GREED_NW = "dir_greed_nw"
    DIR_GREED_SE = "dir_greed_se"
    DIR_GREED_SW = "dir_greed_sw"
    EVO_GREED       = "evo_greed"
    SURR_GREED      = "surr_greed"
    FILLER_GREED    = "filler_greed"
    EMPTY           = "empty"
    # Inventory optimizer only: transparent slot-filler when inventory is exhausted.
    # Contributes nothing to NDM and does not participate in any same-color count.
    DEAD            = "dead"


class Color(Enum):
    """Card color, used only by the inventory-based optimizer.

    Positional bonuses count only same-color cards in scan range; the COLOR
    core only boosts matching-color cards. The classic optimizer is color-blind
    and never uses this enum.
    """
    RED    = "red"
    GREEN  = "green"
    BLUE   = "blue"
    YELLOW = "yellow"


GREED_TYPES    = frozenset({
    CardType.DIR_GREED_UP,    CardType.DIR_GREED_DOWN,
    CardType.DIR_GREED_LEFT,  CardType.DIR_GREED_RIGHT,
    CardType.DIR_GREED_NE,    CardType.DIR_GREED_NW,
    CardType.DIR_GREED_SE,    CardType.DIR_GREED_SW,
    CardType.EVO_GREED,       CardType.SURR_GREED,
    CardType.FILLER_GREED,
})
REGULAR_TYPES  = frozenset({CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG})
DELUXE_TYPES   = frozenset({CardType.DELUXE})
TYPELESS_TYPES = frozenset({CardType.TYPELESS})

PLACEABLE: List[CardType] = [
    CardType.ROW,             CardType.COL,            CardType.SURR,
    CardType.DIAG,
    CardType.DIR_GREED_UP,    CardType.DIR_GREED_DOWN,
    CardType.DIR_GREED_LEFT,  CardType.DIR_GREED_RIGHT,
    CardType.DIR_GREED_NE,    CardType.DIR_GREED_NW,
    CardType.DIR_GREED_SE,    CardType.DIR_GREED_SW,
    CardType.EVO_GREED,       CardType.SURR_GREED,
]


class CardClass(Enum):
    EVO   = "evo"
    SHINY = "shiny"


class CoreType(Enum):
    PURE        = "pure"
    EQUILIBRIUM = "equilibrium"
    STEADFAST   = "steadfast"
    COLOR       = "color"
    FOIL        = "foil"
    DELUXE_CORE = "deluxe_core"
