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

_REPO_ROOT = Path(__file__).resolve().parent.parent
_IMPLICITS_FILE = _REPO_ROOT / "decks" / "wolds_implicits.json"


def _modifiers_file() -> Path:
    """The active mode's game-card dump (``cards.json_file`` in config.yaml —
    wolds and vanilla ship different rosters). Falls back to the wolds file
    when config can't be imported (e.g. this module used standalone)."""
    try:
        from .config import CARDS_JSON_FILE
        return _REPO_ROOT / CARDS_JSON_FILE
    except Exception:
        return _REPO_ROOT / "modifiers.json"

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


#: Non-stat categories: cards carrying these give no player stats → 0 NDM
#: themselves (kernel NONSTAT_GROUPS). Never blanket-assigned.
NONSTAT_GROUPS: List[str] = ["Resource", "Temporal"]


def _relevant_groups(implicits: List[ImplicitTuple]) -> List[str]:
    """Category tags the active implicits read. ``unique_groups`` (mutant)
    rewards diversity, so every category is relevant to it."""
    out: List[str] = []
    for (kind, _value, groups, _colors, _extra) in implicits:
        if kind == "unique_groups":
            return list(CATEGORY_GROUPS)
        for g in groups:
            if g in CATEGORY_GROUPS and g not in out:
                out.append(g)
    return out


def _load_legal_combos() -> List[List[str]]:
    """Distinct category-tag sets on REAL cards (gear + task_loot entries in
    the mode's card dump). Empty when the file is missing (→ unconstrained)."""
    path = _modifiers_file()
    if not path.is_file():
        import sys as _sys
        print(f"[implicits] WARN: {path.name} not found — tag-combo "
              "legality is unconstrained this run.", file=_sys.stderr)
        return []
    with path.open("r", encoding="utf-8") as fh:
        data = json.load(fh) or {}
    combos = set()
    for entry in (data.get("values") or {}).values():
        if not isinstance(entry, dict) or entry.get("type") not in ("gear", "task_loot"):
            continue
        groups = entry.get("groups") or []
        combos.add(tuple(sorted(g for g in groups if g in CATEGORY_GROUPS)))
    return sorted(list(c) for c in combos)


#: Category combos that exist on real cards; a card's category set must be a
#: subset of one (Wild excepted). Fed to the kernel's toggle-move legality.
LEGAL_COMBOS: List[List[str]] = _load_legal_combos()


def _is_legal_set(tags: List[str]) -> bool:
    cats = [g for g in tags if g in CATEGORY_GROUPS]
    if not cats or not LEGAL_COMBOS:
        return True
    return any(all(g in combo for g in cats) for combo in LEGAL_COMBOS)


def split_blanket_assignable(
    implicits: List[ImplicitTuple],
) -> Tuple[List[str], List[str]]:
    """(blanket, assignable) for Max: stat-safe relevant tags are assigned
    free to every card (NDM-inert); non-stat tags (Resource/Temporal) zero
    the carrying card, so the SA decides per slot ("battery" cards).

    A blanket union that isn't buildable as ONE real card (Mystery pairs,
    mutant's all-categories diversity) demotes wholesale to assignable —
    the SA then picks legal per-card subsets under the kernel combo check."""
    blanket: List[str] = []
    assignable: List[str] = []
    for g in _relevant_groups(implicits):
        (assignable if g in NONSTAT_GROUPS else blanket).append(g)
    if not _is_legal_set(blanket):
        assignable.extend(blanket)
        blanket = []
    return blanket, assignable


def blanket_groups_for(implicits: List[ImplicitTuple], is_shiny: bool) -> List[str]:
    """The favorable free tags Max assigns (stat-safe only)."""
    del is_shiny  # reserved: shiny-only tag rules land here if ever needed
    return split_blanket_assignable(implicits)[0]


def preferred_mono_color(implicits: List[ImplicitTuple]) -> str:
    """The color a color-keyed implicit wants ('' when none). Max's mono
    supply uses it so the readout matches the build guidance — scoring is
    color-blind either way."""
    for (_kind, _value, _groups, colors, _extra) in implicits:
        if colors:
            return colors[0]
    return ""
