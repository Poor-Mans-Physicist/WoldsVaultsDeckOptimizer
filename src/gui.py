"""NiceGUI front-end for the inventory-based optimizer.

Launch with ``uv run --extra rust optimize-gui`` (or ``uv run optimize-gui`` to
force the pure-Python path). Opens a local browser tab; the optimizer is called
in a worker thread so the UI stays responsive.

Layout: deck grid on the left, controls (deck + class pickers, core toggles
with override inputs, type × color inventory table, Run button) on the right.
Total NDM and chosen cores show above the grid after each run.
"""
from __future__ import annotations

import asyncio
import sys
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Set, Tuple

from nicegui import app, run, ui

from . import config as _cfg  # read DECKS / MODE via attribute for live updates
from . import preview as _preview
from .config import Deck
from .modifiers import get_card
from .types import CardClass, CardType, Color, CoreType
from .inventory_optimize import (
    CardInventory,
    CoreInventory,
    CoreSpec,
    InventoryResult,
    SlotBreakdown,
    _RUST_OK,
    optimize_inventory,
)


# ──────────────────────────────────────────────────────────────────────────────
# Palettes (kept in lockstep with src/report.py's xlsx output)
# ──────────────────────────────────────────────────────────────────────────────

_POSITIONAL = {CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG}
_DIR_GREEDS = {
    CardType.DIR_GREED_UP,    CardType.DIR_GREED_DOWN,
    CardType.DIR_GREED_LEFT,  CardType.DIR_GREED_RIGHT,
    CardType.DIR_GREED_NE,    CardType.DIR_GREED_NW,
    CardType.DIR_GREED_SE,    CardType.DIR_GREED_SW,
}
_OTHER_GREEDS = {CardType.EVO_GREED, CardType.SURR_GREED}


def _slot_bg(t: CardType) -> str:
    """Background color for a slot tile, matching the xlsx output."""
    if t in _POSITIONAL:        return "#A9CCE3"  # light blue
    if t == CardType.DELUXE:    return "#D7BDE2"  # purple
    if t in _DIR_GREEDS:        return "#F9E79F"  # yellow
    if t in _OTHER_GREEDS:      return "#FDEBD0"  # peach
    if t == CardType.TYPELESS:  return "#A8D5A2"  # soft green
    if t == CardType.DEAD:      return "#ECECEC"  # neutral gray
    if t == CardType.ARCANE:    return "#E5DEFF"  # light purple
    return "#FFFFFF"


# Purple border color used on every arcane slot, regardless of contents (so the
# user can immediately distinguish arcane positions from regular ones).
_ARCANE_BORDER = "#A78BFA"


_GAME_COLOR_HEX: Dict[Color, str] = {
    Color.RED:    "#E74C3C",
    Color.GREEN:  "#27AE60",
    Color.BLUE:   "#3498DB",
    Color.YELLOW: "#F1C40F",
}


_INVENTORY_TYPES: List[CardType] = [
    CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
    CardType.ARCANE,
    CardType.DELUXE, CardType.TYPELESS,
    CardType.DIR_GREED_UP, CardType.DIR_GREED_DOWN,
    CardType.DIR_GREED_LEFT, CardType.DIR_GREED_RIGHT,
    CardType.DIR_GREED_NE, CardType.DIR_GREED_NW,
    CardType.DIR_GREED_SE, CardType.DIR_GREED_SW,
    CardType.EVO_GREED, CardType.SURR_GREED,
]
_COLORS: List[Color] = [Color.RED, Color.GREEN, Color.BLUE, Color.YELLOW]


_TYPE_LABEL: Dict[CardType, str] = {
    CardType.ROW: "Row", CardType.COL: "Col", CardType.SURR: "Surr", CardType.DIAG: "Diag",
    CardType.ARCANE: "Arcane",
    CardType.DELUXE: "Deluxe", CardType.TYPELESS: "Typeless",
    CardType.DIR_GREED_UP: "Greed ↑", CardType.DIR_GREED_DOWN: "Greed ↓",
    CardType.DIR_GREED_LEFT: "Greed ←", CardType.DIR_GREED_RIGHT: "Greed →",
    CardType.DIR_GREED_NE: "Greed ↗", CardType.DIR_GREED_NW: "Greed ↖",
    CardType.DIR_GREED_SE: "Greed ↘", CardType.DIR_GREED_SW: "Greed ↙",
    CardType.EVO_GREED: "Evo Greed", CardType.SURR_GREED: "Surr Greed",
}
_TYPE_GLYPH: Dict[CardType, str] = dict(Deck._CHAR)


_CORE_OPTIONS: List[Tuple[CoreType, Optional[Color]]] = [
    (CoreType.PURE,        None),
    (CoreType.EQUILIBRIUM, None),
    (CoreType.STEADFAST,   None),
    (CoreType.FOIL,        None),
    (CoreType.DELUXE_CORE, None),
    (CoreType.VOID_CORE,   None),
    (CoreType.PLUTO_CORE,  None),
    (CoreType.COLOR,       Color.RED),
    (CoreType.COLOR,       Color.GREEN),
    (CoreType.COLOR,       Color.BLUE),
    (CoreType.COLOR,       Color.YELLOW),
]


def _core_label(ct: CoreType, color: Optional[Color]) -> str:
    if ct == CoreType.COLOR and color is not None:
        return f"Color · {color.value.title()}"
    return ct.value.replace("_", " ").title()


# ──────────────────────────────────────────────────────────────────────────────
# UI state
# ──────────────────────────────────────────────────────────────────────────────

@dataclass
class _AppState:
    deck: Deck
    card_class: CardClass = CardClass.SHINY
    mode: str = "wolds"            # "wolds" or "vanilla" — flipped by the GUI toggle
    inventory_counts: Dict[Tuple[CardType, Color], int] = field(default_factory=dict)
    # Forced inventory: cards the optimizer MUST place (minimum count per
    # (type, color)). Cap per (type, color) = inventory + forced.
    forced_counts: Dict[Tuple[CardType, Color], int] = field(default_factory=dict)
    # Which inventory table is visible in the UI ("regular" or "forced").
    # Both grids persist when hidden; preset buttons apply to the active view.
    inventory_view: str = "regular"
    core_state: Dict[int, Tuple[bool, Optional[float]]] = field(default_factory=dict)
    # User-adjustable "Bonus Cores" delta. Seeded from ``_cfg.DECKMOD`` (wolds
    # default 1, vanilla default 0) and re-seeded on every mode flip.
    # Effective deck core slots = max(0, deck.core_slots - DECKMOD + bonus_cores)
    # — see ``_run_optimization``. Negative values silently clamp to 0 cores.
    bonus_cores: int = 0
    last_result: Optional[InventoryResult] = None
    n_iter: int = 60_000
    restarts: int = 12
    # Top-level view: "optimize" (default UI) or "preview" (assign stat cards
    # to the last optimized layout and see player-facing stat totals).
    view: str = "optimize"
    # Slot -> (modifier_key, tier). Survives Run when slot family is unchanged.
    preview_assignments: Dict[Tuple[int, int], Tuple[str, int]] = field(default_factory=dict)
    # Arcane auto-place. True (default from config.yaml): pre-fill arcane
    # slots with ARCANE; SA may shuffle colors but cannot swap to DEAD.
    # False: SA may swap ARCANE ↔ DEAD per arcane slot.
    auto_place_arcane: bool = True


