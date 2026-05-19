"""Configuration + Deck definitions.

On import this module:
  * parses ``--mode`` from ``sys.argv``
  * loads ``config.yaml`` (with mode overrides)
  * exposes every tunable as an UPPERCASE module constant
  * defines the ``Deck`` class and loads ``decks/*.yaml`` into ``DECKS``
"""
from __future__ import annotations

from pathlib import Path
from typing import Any, Dict, FrozenSet, List, Optional, Set, Tuple

import argparse as _argparse
import json
import re
import yaml

from .types import (
    CardType, CardClass, Position,
    PLACEABLE, REGULAR_TYPES, DELUXE_TYPES,
)

_mode_parser = _argparse.ArgumentParser(
    description="Wold's Vaults / Vanilla NDM deck optimizer.",
    add_help=True,
)
_mode_parser.add_argument(
    "--mode",
    choices=("wolds", "vanilla"),
    default="wolds",
    help="Optimizer preset: 'wolds' (default) or 'vanilla'.",
)
MODE: str = _mode_parser.parse_known_args()[0].mode
_VANILLA: bool = MODE == "vanilla"

# ──────────────────────────────────────────────────────────────────────────────
# Configuration (config.yaml + decks/*.yaml)
# ──────────────────────────────────────────────────────────────────────────────

_HERE = Path(__file__).resolve().parent.parent  # project root (parent of src/)
_CONFIG_PATH = _HERE / "config.yaml"
_DECKS_DIR = _HERE / "decks"


def _deep_merge(base: Dict[str, Any], override: Dict[str, Any]) -> Dict[str, Any]:
    """Recursively merge ``override`` into ``base`` (mutating ``base``)."""
    for key, value in override.items():
        if isinstance(value, dict) and isinstance(base.get(key), dict):
            _deep_merge(base[key], value)
        else:
            base[key] = value
    return base


with _CONFIG_PATH.open("r", encoding="utf-8") as _f:
    _CFG: Dict[str, Any] = yaml.safe_load(_f)

_mode_overrides = (_CFG.get("modes") or {}).get(MODE, {}) or {}
_deep_merge(_CFG, _mode_overrides)

# ──────────────────────────────────────────────────────────────────────────────
# Tunable multiplier constants (loaded from config.yaml)
# ──────────────────────────────────────────────────────────────────────────────

DECKMOD: int = _CFG["deckmod"]

EXCLUDED_DECKS: FrozenSet[str] = frozenset(_CFG.get("excluded_decks") or ())

MULT_DIR_GREED_VERT:      float = _CFG["greed"]["dir_vert"]
MULT_DIR_GREED_HORIZ:     float = _CFG["greed"]["dir_horiz"]
MULT_EVO_GREED:           float = _CFG["greed"]["evo"]
MULT_SURR_GREED:          float = _CFG["greed"]["surr"]
MULT_DIR_GREED_DIAG_UP:   float = _CFG["greed"]["dir_diag_up"]
MULT_DIR_GREED_DIAG_DOWN: float = _CFG["greed"]["dir_diag_down"]

MULT_PURE_BASE:   float = _CFG["cores"]["pure_base"]
MULT_PURE_SCALE:  float = _CFG["cores"]["pure_scale"]
MULT_EQUILIBRIUM: float = _CFG["cores"]["equilibrium"]
MULT_FOIL:        float = _CFG["cores"]["foil"]
MULT_STEADFAST:   float = _CFG["cores"]["steadfast"]
MULT_COLOR:       float = _CFG["cores"]["color"]

ALLOW_DELUXE:           bool  = _CFG["deluxe"]["allow"]
MULT_DELUXE_FLAT:       float = _CFG["deluxe"]["flat"]
MULT_DELUXE_CORE_BASE:  float = _CFG["deluxe"]["core_base"]
MULT_DELUXE_CORE_SCALE: float = _CFG["deluxe"]["core_scale"]

GREED_ADDITIVE: bool = _CFG["stacking"]["greed_additive"]
ADDITIVE_CORES: bool = _CFG["stacking"]["additive_cores"]

SHINY_POSITIONAL: bool = _CFG["shiny"]["positional"]

