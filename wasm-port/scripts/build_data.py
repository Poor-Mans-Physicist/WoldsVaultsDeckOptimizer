"""Build-time data emitter for the static web app.

Reads ``config.yaml`` + ``decks/*.{yaml,json}`` + ``modifiers.json`` from the
repo root and produces three JSON blobs under ``web/public/``:

  * ``config.json``    — every mode fully resolved (defaults + mode overrides
                         deep-merged); the browser picks one at runtime.
  * ``decks.json``     — per-mode lists of decks with slot positions, base
                         core slots (pre-deckmod), arcane slot positions, and
                         constraints. ``core_slots`` is derived in the browser
                         as ``base_core_slots + deckmod``. Shape:
                         ``{"wolds": [Deck, ...], "vanilla": [Deck, ...]}``.
  * ``modifiers.json`` — verbatim copy of the game data file used by the
                         Preview panel. Browser does its own filter pass.

Per-mode pipeline: each mode's ``decks.json_file`` config picks ONE JSON file
to load for that mode (e.g. wolds → ``wolds_decks.json``, vanilla →
``vh_decks.json``). YAML decks are mode-agnostic and merge into every mode's
roster unless excluded via ``excluded_decks``.

This script is deliberately self-contained — no imports from ``src/`` — so
it does not trigger ``src.config``'s sys.argv parsing or single-mode load
side effects. Run via ``uv run python scripts/build_data.py``.
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
from copy import deepcopy
from pathlib import Path
from typing import Any, Dict, List, Set, Tuple

import yaml

# Windows consoles default to cp1252 — force UTF-8 so the summary line prints.
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")


# After the wasm-port dedup refactor, config + decks + modifiers live at the
# OUTER repo root (single source of truth for both CLI and web app).
# Script path: <repo>/wasm-port/scripts/build_data.py  →  three .parent hops.
_REPO_ROOT   = Path(__file__).resolve().parent.parent.parent
_WASM_ROOT   = _REPO_ROOT / "wasm-port"
_CONFIG_PATH = _REPO_ROOT / "config.yaml"
_DECKS_DIR   = _REPO_ROOT / "decks"
_MODIFIERS   = _REPO_ROOT / "modifiers.json"
_IMPLICITS   = _DECKS_DIR / "wolds_implicits.json"
_DEFAULT_OUT = _WASM_ROOT / "web" / "public"


# ── config.yaml resolution ────────────────────────────────────────────────────

def _deep_merge(base: Dict[str, Any], override: Dict[str, Any]) -> Dict[str, Any]:
    """Recursively merge ``override`` into ``base`` (mutating ``base``)."""
    for key, value in override.items():
        if isinstance(value, dict) and isinstance(base.get(key), dict):
            _deep_merge(base[key], value)
        else:
            base[key] = value
    return base


def _resolve_modes(raw: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    """Return ``{mode_name: fully_resolved_config}`` for every mode declared."""
    modes_block = raw.get("modes") or {}
    resolved: Dict[str, Dict[str, Any]] = {}
    for name, overrides in modes_block.items():
        cfg = deepcopy(raw)
        cfg.pop("modes", None)                # don't recurse into mode defs
        _deep_merge(cfg, overrides or {})
        resolved[name] = cfg
    return resolved


# ── deck loading (mirrors src.config logic but standalone) ────────────────────

def _parse_layout(layout: str) -> Tuple[List[List[int]], List[List[int]]]:
    """``layout`` grid → ``(all_slot_positions, arcane_slot_positions)``.

    Both are lists of ``[row, col]`` pairs (JSON-friendly). ``all_slot_positions``
    is the FULL slot set (regular ∪ arcane); arcane positions also appear in
    the second list so the web app can render them with a purple border and
    the SA kernel can enforce the ARCANE/DEAD-only rule on them.
    """
    slots: List[List[int]] = []
    arcane: List[List[int]] = []
    for r, line in enumerate(layout.rstrip("\n").splitlines()):
        for c, ch in enumerate(line):
            if ch == "O":
                slots.append([r, c])
            elif ch == "A":
                slots.append([r, c])
                arcane.append([r, c])
    return slots, arcane


def _yaml_key(path: Path) -> str:
    """Dedup key: filename stem with any ``NN_`` numeric prefix stripped."""
    return re.sub(r"^\d+_", "", path.stem)


def _load_yaml_decks(excluded: Set[str], seen_keys: Set[str]) -> List[Dict[str, Any]]:
    """Load mode-agnostic YAML decks. Mutates ``seen_keys`` so JSON loaders
    (which run per-mode) can skip already-claimed keys."""
    decks: List[Dict[str, Any]] = []
    for path in sorted(_DECKS_DIR.glob("*.yaml")):
        key = _yaml_key(path)
        if key in excluded:
            continue
        with path.open("r", encoding="utf-8") as fh:
            data = yaml.safe_load(fh) or {}
        if not data.get("enabled", True):
            continue
        slots, arcane = _parse_layout(data["layout"])
        if not slots:
            raise ValueError(f"Deck {path.name} has no slots — check the layout grid.")
        decks.append({
            "key":             key,
            "name":            str(data["name"]),
            "slots":           slots,
            "arcane_slots":    arcane,
            "base_core_slots": int(data["core_slots"]),
            "min_regular":     int(data.get("min_regular", -1)),
            "max_greed":       int(data.get("max_greed",   -1)),
        })
        seen_keys.add(key)
    return decks


def _load_named_json_file(
    filename: str,
    excluded: Set[str],
    seen_keys: Set[str],
) -> List[Dict[str, Any]]:
    """Load exactly one named JSON file from the decks directory.

    Empty filename → no JSON loading (returns []). Missing file → warns to
    stderr and returns [] so other modes / the YAML pass still proceed.
    """
    if not filename:
        return []
    path = _DECKS_DIR / filename
    if not path.is_file():
        print(
            f"[build_data] WARN: configured deck file {path} not found — "
            f"this mode will load only YAML decks.",
            file=sys.stderr,
        )
        return []
    decks: List[Dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as fh:
        data = json.load(fh) or {}
    for key, entry in (data.get("values") or {}).items():
        if key in excluded or key in seen_keys:
            continue
        sockets     = entry.get("socketCount") or {}
        max_sockets = sockets.get("max")
        if max_sockets is None:                # dungeon-only variant
            continue
        layouts = entry.get("layout") or []
        if not layouts:
            continue
        layout_str = "\n".join(layouts[0]["value"])
        slots, arcane = _parse_layout(layout_str)
        if not slots:
            continue
        decks.append({
            "key":             key,
            "name":            str(entry.get("name") or key),
            "slots":           slots,
            "arcane_slots":    arcane,
            "base_core_slots": int(max_sockets),
            "min_regular":     -1,
            "max_greed":       -1,
        })
    return decks


def _load_decks_for_mode(mode_cfg: Dict[str, Any]) -> List[Dict[str, Any]]:
    """Build the deck list for one mode: YAML (mode-agnostic) + the mode's
    chosen JSON file (per ``cfg.decks.json_file``)."""
    excluded: Set[str] = set(mode_cfg.get("excluded_decks") or ())
    seen_keys: Set[str] = set()
    decks = _load_yaml_decks(excluded, seen_keys)
    json_filename = (mode_cfg.get("decks") or {}).get("json_file") or ""
    decks += _load_named_json_file(json_filename, excluded, seen_keys)
    return decks


# ── main ──────────────────────────────────────────────────────────────────────

def main() -> None:
    ap = argparse.ArgumentParser(description="Emit static JSON for the web app.")
    ap.add_argument("--out", type=Path, default=_DEFAULT_OUT,
                    help=f"Output directory (default: {_DEFAULT_OUT})")
    args = ap.parse_args()
    out_dir: Path = args.out
    out_dir.mkdir(parents=True, exist_ok=True)

    if not _DECKS_DIR.is_dir():
        raise FileNotFoundError(f"Deck directory not found: {_DECKS_DIR}")

    with _CONFIG_PATH.open("r", encoding="utf-8") as f:
        raw_cfg = yaml.safe_load(f)

    modes = _resolve_modes(raw_cfg)
    if "wolds" not in modes:
        raise RuntimeError("config.yaml must declare a 'wolds' mode (the default).")

    # Per-mode deck rosters. wolds and vanilla get different JSON files via
    # cfg.decks.json_file, so the same key can have different stats per mode.
    decks_by_mode: Dict[str, List[Dict[str, Any]]] = {}
    for mode_name, mode_cfg in modes.items():
        decks_by_mode[mode_name] = _load_decks_for_mode(mode_cfg)
    if not any(decks_by_mode.values()):
        raise RuntimeError(f"No enabled decks found in any mode under {_DECKS_DIR}")

    # config.json: emit every mode resolved, plus a default-mode marker.
    config_blob: Dict[str, Any] = {
        "default_mode": "wolds",
        "modes":        modes,
    }
    (out_dir / "config.json").write_text(
        json.dumps(config_blob, indent=2, sort_keys=False) + "\n",
        encoding="utf-8",
    )

    # Deck implicits (Optimizer 2.0, Wold's-only). Attach each deck's implicit
    # entry to its RawDeck (wolds roster only) AND ship the full catalog so
    # the Mystery deck's pair-picker can enumerate every implicit.
    implicit_catalog: Dict[str, Any] = {}
    if _IMPLICITS.is_file():
        with _IMPLICITS.open("r", encoding="utf-8") as fh:
            implicit_catalog = (json.load(fh) or {}).get("implicits") or {}
        for deck in decks_by_mode.get("wolds", []):
            imp = implicit_catalog.get(deck["key"])
            if imp is not None:
                deck["implicit"] = imp
    else:
        print(f"[build_data] WARN: {_IMPLICITS} not found — decks ship without implicits.")

    # Legal category-tag combos: the distinct category sets that exist on
    # REAL cards (gear stat cards + task_loot resource cards). No optimizer
    # surface may invent a combination outside this catalog (subset rule;
    # Wild excepted). Shipped so the SA's tag toggles, the Exact builder,
    # and the what-if popup all enforce the same reality.
    categories = ["Offensive", "Defensive", "Physical", "Magical", "Utility",
                  "Resource", "Knack", "Temporal", "Essence"]
    combo_set = set()
    if _MODIFIERS.exists():
        with _MODIFIERS.open("r", encoding="utf-8") as fh:
            mods = json.load(fh) or {}
        for entry in (mods.get("values") or {}).values():
            if not isinstance(entry, dict):
                continue
            if entry.get("type") not in ("gear", "task_loot"):
                continue
            groups = entry.get("groups") or []
            combo = tuple(sorted(g for g in groups if g in categories))
            combo_set.add(combo)
    tag_combos = sorted(list(c) for c in combo_set)

    # decks.json: per-mode keyed dict + reserved "_"-prefixed catalogs
    # (mode names never start with an underscore).
    decks_blob: Dict[str, Any] = dict(decks_by_mode)
    decks_blob["_implicits"] = implicit_catalog
    decks_blob["_tagCombos"] = tag_combos
    (out_dir / "decks.json").write_text(
        json.dumps(decks_blob, indent=2) + "\n",
        encoding="utf-8",
    )

    # modifiers.json: verbatim copy. Browser filters / classifies.
    if _MODIFIERS.exists():
        shutil.copyfile(_MODIFIERS, out_dir / "modifiers.json")
    else:
        print(f"[build_data] WARN: {_MODIFIERS} not found — Preview will be empty.")

    deck_summary = ", ".join(f"{m}={len(ds)}" for m, ds in decks_by_mode.items())
    print(f"[build_data] wrote {len(modes)} mode(s), decks: [{deck_summary}] → {out_dir}")


if __name__ == "__main__":
    main()