# ──────────────────────────────────────────────────────────────────────────────
# Deck grid rendering
# ──────────────────────────────────────────────────────────────────────────────

_SLOT_PX = 64
_GAP_PX  = 6


def _render_deck_grid(
    container: ui.element,
    state: _AppState,
    *,
    on_preview_change: Optional[callable] = None,
) -> None:
    """Re-render the deck grid for the current state.view.

    Optimize view  → slot click opens the math-breakdown dialog (when result).
    Preview view   → slot click opens the card-assign dialog (when assignable);
                     ``on_preview_change`` is invoked after a successful assign
                     so the caller can refresh the stats panel + the grid.
    """
    container.clear()
    deck = state.deck
    result = state.last_result

    rows = [r for r, _ in deck.slots]
    cols = [c for _, c in deck.slots]
    min_r, max_r = min(rows), max(rows)
    min_c, max_c = min(cols), max(cols)
    w = max_c - min_c + 1
    h = max_r - min_r + 1

    with container:
        with ui.element("div").style(
            f"display:grid;"
            f"grid-template-columns: repeat({w}, {_SLOT_PX}px);"
            f"grid-template-rows: repeat({h}, {_SLOT_PX}px);"
            f"gap: {_GAP_PX}px;"
            f"padding: 12px;"
            f"background: #0F172A;"
            f"border: 1px solid #334155;"
            f"border-radius: 10px;"
        ):
            for r in range(min_r, max_r + 1):
                for c in range(min_c, max_c + 1):
                    pos = (r, c)
                    if pos not in deck.slots:
                        ui.element("div").style(
                            f"width:{_SLOT_PX}px;height:{_SLOT_PX}px;background:transparent;"
                        )
                        continue
                    _render_slot(pos, state, on_preview_change=on_preview_change)


def _render_slot(
    pos: Tuple[int, int],
    state: _AppState,
    *,
    on_preview_change: Optional[callable] = None,
) -> None:
    result = state.last_result
    if result is not None and pos in result.assignment:
        t, color = result.assignment[pos]
    else:
        t, color = CardType.EMPTY, None

    bg     = _slot_bg(t) if t != CardType.EMPTY else "#374151"
    glyph  = _TYPE_GLYPH.get(t, "·") if t != CardType.EMPTY else "□"
    ndm    = result.per_slot_ndm.get(pos) if result is not None else None
    breakdown = (
        result.per_slot_breakdown.get(pos)
        if result is not None and result.per_slot_breakdown
        else None
    )

    # Preview state for this slot
    in_preview = (state.view == "preview")
    preview_assignable = (
        in_preview and t != CardType.EMPTY
        and _preview.is_assignable_slot(t, state.card_class)
    )
    preview_assignment = state.preview_assignments.get(pos) if in_preview else None
    preview_card = get_card(preview_assignment[0]) if preview_assignment else None

    # Cursor + opacity
    if in_preview:
        if preview_assignable:
            cursor = "pointer"
            opacity = 1.0
        else:
            cursor = "not-allowed"
            opacity = 0.55  # gray out non-assignable slots in preview
    else:
        cursor = "help" if breakdown is not None else "default"
        opacity = 1.0

    # Arcane slots get a thick purple border (and an "a" glyph if empty) so the
    # user can immediately tell them apart from regular slots.
    is_arcane_slot = pos in state.deck.arcane_slots
    border_style = (
        f"border: 2px solid {_ARCANE_BORDER};" if is_arcane_slot
        else "border: 1px solid rgba(0,0,0,.08);"
    )
    if is_arcane_slot and t == CardType.EMPTY:
        # Empty arcane slot before any run — show the arcane glyph faintly.
        glyph = "a"

    with ui.element("div").style(
        f"width:{_SLOT_PX}px;height:{_SLOT_PX}px;"
        f"background:{bg};"
        f"{border_style}"
        f"border-radius: 8px;"
        f"position:relative;"
        f"opacity:{opacity};"
        f"display:flex;flex-direction:column;align-items:center;justify-content:center;"
        f"font-family:'JetBrains Mono','Consolas',monospace;"
        f"cursor:{cursor};"
    ) as slot_div:
        ui.label(glyph).style(
            "font-size:22px; font-weight:600; line-height:1; color:#1F2937;"
        )
        if ndm is not None and ndm > 0:
            ui.label(f"{ndm:.1f}").style("font-size:10px; color:#374151; margin-top:2px;")
        if color is not None:
            ui.element("div").style(
                f"position:absolute;top:4px;right:4px;"
                f"width:8px;height:8px;border-radius:50%;"
                f"background:{_GAME_COLOR_HEX[color]};"
                f"border:1px solid rgba(0,0,0,.2);"
            )

        # Preview assignment badge (tier + attribute abbreviation)
        if preview_card is not None and preview_assignment is not None:
            tier = preview_assignment[1]
            ui.label(f"T{tier}").style(
                "position:absolute;top:3px;left:4px;"
                "font-size:9px;font-weight:700;color:#1F2937;"
                "background:#FDE68A;border-radius:3px;padding:0 3px;line-height:13px;"
            )
            ui.label(_preview.attr_abbrev(preview_card.attribute_short)).style(
                "position:absolute;bottom:3px;left:0;right:0;text-align:center;"
                "font-size:9px;font-weight:600;color:#1F2937;"
            )

        # Click behavior depends on view
        if in_preview:
            if preview_assignable and result is not None:
                slot_ndm = ndm or 0.0
                slot_class = state.card_class
                slot_type_t = t
                slot_div.on(
                    "click",
                    lambda _e=None, p=pos, st=slot_type_t, cls=slot_class,
                            n=slot_ndm: _preview.open_assign_dialog(
                        p, st, cls, n, state,
                        on_done=(on_preview_change or (lambda: None)),
                    ),
                )
            # non-assignable preview slots: no click handler (cursor: not-allowed)
        else:
            # Optimize view: open breakdown dialog if available
            if breakdown is not None:
                # Click-to-open dialog (NOT a hover tooltip) because Quasar tooltips
                # dismiss on mouse-leave and can't be scrolled.
                with ui.dialog() as bd_dialog:
                    with ui.card().style(
                        "background:#1F2937;color:#F9FAFB;"
                        "padding:14px 18px;border-radius:10px;"
                        "min-width:360px;max-width:560px;"
                        "max-height:80vh;overflow-y:auto;"
                        "font-family:'JetBrains Mono','Consolas',monospace;"
                        "font-size:12px;line-height:1.5;white-space:pre-wrap;"
                    ):
                        ui.label(_format_breakdown(pos, breakdown))
                        ui.button("Close", on_click=bd_dialog.close) \
                            .props("flat dense color=white").classes("mt-2")
                slot_div.on("click", lambda _e=None, d=bd_dialog: d.open())