ENABLE_EXPERIMENTAL_EXPONENT: bool  = _CFG["experimental"]["enable_exponent"]
EXPERIMENTAL_EXPONENT:        float = _CFG["experimental"]["exponent"]
EXPERIMENTAL_BOOST:           float = _CFG["experimental"]["boost"]

DELUXE_COUNTED_AS_REGULAR: bool = _CFG["constraints"]["deluxe_counted_as_regular"]

# Display options
BALANCE_DISPLAY: bool = _CFG["display"]["balance"]
HEATMAP_DISPLAY: bool = _CFG["display"]["heatmap"]

HNS_Q:         float = _CFG["metrics"]["hns_q"]
SPREAD_METRIC: bool  = _CFG["metrics"]["spread"]

# Spreadsheet export
EXPORT_SPREADSHEET: bool = _CFG["output"]["export_spreadsheet"]
SPREADSHEET_PREFIX: str  = _CFG["output"]["spreadsheet_prefix"]

# Full test panel: runs the configured constraint configs per deck/class
# instead of the deck's own min_regular / max_greed settings.
FULL_TEST_PANEL: bool = _CFG["testing"]["full_panel"]

if ALLOW_DELUXE:
    PLACEABLE.append(CardType.DELUXE)


class Deck:
    def __init__(
        self,
        slots:       Set[Position],
        core_slots:  int,
        n_arcane:    int = 0,
        min_regular: int = -1,
        max_greed:   int = -1,
        name:        str = "Unnamed Deck",
    ) -> None:
        self.slots       = frozenset(slots)
        self.core_slots  = core_slots
        self.n_arcane    = n_arcane
        self.min_regular = min_regular
        self.max_greed   = max_greed
        self.name        = name

        self._row_peers:  Dict[Position, FrozenSet[Position]] = {}
        self._col_peers:  Dict[Position, FrozenSet[Position]] = {}
        self._surr_peers: Dict[Position, FrozenSet[Position]] = {}
        self._diag_peers: Dict[Position, FrozenSet[Position]] = {}

        for p in self.slots:
            r, c = p
            self._row_peers[p]  = frozenset(q for q in self.slots if q[0] == r and q != p)
            self._col_peers[p]  = frozenset(q for q in self.slots if q[1] == c and q != p)
            self._surr_peers[p] = frozenset(
                q for q in self.slots if q != p and max(abs(q[0]-r), abs(q[1]-c)) <= 1)
            for p in self.slots:
                r, c = p
                self._diag_peers[p] = frozenset(
                    q for q in self.slots
                    if q != p and (
                        (q[0] - q[1] == r - c) or   # NW-SE diagonal
                        (q[0] + q[1] == r + c)       # NE-SW diagonal
                    )
                )

    _CHAR: Dict[CardType, str] = {
        CardType.ROW:             "R",
        CardType.COL:             "C",
        CardType.SURR:            "S",
        CardType.DIAG:            "X",
        CardType.DELUXE:          "D",
        CardType.TYPELESS:        "T",
        CardType.DIR_GREED_UP:    "^",
        CardType.DIR_GREED_DOWN:  "v",
        CardType.DIR_GREED_LEFT:  "<",
        CardType.DIR_GREED_RIGHT: ">",
        CardType.DIR_GREED_NE:    "↗",
        CardType.DIR_GREED_NW:    "↖",
        CardType.DIR_GREED_SE:    "↘",
        CardType.DIR_GREED_SW:    "↙",
        CardType.EVO_GREED:       "e",
        CardType.SURR_GREED:      "o",
        CardType.FILLER_GREED:    ".",
        CardType.EMPTY:           "·",
    }

    def display(self, assignment: Optional[Dict[Position, CardType]] = None) -> str:
        if not self.slots:
            return "(empty deck)"
        min_r = min(r for r, _ in self.slots); max_r = max(r for r, _ in self.slots)
        min_c = min(c for _, c in self.slots); max_c = max(c for _, c in self.slots)
        lines = []
        for r in range(min_r, max_r + 1):
            cells = []
            for c in range(min_c, max_c + 1):
                p = (r, c)
                if   p not in self.slots:              cells.append(" ")
                elif assignment and p in assignment:   cells.append(self._CHAR[assignment[p]])
                else:                                  cells.append("□")
            lines.append(" ".join(cells))
        return "\n".join(lines)

    def with_constraints(self, min_regular: int, max_greed: int) -> "Deck":
        """Return a shallow copy of this deck with overridden constraints."""
        return Deck(
            self.slots, self.core_slots, self.n_arcane,
            min_regular, max_greed, self.name,
        )

    def constraint_str(self) -> str:
        """Human-readable summary of this deck's min_regular / max_greed."""
        n = len(self.slots)
        overridden = (
            self.min_regular >= 0
            and self.max_greed  >= 0
            and self.min_regular + self.max_greed > n
        )
        parts = []
        if self.min_regular >= 0:
            note = " (overridden)" if overridden else ""
            parts.append(f"min {self.min_regular} regular{note}")
        if self.max_greed >= 0:
            parts.append(f"max {self.max_greed} greed")
        return "  |  ".join(parts) if parts else "unconstrained"

    def __repr__(self) -> str:
        return f"Deck({len(self.slots)} slots, {self.core_slots} core slots)"


