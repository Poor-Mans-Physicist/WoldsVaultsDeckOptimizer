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
    return "#FFFFFF"


_GAME_COLOR_HEX: Dict[Color, str] = {
    Color.RED:    "#E74C3C",
    Color.GREEN:  "#27AE60",
    Color.BLUE:   "#3498DB",
    Color.YELLOW: "#F1C40F",
}


_INVENTORY_TYPES: List[CardType] = [
    CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG,
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
    core_state: Dict[int, Tuple[bool, Optional[float]]] = field(default_factory=dict)
    last_result: Optional[InventoryResult] = None
    n_iter: int = 60_000
    restarts: int = 12
    # Top-level view: "optimize" (default UI) or "preview" (assign stat cards
    # to the last optimized layout and see player-facing stat totals).
    view: str = "optimize"
    # Slot -> (modifier_key, tier). Survives Run when slot family is unchanged.
    preview_assignments: Dict[Tuple[int, int], Tuple[str, int]] = field(default_factory=dict)


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
            f"background: #FAFBFC;"
            f"border: 1px solid #E5E7EB;"
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

    bg     = _slot_bg(t) if t != CardType.EMPTY else "#FFFFFF"
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

    with ui.element("div").style(
        f"width:{_SLOT_PX}px;height:{_SLOT_PX}px;"
        f"background:{bg};"
        f"border: 1px solid rgba(0,0,0,.08);"
        f"border-radius: 8px;"
        f"position:relative;"
        f"opacity:{opacity};"
        f"display:flex;flex-direction:column;align-items:center;justify-content:center;"
        f"font-family:'JetBrains Mono','Consolas',monospace;"
        f"cursor:{cursor};"
    ) as slot_div:
        ui.label(glyph).style("font-size:22px; font-weight:600; line-height:1;")
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
                    "• Pick a deck and class, then enter how many of each (type, color) card you own "
                    "in the inventory table. Use Unlimited (100×) for unconstrained testing or Clear to reset."
                ).classes("text-xs text-gray-600")
                ui.label(
                    "• Toggle the cores you own. The override field replaces the config default — "
                    "for PURE and DELUXE_CORE it overrides only the scale term (formula stays "
                    "base + scale × n)."
                ).classes("text-xs text-gray-600")
                ui.label(
                    "• Hit Run. The deck repaints with the optimizer's chosen placement; each tile shows "
                    "the card's symbol and its NDM contribution. Click any tile to see the full math "
                    "(base × cores × boost) for that slot."
                ).classes("text-xs text-gray-600")
                ui.label(
                    "• The badge above the deck reports whether the Rust and Python paths agree on the "
                    "total. Green = agreement, red = mismatch (with both numbers shown)."
                ).classes("text-xs text-gray-600")


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
        )
        ui.label(label).style("color:#1F2937;")


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
    state = _AppState(deck=_cfg.DECKS[0], mode=_cfg.MODE)
    for idx in range(len(_CORE_OPTIONS)):
        state.core_state[idx] = (False, None)

    ui.add_head_html("""
        <style>
          body { background: #F4F5F7; }
          .q-card { background: #FFFFFF; border: 1px solid #E5E7EB; }
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
                            deck_select = ui.select(
                                options=[d.name for d in _cfg.DECKS],
                                value=state.deck.name,
                                label="Deck",
                                on_change=_on_deck_change,
                            ).classes("flex-grow")
                            ui.select(
                                options={CardClass.SHINY.value: "Shiny", CardClass.EVO.value: "Evo"},
                                value=state.card_class.value,
                                label="Class",
                                on_change=lambda e: setattr(state, "card_class", CardClass(e.value)),
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
                            
                            deck_select.options = [d.name for d in _cfg.DECKS]
                            deck_select.value = state.deck.name
                            deck_select.update()
                            total_label.text = "NDM  —"
                            cores_label.text = ""
                            verify_label.text = ""
                            _render_deck_grid(
                                grid_container, state,
                                on_preview_change=_on_preview_change,
                            )
                            if state.view == "preview":
                                _preview.build_stats_panel(preview_panel, state)
                            ui.notify(f"Optimizer mode: {e.value}", color="positive")
                        with ui.row().classes("w-full items-center gap-3 mt-2"):
                            ui.label("Mode").classes("text-xs text-gray-500")
                            ui.toggle(
                                {"wolds": "Wolds", "vanilla": "Vanilla"},
                                value=state.mode,
                                on_change=_on_mode_change,
                            ).props("dense")

                # Cores
                core_rows: List[Tuple["ui.checkbox", "ui.number", "callable"]] = []
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
                            core_rows.append(_build_core_row(idx, ct, color, state))

                # Inventory
                inv_inputs: Dict[Tuple[CardType, Color], ui.number] = {}
                with ui.card().tight().classes("w-full"):
                    with ui.card_section():
                        with ui.row().classes("w-full items-center justify-between"):
                            ui.label("Inventory") \
                                .classes("text-sm font-semibold uppercase text-gray-500")
                            with ui.row().classes("gap-2"):
                                ui.button(
                                    "Unlimited (100×)",
                                    on_click=lambda: _apply_preset(100, inv_inputs, state),
                                ).props("flat dense color=primary")
                                ui.button(
                                    "Clear",
                                    on_click=lambda: _apply_preset(0, inv_inputs, state),
                                ).props("flat dense color=grey")
                        _build_inventory_table(inv_inputs, state)

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
) -> Tuple["ui.checkbox", "ui.number", "callable"]:
    """Build one core row. Returns (checkbox, override_input, sync_fn) so the
    Enable/Disable-all buttons can drive widget state programmatically."""
    enabled, override = state.core_state.get(idx, (False, None))
    with ui.row().classes("w-full items-center gap-2 no-wrap"):
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
    return cb, ov, _sync


def _build_inventory_table(
    inputs: Dict[Tuple[CardType, Color], "ui.number"],
    state: _AppState,
) -> None:
    """Populate ``inputs`` with one number widget per (type, color)."""
    with ui.element("div").style(
        f"display:grid;"
        f"grid-template-columns: 130px repeat({len(_COLORS)}, 1fr);"
        f"gap: 6px;align-items:center;"
        f"margin-top:8px;"
    ):
        # Header row: empty cell, then a color dot per color column.
        ui.label("").style("font-size:10px;")
        for c in _COLORS:
            with ui.element("div").style("display:flex;justify-content:center;align-items:center;"):
                ui.element("div").style(
                    f"width:14px;height:14px;border-radius:50%;"
                    f"background:{_GAME_COLOR_HEX[c]};"
                    f"border:1px solid rgba(0,0,0,.15);"
                )

        # Body rows
        for t in _INVENTORY_TYPES:
            with ui.element("div").style(
                "display:flex;align-items:center;gap:6px;padding-left:4px;"
            ):
                ui.label(_TYPE_GLYPH.get(t, "?")) \
                    .style("font-family:'JetBrains Mono',monospace;width:18px;text-align:center;color:#4B5563;")
                ui.label(_TYPE_LABEL[t]).classes("text-xs")
            for c in _COLORS:
                init = state.inventory_counts.get((t, c), 0)
                w = ui.number(value=init, min=0, max=999, step=1, format="%d") \
                    .props("dense outlined hide-bottom-space") \
                    .style("width:100%;")
                def _bind(_e=None, key=(t, c), widget=w):
                    state.inventory_counts[key] = int(widget.value or 0)
                w.on("update:model-value", _bind)
                _bind()
                inputs[(t, c)] = w


def _apply_preset(
    value: int,
    inputs: Dict[Tuple[CardType, Color], "ui.number"],
    state: _AppState,
) -> None:
    for key, widget in inputs.items():
        widget.value = value
        state.inventory_counts[key] = value


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
    counts = {k: v for k, v in state.inventory_counts.items() if v > 0}
    if not counts:
        ui.notify("Inventory is empty — set some card counts first.", color="warning")
        return

    cores: set[CoreSpec] = set()
    for idx, (enabled, override) in state.core_state.items():
        if not enabled:
            continue
        ct, color = _CORE_OPTIONS[idx]
        cores.add(CoreSpec(core_type=ct, color=color, override=override))

    inv = CardInventory(
        counts=counts,
        card_class=state.card_class,
        cores=CoreInventory(cores=cores),
    )

    run_button.props(add="loading")
    run_button.disable()
    total_label.text = "Optimizing…"
    cores_label.text = ""
    verify_label.text = ""

    try:
        result: InventoryResult = await run.io_bound(
            optimize_inventory, state.deck, inv, state.n_iter, state.restarts
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