def _build_legend() -> None:
    """Card-symbol key + brief usage tips, rendered below the deck grid."""

    # Group entries by category so the legend reads cleanly. Each tuple is
    # (glyph, label, background color).
    positional = [
        ("R", "Row",  _slot_bg(CardType.ROW)),
        ("C", "Col",  _slot_bg(CardType.COL)),
        ("S", "Surr", _slot_bg(CardType.SURR)),
        ("X", "Diag", _slot_bg(CardType.DIAG)),
    ]
    other = [
        ("D", "Deluxe",   _slot_bg(CardType.DELUXE)),
        ("T", "Typeless", _slot_bg(CardType.TYPELESS)),
        ("·", "Dead",     _slot_bg(CardType.DEAD)),
    ]
    dir_greeds = [
        ("↑", "Greed Up",    _slot_bg(CardType.DIR_GREED_UP)),
        ("↓", "Greed Down",  _slot_bg(CardType.DIR_GREED_DOWN)),
        ("←", "Greed Left",  _slot_bg(CardType.DIR_GREED_LEFT)),
        ("→", "Greed Right", _slot_bg(CardType.DIR_GREED_RIGHT)),
        ("↗", "Greed NE",    _slot_bg(CardType.DIR_GREED_NE)),
        ("↖", "Greed NW",    _slot_bg(CardType.DIR_GREED_NW)),
        ("↘", "Greed SE",    _slot_bg(CardType.DIR_GREED_SE)),
        ("↙", "Greed SW",    _slot_bg(CardType.DIR_GREED_SW)),
    ]
    other_greeds = [
        ("e", "Evo Greed",  _slot_bg(CardType.EVO_GREED)),
        ("o", "Surr Greed", _slot_bg(CardType.SURR_GREED)),
    ]

    with ui.card().tight().classes("w-full mt-3").style("max-width: 720px;"):
        with ui.card_section():
            ui.label("Card Key").classes("text-sm font-semibold uppercase text-gray-500 mb-2")
            with ui.row().classes("items-center gap-2 flex-wrap"):
                for entry in positional + other + dir_greeds + other_greeds:
                    _legend_chip(*entry)

            ui.separator().classes("my-3")

            ui.label("How to use").classes("text-sm font-semibold uppercase text-gray-500 mb-1")
            with ui.column().classes("gap-1"):
                ui.label(
                    "• Pick a deck, class, and mode at the top of the controls panel."
                ).classes("text-xs text-gray-600")
                ui.label(
                    "• Toggle the cores you own (set overrides for non-default values — see Tips)."
                ).classes("text-xs text-gray-600")
                ui.label(
                    "• Fill the inventory table with cards you own. Use Unlimited (100×) / Clear "
                    "for bulk presets, or the per-row / per-column 100× buttons for finer fills. "
                    "Toggle to the Forced view to force-place specific cards (see Tips)."
                ).classes("text-xs text-gray-600")
                ui.label(
                    "• Hit Run. The deck repaints with the optimizer's chosen placement; each tile "
                    "shows the card's symbol and its NDM contribution. Click any tile to see the "
                    "full math (base × cores × boost) for that slot."
                ).classes("text-xs text-gray-600")
                ui.label(
                    "• The badge above the deck reports whether the Rust and Python paths agree on "
                    "the total. Green = agreement, red = mismatch (with both numbers shown)."
                ).classes("text-xs text-gray-600")
                ui.label(
                    "• After a Run, switch the top toggle to Preview to assign concrete stat cards "
                    "to the layout and see player-facing totals (see Tips)."
                ).classes("text-xs text-gray-600")

            ui.separator().classes("my-3")

            ui.label("Tips").classes("text-sm font-semibold uppercase text-gray-500 mb-2")
            with ui.column().classes("gap-3"):
                _tip_section(
                    "Core overrides",
                    [
                        "Each enabled core has an override field on the right. Leave blank to use the "
                        "config default, or type a number to replace it for the next run only.",
                        "• Static cores (Foil, Color, Equilibrium, Steadfast, Pluto): the override is "
                        "the FULL multiplier. A core that boosts a card by +50% multiplies by ×1.5, "
                        "so enter 1.5. A flat 3× core needs 3.0.",
                        "• Scaling cores (Pure, Deluxe Core, Void Core): the override replaces ONLY "
                        "the per-card scale term — the formula stays base + scale × n. A pure core "
                        "that grants +20% per non-shiny card has scale = 0.2, so enter 0.2 (NOT 1.2). "
                        "A void core at +0.3 per dead card needs 0.3 in the box.",
                        "• Quick rule of thumb: if the in-game tooltip describes the core as 'gives "
                        "+X%' (flat), enter 1 + X/100. If it describes 'per card' or 'per stack', "
                        "enter the per-card fraction (X/100) directly.",
                    ],
                )
                _tip_section(
                    "Forced inventory (yellow table)",
                    [
                        "The Inventory header has a Regular / Forced toggle. Cards entered in the "
                        "Forced view MUST be placed by the optimizer — they're a per-(type, color) "
                        "lower bound, analogous to min_regular in the spreadsheet optimizer.",
                        "• Forced and regular add together for the cap. Example: 2 green Row in "
                        "Forced + 3 green Row in Regular → optimizer has 5 to draw from, but at "
                        "least 2 must end up on the board.",
                        "• If the total forced count exceeds the deck's slot count, the run is "
                        "rejected before SA starts with a clear error message.",
                        "• Per-row / per-column 100× buttons appear on the Regular table only. "
                        "Forced cells are entered by hand to keep that bucket intentional.",
                    ],
                )
                _tip_section(
                    "Preview mode",
                    [
                        "Once a Run produces a layout, flip the top View toggle from Optimize to "
                        "Preview. The right column collapses to a player-stats panel; the deck grid "
                        "stays in place and becomes a stat-card picker.",
                        "• Click any scoring slot (positional, deluxe, or typeless) to open a "
                        "searchable picker, filtered by that slot's family (Shiny / Evo / Deluxe / "
                        "Typeless). Pick a tier — its base value × the slot's NDM is the card's "
                        "contribution. Greed and dead slots are inert and grayed out.",
                        "• The sidebar accumulates contributions by attribute, split into two "
                        "buckets: Flat (e.g. +1200 Attack Damage) and Percent (e.g. +50% Damage "
                        "Increase). Multiple slots assigned to the same stat sum together.",
                        "• Re-running the optimizer preserves assignments at the same (row, col) "
                        "IF the slot's class family is unchanged; otherwise they're dropped and "
                        "the UI notifies you how many got cleared.",
                    ],
                )


def _legend_chip(glyph: str, label: str, bg: str) -> None:
    """One compact pill: colored swatch + glyph + label."""
    with ui.element("div").style(
        "display:inline-flex;align-items:center;gap:6px;"
        f"background:{bg};"
        "border:1px solid rgba(0,0,0,.08);border-radius:6px;"
        "padding:3px 8px;font-size:12px;"
    ):
        ui.label(glyph).style(
            "font-family:'JetBrains Mono','Consolas',monospace;"
            "font-weight:600;font-size:13px;min-width:14px;text-align:center;"
            "color:#1F2937;"
        )
        ui.label(label).style("color:#1F2937;")


def _tip_section(title: str, bullets: List[str]) -> None:
    """One tip block: a bold sub-header + a column of bullet lines.

    The first bullet often reads as a topic intro (no leading marker); follow-up
    bullets typically start with the ``•`` glyph already baked into the string.
    """
    with ui.column().classes("gap-1"):
        ui.label(title).style(
            "font-size: 12px; font-weight: 600; color: #F1F5F9;"
        )
        for line in bullets:
            ui.label(line).style(
                "font-size: 11.5px; color: #CBD5E1; line-height: 1.45;"
            )