# Each panel config is (label, min_regular, max_greed); loaded from config.yaml.
_FULL_TEST_CONFIGS: List[Tuple[str, int, int]] = [
    (str(c["label"]), int(c["min_regular"]), int(c["max_greed"]))
    for c in _CFG["testing"]["panel_configs"]
]


def _get_test_configs(deck: Deck) -> List[Tuple[str, int, int]]:
    """Return the list of (label, min_regular, max_greed) to run for this deck.

    Lives here because it's purely a derivation of the configured test panel
    plus the deck's own constraints — no algorithm logic, no I/O.
    """
    if not FULL_TEST_PANEL:
        return [("default", deck.min_regular, deck.max_greed)]

    n = len(deck.slots)
    configs: List[Tuple[str, int, int]] = []
    for label, min_reg, max_greed in _FULL_TEST_CONFIGS:
        if min_reg > n:          # collapse: min_reg exceeds capacity
            min_reg   = -1
            max_greed = 0
        configs.append((label, min_reg, max_greed))
    return configs


# ──────────────────────────────────────────────────────────────────────────────
# Deck configurations
# ──────────────────────────────────────────────────────────────────────────────
# Decks are loaded from the `decks/` directory next to this script. Two formats
# are supported (see `decks/README.md` for the full schema):
#
#   * ``*.yaml``  — one deck per file (hand-curated; supports `enabled`,
#                   `min_regular`, `max_greed` overrides).
#   * ``*.json``  — batch import in the Wold's Vaults game-data format
#                   ({"values": {key: {name, layout, socketCount, …}}}).
#
# Layout grid characters (both formats):
#     O → regular slot (placeable)
#     A → arcane slot  (counted toward n_arcane; never placed on)
#     X → empty / wall (any other character is also treated as empty)


def _parse_layout(layout: str) -> Tuple[Set[Tuple[int, int]], int]:
    """Convert a layout grid into ``(regular slot positions, arcane count)``.

    Arcane slot positions are counted but not returned — the optimizer only
    needs the total count for PURE-core scaling, not the geometry.
    """
    slots: Set[Tuple[int, int]] = set()
    n_arcane = 0
    for r, line in enumerate(layout.rstrip("\n").splitlines()):
        for c, ch in enumerate(line):
            if   ch == "O": slots.add((r, c))
            elif ch == "A": n_arcane += 1
    return slots, n_arcane


def _yaml_key(path: Path) -> str:
    """Dedup key for a YAML deck file: stem with any leading ``NN_`` stripped.

    e.g. ``01_cake.yaml`` → ``cake``; matches the JSON ``values.<key>`` field
    so a YAML can override the same-keyed JSON entry.
    """
    return re.sub(r"^\d+_", "", path.stem)


def _load_yaml_decks(seen_keys: Set[str]) -> List[Deck]:
    decks: List[Deck] = []
    for path in sorted(_DECKS_DIR.glob("*.yaml")):
        key = _yaml_key(path)
        if key in EXCLUDED_DECKS:
            continue

        with path.open("r", encoding="utf-8") as fh:
            data = yaml.safe_load(fh) or {}

        if not data.get("enabled", True):
            continue

        slots, n_arcane = _parse_layout(data["layout"])
        if not slots:
            raise ValueError(
                f"Deck file {path.name} has no slots — check the layout grid."
            )

        decks.append(Deck(
            slots       = slots,
            core_slots  = int(data["core_slots"]) + DECKMOD,
            n_arcane    = n_arcane,
            min_regular = int(data.get("min_regular", -1)),
            max_greed   = int(data.get("max_greed", -1)),
            name        = str(data["name"]),
        ))
        seen_keys.add(key)
    return decks


