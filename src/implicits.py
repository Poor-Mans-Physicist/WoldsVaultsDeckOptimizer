"""Deck-implicit definitions (Optimizer 2.0).

Loads ``decks/wolds_implicits.json`` — the per-deck implicit modifiers
extracted from the woldsvaults addon datagen — and converts entries into the
tuple form the Rust tag-aware kernel expects.

Implicits are Wold's-only: vanilla decks never carry one, and the loader
returns an empty mapping when the JSON file is absent.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Dict, List, Tuple

_IMPLICITS_FILE = Path(__file__).resolve().parent.parent / "decks" / "wolds_implicits.json"

# Kernel tuple: (kind, value, groups, colors, extra) — see tagsim_py.rs.
ImplicitTuple = Tuple[str, float, List[str], List[str], str]


def _load() -> Dict[str, dict]:
    if not _IMPLICITS_FILE.is_file():
        return {}
    with _IMPLICITS_FILE.open("r", encoding="utf-8") as fh:
        data = json.load(fh) or {}
    return data.get("implicits") or {}


#: deck key → raw implicit entry (kind, value, groups/colors, name, desc).
DECK_IMPLICITS: Dict[str, dict] = _load()

#: The 9 freeform category tags an implicit can read (spec §2.2 Bucket B).
CATEGORY_GROUPS: List[str] = [
    "Offensive", "Defensive", "Physical", "Magical", "Utility",
    "Resource", "Knack", "Temporal", "Essence",
]


def to_kernel_tuple(entry: dict) -> ImplicitTuple:
    """Convert one JSON implicit entry into the kernel's tuple form."""
    kind = str(entry.get("kind", "gameplay"))
    value = float(entry.get("value", 0.0))
    groups = [str(g) for g in (entry.get("groups") or [])]
    if "group" in entry:                       # adjacency single-group form
        groups = [str(entry["group"])]
    colors = [str(c) for c in (entry.get("colors") or [])]
    extra = str(entry.get("ptype") or entry.get("range") or "")
    return (kind, value, groups, colors, extra)


def implicits_for_deck(key: str) -> List[ImplicitTuple]:
    """Kernel implicits for one deck key. Gameplay-only / mystery entries
    yield nothing here (mystery pairs are a UI concern; the spreadsheet
    treats mystery as implicit-less)."""
    entry = DECK_IMPLICITS.get(key)
    if entry is None:
        return []
    tup = to_kernel_tuple(entry)
    if tup[0] in ("gameplay", "mystery"):
        return []
    return [tup]


def blanket_groups_for(implicits: List[ImplicitTuple], is_shiny: bool) -> List[str]:
    """The favorable free tags Max assigns (spec §3.1): exactly the category
    groups an active implicit rewards. Stat/Foil are run-derived (handled in
    the kernel), never blanket-assigned here."""
    wanted: List[str] = []
    for (_kind, _value, groups, _colors, _extra) in implicits:
        for g in groups:
            if g in CATEGORY_GROUPS and g not in wanted:
                wanted.append(g)
    del is_shiny  # reserved: shiny-only tag rules land here if ever needed
    return wanted