def _format_breakdown(pos: Tuple[int, int], b: SlotBreakdown) -> str:
    """Human-readable multi-line breakdown for the click-to-open popup.

    Reflects the per-card single-core_mult model: every applicable core folds
    into one additive (or multiplicative) sum; cores that didn't apply to this
    card are listed in a separate section with the reason.
    """
    type_name = _TYPE_LABEL.get(b.card_type, b.card_type.value)
    color_name = b.color.value.title() if b.color is not None else "—"
    head = f"{type_name} · {color_name}  @ ({pos[0]},{pos[1]})"
    sep  = "─" * max(len(head), 24)

    lines: List[str] = [head, sep, ""]

    # Base value
    lines.append("Base value:")
    lines.append(f"  {b.base_explain}")
    lines.append(f"  → {b.base_value:g}")
    if b.final_ndm == 0.0:
        # Non-scoring slot — show only the base explanation and stop.
        lines.append("")
        lines.append("(does not contribute to NDM)")
        return "\n".join(lines)
    lines.append("")

    # Cores that apply to this specific card
    lines.append("Cores applied to this card:")
    if not b.applied_cores:
        lines.append("  (none)")
    for c in b.applied_cores:
        label = c.core_type.value
        if c.color is not None:
            label = f"{label} ({c.color.value})"
        tag = " (override)" if c.override else ""
        lines.append(f"  • {label:<18s} ×{c.value:.3f}{tag}")
    lines.append(f"  formula: {b.core_mult_formula}")
    lines.append(f"  → core_mult = ×{b.core_mult:.3f}")
    lines.append("")

    # Cores in the deck that don't apply to this card
    if b.excluded_cores:
        lines.append("Cores excluded from this card:")
        for x in b.excluded_cores:
            label = x.core_type.value
            if x.color is not None:
                label = f"{label} ({x.color.value})"
            lines.append(f"  • {label} — {x.reason}")
        lines.append("")

    # Greed boost
    lines.append("Boost (greed):")
    if not b.boost_sources:
        lines.append("  (no greed targeting this slot)")
    for src in b.boost_sources:
        lines.append(
            f"  • {src.greed_type.value:<14s} from ({src.from_position[0]},{src.from_position[1]}) "
            f"→ ×{src.multiplier:.3f}"
        )
    lines.append(f"  → boost = ×{b.boost:.3f}")
    lines.append("")

    # Final
    lines.append(
        f"Final: {b.base_value:g} × {b.core_mult:.3f} × {b.boost:.3f}"
    )
    lines.append(f"     = {b.final_ndm:.3f}")
    return "\n".join(lines)


# ──────────────────────────────────────────────────────────────────────────────
# Page assembly
# ──────────────────────────────────────────────────────────────────────────────