def _load_json_decks(seen_keys: Set[str]) -> List[Deck]:
    """Batch import from Wold's Vaults game-data JSON files.

    Schema: ``{"values": {<key>: {"name", "layout": [{"value": [<rows>]}],
    "socketCount": {"max"}}}}``. Entries with a null ``socketCount`` (dungeon-
    only variants) are skipped. JSON entries whose ``<key>`` matches a YAML
    filename (stripped of any ``NN_`` prefix) are skipped — the YAML wins.
    """
    decks: List[Deck] = []
    for path in sorted(_DECKS_DIR.glob("*.json")):
        with path.open("r", encoding="utf-8") as fh:
            data = json.load(fh) or {}

        for key, entry in (data.get("values") or {}).items():
            if key in EXCLUDED_DECKS:
                continue
            if key in seen_keys:                  # YAML override
                continue
            sockets     = entry.get("socketCount") or {}
            max_sockets = sockets.get("max")
            if max_sockets is None:               # dungeon-only variant
                continue
            layouts = entry.get("layout") or []
            if not layouts:
                continue
            layout_str = "\n".join(layouts[0]["value"])
            slots, n_arcane = _parse_layout(layout_str)
            if not slots:
                continue
            decks.append(Deck(
                slots       = slots,
                core_slots  = int(max_sockets) + DECKMOD,
                n_arcane    = n_arcane,
                min_regular = -1,
                max_greed   = -1,
                name        = str(entry.get("name") or key),
            ))
            seen_keys.add(key)
    return decks


def _load_decks() -> List[Deck]:
    if not _DECKS_DIR.is_dir():
        raise FileNotFoundError(
            f"Deck directory not found: {_DECKS_DIR}. "
            "Add YAML/JSON deck files there or restore the directory."
        )

    seen_keys: Set[str] = set()
    decks: List[Deck] = []
    decks.extend(_load_yaml_decks(seen_keys))
    decks.extend(_load_json_decks(seen_keys))

    if not decks:
        raise RuntimeError(
            f"No enabled deck files found in {_DECKS_DIR}."
        )
    return decks


DECKS: List[Deck] = _load_decks()


# ──────────────────────────────────────────────────────────────────────────────
# Runtime mode switching (GUI only)
# ──────────────────────────────────────────────────────────────────────────────
#
# The CLI sets MODE once at import. The GUI lets the user flip between wolds
# and vanilla without restarting; ``set_mode`` re-reads ``config.yaml``,
# re-applies the merge, and rebinds every UPPERCASE module constant so the
# inventory optimizer sees the new values. The legacy batch optimizer is not
# affected — it reads constants once at module load.
#
# IMPORTANT: callers that import these names directly (``from .config import
# ADDITIVE_CORES``) get a stale value until reimported. Reference them through
# the module (``config.ADDITIVE_CORES``) for live updates.