def _build_page() -> None:
    state = _AppState(
        deck=_cfg.DECKS[0],
        mode=_cfg.MODE,
        auto_place_arcane=_cfg.AUTO_PLACE_ARCANE,
        bonus_cores=_cfg.DECKMOD,
    )
    for idx in range(len(_CORE_OPTIONS)):
        state.core_state[idx] = (False, None)

    # Dark theme. ``ui.dark_mode().enable()`` flips Quasar's built-in colors for
    # cards / inputs / selects / dialogs etc. The CSS below layers our own bg
    # + card colors on top so the page has consistent slate tones.
    ui.dark_mode().enable()
    ui.add_head_html("""
        <style>
          body { background: #0F172A; color: #F1F5F9; }
          /* Cards sit one shade lighter than the body so they float visually. */
          .q-card { background: #1E293B !important; border: 1px solid #334155 !important; }
          /* Make heading labels with text-gray-800/700/600 readable on dark bg. */
          .text-gray-800 { color: #F1F5F9 !important; }
          .text-gray-600 { color: #CBD5E1 !important; }
          .text-gray-500 { color: #94A3B8 !important; }
        </style>
    """)

    # Forward closure: rebinds at call time. ``grid_container`` and
    # ``preview_panel`` are defined later in this function but the closure
    # captures the names lazily, so this is safe as long as we don't *call*
    # the function before those names exist.
    def _on_preview_change() -> None:
        _render_deck_grid(grid_container, state, on_preview_change=_on_preview_change)
        if state.view == "preview":
            _preview.build_stats_panel(preview_panel, state)

    # Mode-aware visibility for the inventory rows + core options + class label.
    # Same forward-closure pattern: these names get bound later in this function.
    # NOTE: hidden options are also filtered at Run time (see _run_optimization)
    # so that switching modes doesn't destroy the user's entered values.
    def _apply_mode_visibility() -> None:
        hidden_types = _hidden_inventory_types(state.mode, state.card_class)
        hidden_cores = _hidden_core_types(state.mode)
        # Both inventory tables (regular + forced) share the same per-row rules.
        for t, row in inv_row_containers_regular.items():
            row.set_visibility(t not in hidden_types)
        for t, row in inv_row_containers_forced.items():
            row.set_visibility(t not in hidden_types)
        # Core rows: index-aligned with _CORE_OPTIONS.
        for idx, row in enumerate(core_row_containers):
            ct, _color = _CORE_OPTIONS[idx]
            row.set_visibility(ct not in hidden_cores)
        # Relabel the Class picker (Shiny → Stat in vanilla).
        class_select.options = _class_select_options(state.mode)
        class_select.update()

    with ui.row().classes("w-full items-start gap-6 p-6 no-wrap"):
        # ── Left: results + grid ─────────────────────────────────────────────
        with ui.column().classes("gap-3 items-center"):
            ui.label("Vault Hunters Deck Optimizer").classes("text-2xl font-semibold")
            total_label = ui.label("NDM  —").classes("text-3xl font-bold text-gray-800")
            cores_label = ui.label("").classes("text-sm text-gray-500")
            verify_label = ui.label("").style("font-size:12px;")
            grid_container = ui.element("div")
            _render_deck_grid(grid_container, state, on_preview_change=_on_preview_change)
            _build_legend()

        # ── Right: controls ──────────────────────────────────────────────────
        with ui.column().classes("gap-3").style("min-width: 480px;"):
            # ── View toggle (always visible) ─────────────────────────────────
            with ui.card().tight().classes("w-full"):
                with ui.card_section():
                    with ui.row().classes("w-full items-center gap-3"):
                        ui.label("View").classes("text-sm font-semibold uppercase text-gray-500")
                        def _on_view_change(e):
                            state.view = e.value
                            optimize_panel.set_visibility(state.view == "optimize")
                            preview_panel.set_visibility(state.view == "preview")
                            if state.view == "preview":
                                _preview.build_stats_panel(preview_panel, state)
                            # Re-render grid so click handlers match the new view.
                            _render_deck_grid(
                                grid_container, state,
                                on_preview_change=_on_preview_change,
                            )
                        ui.toggle(
                            {"optimize": "Optimize", "preview": "Preview"},
                            value=state.view,
                            on_change=_on_view_change,
                        ).props("dense")
                    ui.label(
                        "Optimize finds the best layout; Preview lets you assign "
                        "stat cards to the layout and see the totals."
                    ).classes("text-xs text-gray-500 mt-1")

            # ── Optimize panel (deck/class, cores, inventory, run) ───────────
            optimize_panel = ui.column().classes("gap-3 w-full")
            with optimize_panel:
                # Deck & class
                with ui.card().tight().classes("w-full"):
                    with ui.card_section():
                        ui.label("Deck & class").classes("text-sm font-semibold uppercase text-gray-500")
                        with ui.row().classes("w-full items-center gap-3"):
                            def _on_deck_change(e):
                                for d in _cfg.DECKS:
                                    if d.name == e.value:
                                        state.deck = d
                                        state.last_result = None
                                        # New deck = different slot set; old preview
                                        # assignments no longer apply.
                                        state.preview_assignments.clear()
                                        total_label.text = "NDM  —"
                                        cores_label.text = ""
                                        verify_label.text = ""
                                        _render_deck_grid(
                                            grid_container, state,
                                            on_preview_change=_on_preview_change,
                                        )
                                        if state.view == "preview":
                                            _preview.build_stats_panel(preview_panel, state)
                                        return
                            # Captured so _on_mode_change can refresh the deck
                            # roster when wolds ↔ vanilla swaps the JSON file.
                            deck_select = ui.select(
                                options=[d.name for d in _cfg.DECKS],
                                value=state.deck.name,
                                label="Deck",
                                on_change=_on_deck_change,
                            ).classes("flex-grow")
                            def _on_class_change(e):
                                # Class flip can change which inventory rows are
                                # hidden (vanilla + stat hides positionals).
                                state.card_class = CardClass(e.value)
                                _apply_mode_visibility()
                            class_select = ui.select(
                                options=_class_select_options(state.mode),
                                value=state.card_class.value,
                                label="Class",
                                on_change=_on_class_change,
                            ).classes("w-28")
                        # Mode toggle (Wolds / Vanilla). Switching modes calls
                        # config.set_mode() which re-merges config.yaml and reloads
                        # DECKS, so a deck's core_slots reflects the new mode's
                        # deckmod immediately.
                        def _on_mode_change(e):
                            try:
                                _cfg.set_mode(e.value)
                            except Exception as exc:  # noqa: BLE001
                                ui.notify(f"Mode change failed: {exc}", color="negative")
                                return
                            state.mode = e.value
                            state.last_result = None
                            state.preview_assignments.clear()
                            # Re-seed Bonus Cores from the new mode's deckmod
                            # (wolds=1, vanilla=0). Drops any in-flight user
                            # override on flip — defaults are mode-meaningful
                            # and a "sticky" override across modes is confusing.
                            state.bonus_cores = _cfg.DECKMOD
                            # Re-fetch our currently-selected deck from the new DECKS list
                            # (each Deck's core_slots may have shifted due to deckmod change).
                            prev_name = state.deck.name
                            match = next((d for d in _cfg.DECKS if d.name == prev_name), None)
                            if match is not None:
                                state.deck = match
                            else:
                                state.deck = _cfg.DECKS[0]
                                ui.notify(
                                    f"Deck '{prev_name}' not available in {e.value} mode — "
                                    f"switched to '{state.deck.name}'.",
                                    color="warning",
                                )
                            total_label.text = "NDM  —"
                            cores_label.text = ""
                            verify_label.text = ""
                            # Refresh the deck dropdown — the mode's JSON file
                            # changed the roster (e.g. vanilla drops the 5
                            # Wold-exclusive decks). The select's `value` is
                            # set before `options` so it survives the rebind.
                            deck_select.options = [d.name for d in _cfg.DECKS]
                            deck_select.value   = state.deck.name
                            deck_select.update()
                            # Push the re-seeded bonus_cores value into its
                            # input widget (state mutation alone doesn't drive
                            # the UI; see the comment in the cores section).
                            bonus_cores_input.set_value(state.bonus_cores)
                            _render_deck_grid(
                                grid_container, state,
                                on_preview_change=_on_preview_change,
                            )
                            if state.view == "preview":
                                _preview.build_stats_panel(preview_panel, state)
                            # Hide / show options that aren't valid in the new
                            # mode (also relabels Shiny → Stat in vanilla).
                            _apply_mode_visibility()
                            ui.notify(f"Optimizer mode: {e.value}", color="positive")
                        with ui.row().classes("w-full items-center gap-3 mt-2"):
                            ui.label("Mode").classes("text-xs text-gray-500")
                            ui.toggle(
                                {"wolds": "Wolds", "vanilla": "Vanilla"},
                                value=state.mode,
                                on_change=_on_mode_change,
                            ).props("dense")
                            # Arcane auto-place toggle. ON (default) = SA fills
                            # every arcane slot with ARCANE (color-only swaps).
                            # OFF = SA may swap arcane slots to DEAD for void
                            # trade-offs.
                            ui.label("Arcane").classes("text-xs text-gray-500 ml-3")
                            arcane_toggle = ui.toggle(
                                {True: "Auto-fill", False: "Optimize"},
                                value=state.auto_place_arcane,
                                on_change=lambda e: setattr(state, "auto_place_arcane", bool(e.value)),
                            ).props("dense")
                            arcane_toggle.tooltip(
                                "Auto-fill: every arcane slot gets an arcane card. "
                                "Optimize: SA may leave arcane slots dead to feed void core."
                            )

                # Cores
                core_rows: List[Tuple["ui.checkbox", "ui.number", "callable"]] = []
                # Parallel to ``_CORE_OPTIONS`` — used by _apply_mode_visibility
                # to hide mode-incompatible core rows (e.g. void/pluto in vanilla).
                core_row_containers: List["ui.element"] = []
                with ui.card().tight().classes("w-full"):
                    with ui.card_section():
                        with ui.row().classes("w-full items-center justify-between"):
                            ui.label("Cores").classes("text-sm font-semibold uppercase text-gray-500")
                            with ui.row().classes("gap-2"):
                                ui.button(
                                    "Enable all",
                                    on_click=lambda: _set_all_cores(True, core_rows),
                                ).props("flat dense color=primary")
                                ui.button(
                                    "Disable all",
                                    on_click=lambda: _set_all_cores(False, core_rows),
                                ).props("flat dense color=grey")
                        for idx, (ct, color) in enumerate(_CORE_OPTIONS):
                            row, cb, ov, sync = _build_core_row(idx, ct, color, state)
                            core_rows.append((cb, ov, sync))
                            core_row_containers.append(row)

                        # ── Bonus Cores ─────────────────────────────────
                        # User-adjustable delta on top of the deck's raw
                        # core-slot count. Defaults to the mode's `deckmod`
                        # (wolds=1, vanilla=0); the optimizer uses
                        # `max(0, deck.core_slots - DECKMOD + bonus_cores)`,
                        # so the value is unbounded — typing a huge negative
                        # number just clamps the run to 0 cores.
                        ui.separator().classes("my-2")
                        with ui.row().classes("w-full items-center gap-2"):
                            ui.label("Bonus Cores").classes("text-sm flex-grow")
                            def _on_bonus_cores_change(e):
                                # NiceGUI's number input returns a float; cast
                                # back to int and reject `None` (cleared input).
                                v = e.value
                                state.bonus_cores = int(v) if v is not None else 0
                            # Held by name so `_on_mode_change` can push the
                            # mode's new deckmod into the widget (state alone
                            # doesn't drive the UI here — no two-way binding).
                            bonus_cores_input = ui.number(
                                value=state.bonus_cores,
                                step=1,
                                format="%d",
                                on_change=_on_bonus_cores_change,
                            ).props("dense outlined").classes("w-24")
                        # Collapsible "?" with the long explanation. Keeps the
                        # default cores panel tidy; only opens when clicked.
                        with ui.expansion(
                            "What is Bonus Cores?", icon="help_outline",
                        ).classes("w-full text-xs").props("dense"):
                            ui.markdown(
                                "Adjusts how many core slots the optimizer "
                                "has to fill, beyond what the deck normally "
                                "provides.\n\n"
                                "**Positive** — add slots. In Wold's, the "
                                "Core Expertise ability lets you craft a "
                                "deck with one extra slot and strip the "
                                "temp core in a Deck Altar afterwards "
                                "(this is why the default in Wold's is "
                                "`1`).\n\n"
                                "**Negative** — reserve slots for cores "
                                "the optimizer doesn't consider. E.g. you "
                                "plan to slot a Bounty Core for resource "
                                "cards — set Bonus Cores to `-1` so the "
                                "optimizer only fills the remaining slots.\n\n"
                                "Vanilla has no equivalent free-slot "
                                "mechanic, so the default there is `0`."
                            ).classes("text-xs text-gray-400")

                # Inventory — two views (regular / forced) toggled by a button
                # in the card header. Both tables persist in the DOM; only the
                # active one is visible. Preset buttons apply to whichever view
                # is currently active.
                inv_inputs_regular: Dict[Tuple[CardType, Color], ui.number] = {}
                inv_inputs_forced:  Dict[Tuple[CardType, Color], ui.number] = {}
                # Forward refs for the toggle handler (defined here, closed over).
                regular_panel: ui.element  # bound below
                forced_panel:  ui.element  # bound below

                def _active_inputs() -> Dict[Tuple[CardType, Color], "ui.number"]:
                    return inv_inputs_forced if state.inventory_view == "forced" else inv_inputs_regular

                def _active_target() -> Dict[Tuple[CardType, Color], int]:
                    return state.forced_counts if state.inventory_view == "forced" else state.inventory_counts

                def _on_inventory_view_change(e):
                    state.inventory_view = e.value
                    regular_panel.set_visibility(state.inventory_view == "regular")
                    forced_panel.set_visibility(state.inventory_view == "forced")

                with ui.card().tight().classes("w-full"):
                    with ui.card_section():
                        with ui.row().classes("w-full items-center justify-between"):
                            with ui.row().classes("items-center gap-3"):
                                ui.label("Inventory") \
                                    .classes("text-sm font-semibold uppercase text-gray-500")
                                # Regular / Forced view toggle.
                                ui.toggle(
                                    {"regular": "Regular", "forced": "Forced"},
                                    value=state.inventory_view,
                                    on_change=_on_inventory_view_change,
                                ).props("dense")
                            with ui.row().classes("gap-2"):
                                ui.button(
                                    "Unlimited (100×)",
                                    on_click=lambda: _apply_preset(
                                        100, _active_inputs(), _active_target(),
                                    ),
                                ).props("flat dense color=primary")
                                ui.button(
                                    "Clear",
                                    on_click=lambda: _apply_preset(
                                        0, _active_inputs(), _active_target(),
                                    ),
                                ).props("flat dense color=grey")
                        # Brief helper text under the header.
                        ui.label(
                            "Regular: optimizer may use 0..N of each. "
                            "Forced: optimizer MUST place at least N (cap = regular + forced)."
                        ).classes("text-xs text-gray-500 mt-1")
                        # ── Regular table (per-row/column bulk-fill buttons) ─
                        regular_panel = ui.element("div")
                        with regular_panel:
                            inv_row_containers_regular = _build_inventory_table(
                                inv_inputs_regular, state.inventory_counts,
                                show_fill_buttons=True,
                            )
                        # ── Forced table (deep amber, no bulk-fill buttons) ──
                        # Dark-mode amber: still yellow-tinted so the user
                        # can recognise it as the "different" table, but dark
                        # enough that slate-100 text inputs remain legible.
                        forced_panel = ui.element("div")
                        with forced_panel:
                            inv_row_containers_forced = _build_inventory_table(
                                inv_inputs_forced, state.forced_counts,
                                bg_color="#3F2F0A",
                            )
                        # Initial visibility — only the active table is shown.
                        regular_panel.set_visibility(state.inventory_view == "regular")
                        forced_panel.set_visibility(state.inventory_view == "forced")

                # Run controls (this is where the Run button lives)
                with ui.card().tight().classes("w-full"):
                    with ui.card_section():
                        with ui.row().classes("w-full items-center gap-3"):
                            ui.number(label="SA iter", value=state.n_iter, format="%d", step=10_000,
                                      on_change=lambda e: setattr(state, "n_iter", int(e.value or 0))) \
                                .classes("w-32")
                            ui.number(label="Restarts", value=state.restarts, format="%d", step=1,
                                      on_change=lambda e: setattr(state, "restarts", int(e.value or 0))) \
                                .classes("w-28")
                            run_button = ui.button("Run").props("color=primary unelevated").classes("flex-grow")
                        # Backend indicator: green if Rust core is loaded, amber if pure-Python fallback.
                        if _RUST_OK:
                            ui.label("● Using Rust core (parallel restarts)") \
                                .style("color:#15803D; font-size:12px; margin-top:6px;")
                        else:
                            ui.label("● Using pure-Python fallback — much slower; build with --extra rust") \
                                .style("color:#B45309; font-size:12px; margin-top:6px;")

            # ── Preview panel (stats sidebar) ─────────────────────────────────
            # Built lazily on view-switch; pre-built once here so the element
            # exists for set_visibility() before any state is rendered into it.
            preview_panel = ui.column().classes("gap-3 w-full")
            _preview.build_stats_panel(preview_panel, state)

            # Initial visibility based on default view.
            optimize_panel.set_visibility(state.view == "optimize")
            preview_panel.set_visibility(state.view == "preview")

            # Initial mode-driven visibility (hides void/pluto/etc. if we
            # booted into vanilla via --mode vanilla on the command line).
            _apply_mode_visibility()

            # Wire the Run button now that everything else exists.
            # Return the coroutine directly (NOT via asyncio.create_task) so
            # NiceGUI awaits it within the right client/slot context — otherwise
            # element creation after the await fails with "slot stack is empty".
            run_button.on_click(
                lambda: _run_optimization(
                    state,
                    total_label=total_label,
                    cores_label=cores_label,
                    verify_label=verify_label,
                    grid_container=grid_container,
                    preview_panel=preview_panel,
                    run_button=run_button,
                    on_preview_change=_on_preview_change,
                )
            )


def _build_core_row(
    idx: int, ct: CoreType, color: Optional[Color], state: _AppState,
) -> Tuple["ui.element", "ui.checkbox", "ui.number", "callable"]:
    """Build one core row. Returns (row_container, checkbox, override_input,
    sync_fn). The row container lets the caller toggle visibility per mode;
    the rest drive the Enable/Disable-all buttons + value sync."""
    enabled, override = state.core_state.get(idx, (False, None))
    row = ui.row().classes("w-full items-center gap-2 no-wrap")
    with row:
        cb = ui.checkbox(_core_label(ct, color), value=enabled).classes("flex-grow")
        ov = ui.number(
            label="override",
            value=override,
            format="%.3f",
            step=0.05,
        ).props("dense outlined").classes("w-28")

        def _sync(_e=None, _i=idx, _cb=cb, _ov=ov):
            v = _ov.value
            override_val = float(v) if v not in (None, "") else None
            state.core_state[_i] = (bool(_cb.value), override_val)

        cb.on("update:model-value", _sync)
        ov.on("update:model-value", _sync)
        _sync()  # initialize
    return row, cb, ov, _sync


# Per-row / per-column fill-button value. Matches the global "Unlimited (100×)"
# preset so the meaning is consistent across the UI.
_BULK_FILL_VALUE = 100


# ── Mode-aware option visibility ─────────────────────────────────────────────
# These pure helpers decide which inventory rows / core options should be hidden
# in the current (mode, card_class) combination. Used both by the GUI for
# show/hide and by the Run handler to filter out hidden-but-still-in-state
# entries — that way switching modes doesn't destroy the user's previous values.