def set_mode(name: str) -> None:
    """Re-load ``config.yaml`` with the given mode's overrides and rebind
    every mode-dependent module constant in place.

    Also reloads ``DECKS`` because ``deckmod`` (different across modes) affects
    each deck's ``core_slots`` at load time.
    """
    if name not in ("wolds", "vanilla"):
        raise ValueError(f"unknown mode: {name!r}")

    global MODE, _CFG
    global DECKMOD, EXCLUDED_DECKS
    global MULT_DIR_GREED_VERT, MULT_DIR_GREED_HORIZ, MULT_EVO_GREED, MULT_SURR_GREED
    global MULT_DIR_GREED_DIAG_UP, MULT_DIR_GREED_DIAG_DOWN
    global MULT_PURE_BASE, MULT_PURE_SCALE, MULT_EQUILIBRIUM, MULT_FOIL, MULT_STEADFAST, MULT_COLOR
    global ALLOW_DELUXE, MULT_DELUXE_FLAT, MULT_DELUXE_CORE_BASE, MULT_DELUXE_CORE_SCALE
    global GREED_ADDITIVE, ADDITIVE_CORES, SHINY_POSITIONAL
    global ENABLE_EXPERIMENTAL_EXPONENT, EXPERIMENTAL_EXPONENT, EXPERIMENTAL_BOOST
    global DELUXE_COUNTED_AS_REGULAR
    global BALANCE_DISPLAY, HEATMAP_DISPLAY, HNS_Q, SPREAD_METRIC
    global EXPORT_SPREADSHEET, SPREADSHEET_PREFIX, FULL_TEST_PANEL
    global DECKS

    # Re-read + re-merge
    with _CONFIG_PATH.open("r", encoding="utf-8") as _f:
        new_cfg = yaml.safe_load(_f)
    _deep_merge(new_cfg, (new_cfg.get("modes") or {}).get(name, {}) or {})
    _CFG = new_cfg
    MODE = name

    DECKMOD                  = _CFG["deckmod"]
    EXCLUDED_DECKS           = frozenset(_CFG.get("excluded_decks") or ())
    MULT_DIR_GREED_VERT      = _CFG["greed"]["dir_vert"]
    MULT_DIR_GREED_HORIZ     = _CFG["greed"]["dir_horiz"]
    MULT_EVO_GREED           = _CFG["greed"]["evo"]
    MULT_SURR_GREED          = _CFG["greed"]["surr"]
    MULT_DIR_GREED_DIAG_UP   = _CFG["greed"]["dir_diag_up"]
    MULT_DIR_GREED_DIAG_DOWN = _CFG["greed"]["dir_diag_down"]
    MULT_PURE_BASE           = _CFG["cores"]["pure_base"]
    MULT_PURE_SCALE          = _CFG["cores"]["pure_scale"]
    MULT_EQUILIBRIUM         = _CFG["cores"]["equilibrium"]
    MULT_FOIL                = _CFG["cores"]["foil"]
    MULT_STEADFAST           = _CFG["cores"]["steadfast"]
    MULT_COLOR               = _CFG["cores"]["color"]
    ALLOW_DELUXE             = _CFG["deluxe"]["allow"]
    MULT_DELUXE_FLAT         = _CFG["deluxe"]["flat"]
    MULT_DELUXE_CORE_BASE    = _CFG["deluxe"]["core_base"]
    MULT_DELUXE_CORE_SCALE   = _CFG["deluxe"]["core_scale"]
    GREED_ADDITIVE           = _CFG["stacking"]["greed_additive"]
    ADDITIVE_CORES           = _CFG["stacking"]["additive_cores"]
    SHINY_POSITIONAL         = _CFG["shiny"]["positional"]
    ENABLE_EXPERIMENTAL_EXPONENT = _CFG["experimental"]["enable_exponent"]
    EXPERIMENTAL_EXPONENT    = _CFG["experimental"]["exponent"]
    EXPERIMENTAL_BOOST       = _CFG["experimental"]["boost"]
    DELUXE_COUNTED_AS_REGULAR = _CFG["constraints"]["deluxe_counted_as_regular"]
    BALANCE_DISPLAY          = _CFG["display"]["balance"]
    HEATMAP_DISPLAY          = _CFG["display"]["heatmap"]
    HNS_Q                    = _CFG["metrics"]["hns_q"]
    SPREAD_METRIC            = _CFG["metrics"]["spread"]
    EXPORT_SPREADSHEET       = _CFG["output"]["export_spreadsheet"]
    SPREADSHEET_PREFIX       = _CFG["output"]["spreadsheet_prefix"]
    FULL_TEST_PANEL          = _CFG["testing"]["full_panel"]

    # PLACEABLE is module-level state in types.py and was mutated at import time
    # based on ALLOW_DELUXE. Re-sync it now so it matches the new mode.
    from .types import PLACEABLE, CardType as _CT
    while _CT.DELUXE in PLACEABLE:
        PLACEABLE.remove(_CT.DELUXE)
    if ALLOW_DELUXE:
        PLACEABLE.append(_CT.DELUXE)

    # Reload decks — DECKMOD affects each Deck's core_slots at load.
    DECKS = _load_decks()