def _hidden_inventory_types(mode: str, card_class: CardClass) -> "set[CardType]":
    """Inventory rows that should be hidden + ignored at Run time.

    Vanilla: drop the Wold's-only mechanics (evo greed, surr greed, deluxe cards).
    Vanilla + 'stat' class (the SHINY enum re-labeled in vanilla): also drop the
    positional types (row / col / surr / diag) — vanilla 'stat' decks only use
    typeless cards.
    """
    hidden: set[CardType] = set()
    if mode == "vanilla":
        hidden.update({
            CardType.EVO_GREED, CardType.SURR_GREED, CardType.DELUXE,
        })
        if card_class == CardClass.SHINY:
            hidden.update({
                CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
            })
    return hidden


def _hidden_core_types(mode: str) -> "set[CoreType]":
    """Core options that should be hidden + ignored at Run time."""
    if mode == "vanilla":
        return {CoreType.DELUXE_CORE, CoreType.VOID_CORE, CoreType.PLUTO_CORE}
    return set()


def _class_select_options(mode: str) -> Dict[str, str]:
    """Class-picker labels. Vanilla relabels 'Shiny' → 'Stat' since vanilla
    shiny decks are pure typeless / stat-card decks, not positional decks."""
    return {
        CardClass.SHINY.value: "Stat" if mode == "vanilla" else "Shiny",
        CardClass.EVO.value:   "Evo",
    }


def _build_inventory_table(
    inputs:      Dict[Tuple[CardType, Color], "ui.number"],
    target_dict: Dict[Tuple[CardType, Color], int],
    *,
    bg_color:          Optional[str] = None,
    show_fill_buttons: bool          = False,
) -> Dict[CardType, "ui.element"]:
    """Populate ``inputs`` with one number widget per (type, color), bound to
    ``target_dict``. Optional ``bg_color`` paints the table container — used to
    distinguish the forced-inventory view from the regular one.

    When ``show_fill_buttons`` is true, a small "100×" button is rendered above
    each color column and to the left of each card-type row. Clicking it fills
    that entire column / row with ``_BULK_FILL_VALUE``.

    Returns a mapping ``{CardType: row_container}`` so the caller can show/hide
    individual rows by mode without re-rendering the whole table. Each body row
    is its own CSS grid so ``row.set_visibility(False)`` removes only that
    row's slot from layout (and keeps the rest of the table aligned).
    """
    container_style = (
        "display:flex;flex-direction:column;"
        "gap: 6px;"
        "margin-top:8px;"
        "padding: 8px;"
        "border-radius: 6px;"
    )
    if bg_color:
        container_style += f"background: {bg_color};"

    # Each row (header + body) gets this grid template so cells stay aligned.
    row_grid_style = (
        f"display:grid;"
        f"grid-template-columns: 130px repeat({len(_COLORS)}, 1fr);"
        f"gap: 6px;align-items:center;"
    )

    row_containers: Dict[CardType, "ui.element"] = {}

    with ui.element("div").style(container_style):
        # ── Header row ───────────────────────────────────────────────────────
        with ui.element("div").style(row_grid_style):
            ui.label("").style("font-size:10px;")
            for c in _COLORS:
                with ui.element("div").style(
                    "display:flex;flex-direction:column;justify-content:center;"
                    "align-items:center;gap:3px;"
                ):
                    ui.element("div").style(
                        f"width:14px;height:14px;border-radius:50%;"
                        f"background:{_GAME_COLOR_HEX[c]};"
                        f"border:1px solid rgba(0,0,0,.15);"
                    )
                    if show_fill_buttons:
                        btn = ui.button(
                            f"{_BULK_FILL_VALUE}×",
                            on_click=lambda _e=None, color=c: _fill_column(
                                color, _BULK_FILL_VALUE, inputs, target_dict,
                            ),
                        ).props("flat dense color=primary")
                        btn.style(
                            "min-width:auto;padding:0 5px;font-size:9px;line-height:14px;"
                            "height:18px;"
                        )
                        btn.tooltip(f"Fill the {c.value} column with {_BULK_FILL_VALUE}")

        # ── Body rows (one grid container per row, for per-row visibility) ──
        for t in _INVENTORY_TYPES:
            row = ui.element("div").style(row_grid_style)
            row_containers[t] = row
            with row:
                with ui.element("div").style(
                    "display:flex;align-items:center;gap:6px;padding-left:4px;"
                ):
                    if show_fill_buttons:
                        btn = ui.button(
                            f"{_BULK_FILL_VALUE}×",
                            on_click=lambda _e=None, ct=t: _fill_row(
                                ct, _BULK_FILL_VALUE, inputs, target_dict,
                            ),
                        ).props("flat dense color=primary")
                        btn.style(
                            "min-width:auto;padding:0 5px;font-size:9px;line-height:14px;"
                            "height:18px;"
                        )
                        btn.tooltip(f"Fill every color of {_TYPE_LABEL[t]} with {_BULK_FILL_VALUE}")
                    ui.label(_TYPE_GLYPH.get(t, "?")) \
                        .style("font-family:'JetBrains Mono',monospace;width:18px;text-align:center;color:#CBD5E1;")
                    ui.label(_TYPE_LABEL[t]).classes("text-xs")
                for c in _COLORS:
                    init = target_dict.get((t, c), 0)
                    w = ui.number(value=init, min=0, max=999, step=1, format="%d") \
                        .props("dense outlined hide-bottom-space") \
                        .style("width:100%;")
                    def _bind(_e=None, key=(t, c), widget=w, td=target_dict):
                        td[key] = int(widget.value or 0)
                    w.on("update:model-value", _bind)
                    _bind()
                    inputs[(t, c)] = w

    return row_containers


def _apply_preset(
    value:       int,
    inputs:      Dict[Tuple[CardType, Color], "ui.number"],
    target_dict: Dict[Tuple[CardType, Color], int],
) -> None:
    """Set every widget + its backing dict entry to ``value``."""
    for key, widget in inputs.items():
        widget.value = value
        target_dict[key] = value


def _fill_column(
    color:       Color,
    value:       int,
    inputs:      Dict[Tuple[CardType, Color], "ui.number"],
    target_dict: Dict[Tuple[CardType, Color], int],
) -> None:
    """Fill every (type, ``color``) cell with ``value``."""
    for (t, c), widget in inputs.items():
        if c == color:
            widget.value = value
            target_dict[(t, c)] = value


def _fill_row(
    card_type:   CardType,
    value:       int,
    inputs:      Dict[Tuple[CardType, Color], "ui.number"],
    target_dict: Dict[Tuple[CardType, Color], int],
) -> None:
    """Fill every (``card_type``, color) cell with ``value``."""
    for (t, c), widget in inputs.items():
        if t == card_type:
            widget.value = value
            target_dict[(t, c)] = value


def _set_all_cores(
    enabled: bool,
    rows: List[Tuple["ui.checkbox", "ui.number", "callable"]],
) -> None:
    """Programmatically toggle every core checkbox + force state resync."""
    for cb, _ov, sync in rows:
        cb.value = enabled
        sync()


# ──────────────────────────────────────────────────────────────────────────────
# Run handler
# ──────────────────────────────────────────────────────────────────────────────

async def _run_optimization(
    state: _AppState,
    *,
    total_label: ui.label,
    cores_label: ui.label,
    verify_label: ui.label,
    grid_container: ui.element,
    preview_panel: ui.element,
    run_button: ui.button,
    on_preview_change: callable,
) -> None:
    # Filter out options that are hidden in the current mode. State is
    # preserved (the values stay in the UI for when the user switches back),
    # but they don't participate in this run.
    hidden_types = _hidden_inventory_types(state.mode, state.card_class)
    hidden_cores = _hidden_core_types(state.mode)

    counts        = {k: v for k, v in state.inventory_counts.items()
                     if v > 0 and k[0] not in hidden_types}
    forced_counts = {k: v for k, v in state.forced_counts.items()
                     if v > 0 and k[0] not in hidden_types}
    if not counts and not forced_counts:
        ui.notify("Inventory is empty — set some card counts first.", color="warning")
        return

    # Pre-flight: forced cards must fit in the deck (optimizer enforces this
    # too, but a clean GUI notification is friendlier than an exception).
    total_forced = sum(forced_counts.values())
    if total_forced > len(state.deck.slots):
        ui.notify(
            f"Forced inventory ({total_forced} cards) exceeds deck capacity "
            f"({len(state.deck.slots)} slots). Remove some forced entries.",
            color="negative", multi_line=True,
        )
        return

    cores: set[CoreSpec] = set()
    for idx, (enabled, override) in state.core_state.items():
        if not enabled:
            continue
        ct, color = _CORE_OPTIONS[idx]
        if ct in hidden_cores:
            continue
        cores.add(CoreSpec(core_type=ct, color=color, override=override))

    inv = CardInventory(
        counts=counts,
        card_class=state.card_class,
        cores=CoreInventory(cores=cores),
        forced_counts=forced_counts,
    )

    # Apply the user's Bonus Cores override to a fresh deck copy. The deck
    # in DECKS already has `core_slots = base + _cfg.DECKMOD` baked in at
    # load time, so we subtract DECKMOD back out to recover the raw base
    # and then add the user-set bonus. Clamped to 0 — typing a big negative
    # just yields a zero-core optimization.
    effective_core_slots = max(0, state.deck.core_slots - _cfg.DECKMOD + state.bonus_cores)
    run_deck = state.deck.with_core_slots(effective_core_slots)

    run_button.props(add="loading")
    run_button.disable()
    total_label.text = "Optimizing…"
    cores_label.text = ""
    verify_label.text = ""

    try:
        result: InventoryResult = await run.io_bound(
            optimize_inventory, run_deck, inv, state.n_iter, state.restarts,
            state.auto_place_arcane,
        )
        state.last_result = result
        total_label.text = f"NDM  {result.score:,.2f}"
        cores_label.text = _format_cores(result.cores_used)
        _set_verification_badge(verify_label, result)
        # Drop preview assignments whose slot's class family changed under the
        # new layout (keep-by-(row,col) rule). Refresh the preview stats panel
        # if the user is currently viewing it.
        dropped = _preview.reset_assignments_on_run(state)
        _render_deck_grid(grid_container, state, on_preview_change=on_preview_change)
        if state.view == "preview":
            _preview.build_stats_panel(preview_panel, state)
        if dropped:
            ui.notify(
                f"Preview: dropped {dropped} card assignment(s) whose slot family changed.",
                color="warning",
            )
        ui.notify("Done.", color="positive")
    except Exception as exc:  # noqa: BLE001
        ui.notify(f"Optimization failed: {exc}", color="negative", multi_line=True)
        total_label.text = "NDM  —"
        verify_label.text = ""
    finally:
        run_button.props(remove="loading")
        run_button.enable()


_VERIFY_TOL = 1e-6


def _set_verification_badge(label: ui.label, result: InventoryResult) -> None:
    """Compare Rust vs Python totals and surface a colored badge."""
    if result.rust_score is None:
        label.text = "○ No cross-check (pure-Python path — build with --extra rust to enable)"
        label.style("color:#B45309;")
        return
    rust = result.rust_score
    py   = result.python_score
    denom = max(1.0, abs(rust))
    rel = abs(rust - py) / denom
    if rel <= _VERIFY_TOL:
        label.text = f"✓ Python & Rust agree (Δ={abs(rust - py):.2e})"
        label.style("color:#15803D;")
    else:
        label.text = (
            f"✗ MISMATCH — Rust={rust:,.4f}  Python={py:,.4f}  Δ={abs(rust - py):.4f}"
        )
        label.style("color:#B91C1C; font-weight:600;")


def _format_cores(cores: "frozenset[CoreSpec]") -> str:
    parts: List[str] = []
    for s in sorted(cores, key=lambda x: (x.core_type.value, (x.color.value if x.color else ""))):
        label = _core_label(s.core_type, s.color)
        if s.override is not None:
            label = f"{label} ({s.override:.2f})"
        parts.append(label)
    return "  ·  ".join(parts) if parts else "(no cores)"


# ──────────────────────────────────────────────────────────────────────────────
# Entry point
# ──────────────────────────────────────────────────────────────────────────────

@ui.page("/")
def _index_page() -> None:
    """Route handler — re-builds the UI for each browser connection.

    Registering via ``@ui.page`` (rather than top-level element construction
    in ``main``) is required when launching through a console-script entry,
    because NiceGUI's "script mode" auto-reload otherwise can't find a script
    file to re-execute and raises at startup.
    """
    _build_page()


_SHUTDOWN_GRACE_SECONDS = 5.0
_active_clients: Set[str] = set()
_shutdown_task: Optional[asyncio.Task] = None


def _install_auto_shutdown() -> None:
    """Shut the server down a few seconds after the last browser tab disconnects.

    The grace period absorbs page refreshes (which briefly disconnect + reconnect)
    so we don't kill the server on every reload. If the API surface differs from
    what's expected (different NiceGUI version), we log a clear notice and skip
    auto-shutdown rather than fail loudly.
    """
    try:
        @app.on_connect
        def _on_connect(client) -> None:  # type: ignore[no-redef]
            global _shutdown_task
            _active_clients.add(str(getattr(client, "id", id(client))))
            if _shutdown_task is not None and not _shutdown_task.done():
                _shutdown_task.cancel()
                _shutdown_task = None

        @app.on_disconnect
        def _on_disconnect(client) -> None:  # type: ignore[no-redef]
            global _shutdown_task
            _active_clients.discard(str(getattr(client, "id", id(client))))
            if not _active_clients:
                _shutdown_task = asyncio.create_task(_delayed_shutdown())
    except Exception as exc:  # noqa: BLE001
        # NiceGUI API mismatch — degrade gracefully instead of failing startup.
        print(
            f"[gui] WARN: auto-shutdown wiring failed ({type(exc).__name__}: {exc}). "
            f"Server will keep running until you Ctrl+C it.",
            file=sys.stderr,
        )


async def _delayed_shutdown() -> None:
    try:
        await asyncio.sleep(_SHUTDOWN_GRACE_SECONDS)
    except asyncio.CancelledError:
        return
    if _active_clients:
        return
    print("[gui] No active browser tabs — shutting down.")
    try:
        app.shutdown()
    except Exception as exc:  # noqa: BLE001
        print(f"[gui] WARN: app.shutdown() failed ({exc}); forcing exit.", file=sys.stderr)
        sys.exit(0)


def main() -> None:
    """Console-script entry. Boots NiceGUI + opens a browser tab."""
    _install_auto_shutdown()
    ui.run(host="127.0.0.1", port=8080, reload=False, show=True, title="Deck Optimizer")


if __name__ in {"__main__", "__mp_main__"}:
    main()
