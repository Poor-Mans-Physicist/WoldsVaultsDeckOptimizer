"""Preview-mode UI for the GUI.

The Optimize panel produces a placement + per-slot NDM. Preview lets the user
assign a concrete stat card (with tier) to each scoring slot and then see the
accumulated player-facing stats:

    contribution(slot) = pool[tier].value × per_slot_NDM(slot)
    flat_total[attr]    = Σ contribution over slots where card is flat
    percent_total[attr] = Σ contribution over slots where card is percent

Slots are gated by class family (Shiny / Evo / Deluxe / Typeless). Greed and
dead slots are inert.

Public API (called from ``src.gui``):
    slot_family(slot_type, deck_class)            -> Optional[str]
    is_assignable_slot(slot_type, deck_class)     -> bool
    open_assign_dialog(pos, slot_type, deck_class, ndm, state, on_done)
    build_stats_panel(container, state)
    reset_assignments_on_run(state, old_deck, old_class)
"""
from __future__ import annotations

from typing import Callable, Dict, Optional, Tuple

from nicegui import ui

from .modifiers import CardEntry, cards_by_family, get_card
from .types import CardClass, CardType, Position

# ──────────────────────────────────────────────────────────────────────────────
# Family resolution
# ──────────────────────────────────────────────────────────────────────────────

_POSITIONAL_TYPES = {CardType.ROW, CardType.COL, CardType.SURR, CardType.DIAG}


def slot_family(slot_type: CardType, deck_class: CardClass) -> Optional[str]:
    """Which family of stat cards may be placed in this slot.

    Returns None for slots that don't hold stat cards (greed, dead, empty).
    """
    if slot_type in _POSITIONAL_TYPES:
        return "shiny" if deck_class == CardClass.SHINY else "evo"
    if slot_type == CardType.DELUXE:
        return "deluxe"
    if slot_type == CardType.TYPELESS:
        return "typeless"
    return None


def is_assignable_slot(slot_type: CardType, deck_class: CardClass) -> bool:
    return slot_family(slot_type, deck_class) is not None


# ──────────────────────────────────────────────────────────────────────────────
# Assignment dialog
# ──────────────────────────────────────────────────────────────────────────────

# Brief two-letter abbreviation per attribute, for the in-slot badge.
# Falls back to the first 3 chars of the attribute name if absent.
_ATTR_ABBREV: Dict[str, str] = {
    "attack_damage":             "ATK",
    "attack_speed_percent":      "AS%",
    "ability_power":             "AP",
    "ability_power_percent":     "AP%",
    "health":                    "HP",
    "health_percentile":         "HP%",
    "armor_percentile":          "AR%",
    "damage_increase":           "DMG%",
    "cooldown_reduction":        "CDR",
    "critical_hit_mitigation":   "CHM",
    "healing_effectiveness":     "HEAL",
    "knockback_resistance":      "KBR",
    "lucky_hit_chance":          "LHC",
    "mana_additive":             "MANA",
    "mana_additive_percentile":  "MANA%",
    "mana_regen":                "MREG",
    "mining_speed":              "MINE",
    "movement_speed":            "MOVE",
    "item_quantity":             "IQ",
    "item_rarity":               "IR",
    "soul_chance":               "SOUL",
    "resistance":                "RES",
    "area_of_effect":            "AOE",
    "thorns_damage_flat":        "THRN",
    "trap_disarming":            "TRAP",
    "added_ability_level":       "ABIL",
    "added_talent_level":        "TAL",
    "copiously":                 "COPI",
    "random_vault_modifier":     "RVM",
}


def attr_abbrev(attribute_short: str) -> str:
    return _ATTR_ABBREV.get(attribute_short, attribute_short[:4].upper())


def open_assign_dialog(
    pos:         Position,
    slot_type:   CardType,
    deck_class:  CardClass,
    ndm:         float,
    state,
    on_done:     Callable[[], None],
) -> None:
    """Open a modal dialog letting the user assign / change / clear a slot's card.

    ``state`` must expose ``preview_assignments: Dict[Position, Tuple[str, int]]``.
    ``on_done`` is fired after any mutation so the caller can re-render.
    """
    family = slot_family(slot_type, deck_class)
    if family is None:
        return  # not assignable — caller should have prevented the click

    all_cards = cards_by_family(family)
    current = state.preview_assignments.get(pos)  # (key, tier) or None

    # Build the dialog
    dialog = ui.dialog()
    with dialog, ui.card().style(
        "min-width: 540px; max-width: 720px;"
        "max-height: 80vh; display: flex; flex-direction: column;"
    ):
        # Header
        with ui.row().classes("w-full items-center justify-between"):
            ui.label(
                f"Slot ({pos[0]},{pos[1]}) · {family.title()} family · NDM = {ndm:.2f}"
            ).classes("text-sm font-semibold")
            ui.button(icon="close", on_click=dialog.close).props("flat dense round")

        if not all_cards:
            ui.label(
                f"No {family} cards loaded from modifiers.json."
            ).classes("text-sm text-gray-500 mt-2")
            return

        # Search
        search_input = ui.input(label="Search", placeholder="e.g. attack, health, mana") \
            .props("dense outlined clearable") \
            .classes("w-full mt-2")

        # Scrollable list container
        list_container = ui.element("div").style(
            "flex: 1 1 auto; overflow-y: auto; "
            "border: 1px solid #E5E7EB; border-radius: 6px; "
            "padding: 4px; margin-top: 6px;"
        )

        def _do_clear() -> None:
            state.preview_assignments.pop(pos, None)
            dialog.close()
            on_done()

        def _do_assign(card: CardEntry, tier: int) -> None:
            state.preview_assignments[pos] = (card.key, tier)
            dialog.close()
            on_done()

        def _redraw_list(filter_text: str) -> None:
            list_container.clear()
            needle = (filter_text or "").strip().lower()
            with list_container:
                shown = 0
                for card in all_cards:
                    if needle and needle not in card.name.lower() \
                            and needle not in card.attribute_short.lower():
                        continue
                    _render_card_row(card, current, _do_assign)
                    shown += 1
                if shown == 0:
                    ui.label("(no matches)").classes("text-xs text-gray-400 p-2")

        search_input.on(
            "update:model-value",
            lambda e: _redraw_list(search_input.value or ""),
        )
        _redraw_list("")

        # Footer
        with ui.row().classes("w-full items-center justify-end mt-2 gap-2"):
            if current is not None:
                ui.button("Clear assignment", on_click=_do_clear) \
                    .props("flat dense color=negative")
            ui.button("Cancel", on_click=dialog.close).props("flat dense")

    dialog.open()


def _render_card_row(
    card:    CardEntry,
    current: Optional[Tuple[str, int]],
    assign:  Callable[[CardEntry, int], None],
) -> None:
    """One card row: name + attribute + percent badge + tier buttons."""
    is_selected_card = (current is not None and current[0] == card.key)
    selected_tier    = current[1] if is_selected_card else None

    with ui.element("div").style(
        f"display:flex; align-items:center; gap:8px; padding:6px 8px;"
        f"border-bottom:1px solid #334155;"
        f"{'background:#312E81;' if is_selected_card else ''}"
    ):
        with ui.column().classes("gap-0").style("flex: 1 1 auto; min-width: 0;"):
            with ui.row().classes("items-center gap-2 no-wrap"):
                ui.label(card.name).style(
                    "font-size: 13px; font-weight: 500; color: #F1F5F9;"
                )
                if card.is_percent:
                    ui.label("%").style(
                        "font-size: 10px; background:#FEF3C7; color:#92400E;"
                        "border-radius: 4px; padding: 0 5px;"
                    )
            ui.label(card.display_attribute).style(
                "font-size: 11px; color: #94A3B8;"
            )

        # Tier buttons
        with ui.row().classes("gap-1 no-wrap"):
            for ct in card.tiers:
                _label = f"T{ct.tier}"
                value_text = (
                    f"{ct.value * 100:.2f}%" if card.is_percent else f"{ct.value:g}"
                )
                btn = ui.button(_label) \
                    .props("dense outlined").classes("text-xs") \
                    .style("min-width: 36px; padding: 2px 6px;")
                btn.tooltip(f"Tier {ct.tier}: {value_text}")
                if selected_tier == ct.tier:
                    btn.props(remove="outlined")
                    btn.props(add="color=primary unelevated")
                btn.on_click(lambda _e=None, c=card, t=ct.tier: assign(c, t))


# ──────────────────────────────────────────────────────────────────────────────
# Stats panel
# ──────────────────────────────────────────────────────────────────────────────

def build_stats_panel(container: ui.element, state) -> None:
    """Render the accumulated-stats sidebar.

    ``state`` must expose:
        last_result          (None or InventoryResult)
        preview_assignments  Dict[Position, Tuple[str, int]]
        deck                 Deck
        card_class           CardClass
    """
    container.clear()
    with container:
        with ui.card().tight().classes("w-full"):
            with ui.card_section():
                ui.label("Player stats (preview)") \
                    .classes("text-sm font-semibold uppercase text-gray-500")

                if state.last_result is None:
                    ui.label(
                        "Run the optimizer first to produce a layout."
                    ).classes("text-xs text-gray-500 mt-2")
                    return

                flat, percent, n_assigned, total_slots = _aggregate(state)

                with ui.row().classes("items-center gap-3 mt-1"):
                    ui.label(
                        f"{n_assigned} / {total_slots} slots assigned"
                    ).classes("text-xs text-gray-500")

                if not flat and not percent:
                    ui.label("Click a slot in the deck to assign a card.") \
                        .classes("text-xs text-gray-500 mt-2")
                    return

                if flat:
                    ui.separator().classes("my-2")
                    ui.label("Flat").classes("text-xs font-semibold uppercase text-gray-500")
                    for attr, val in sorted(flat.items()):
                        _stat_line(attr, val, is_percent=False)

                if percent:
                    ui.separator().classes("my-2")
                    ui.label("Percent").classes("text-xs font-semibold uppercase text-gray-500")
                    for attr, val in sorted(percent.items()):
                        _stat_line(attr, val, is_percent=True)


def _stat_line(attribute: str, value: float, *, is_percent: bool) -> None:
    short = attribute.split(":", 1)[-1]
    label = " ".join(w.capitalize() for w in short.split("_") if w)
    if is_percent:
        rhs = f"+{value * 100:.2f}%"
    else:
        rhs = f"+{value:.2f}"
    with ui.row().classes("w-full items-center justify-between"):
        ui.label(label).style("font-size: 13px; color: #F1F5F9;")
        ui.label(rhs).style(
            "font-size: 13px; font-weight: 600; font-family: 'JetBrains Mono', monospace;"
            # Bright green for positives (visible on dark), muted slate for zero.
            f"color: {'#4ADE80' if value > 0 else '#94A3B8'};"
        )


def _aggregate(state) -> Tuple[Dict[str, float], Dict[str, float], int, int]:
    """Sum flat + percent contributions per attribute.

    Returns (flat_totals, percent_totals, n_assigned, n_assignable_slots).

    A slot is "assignable" when the optimizer placed an assignable CardType
    there in the current result. We read slot types from ``result.assignment``
    because the Deck object alone doesn't carry per-slot types — those come
    from optimization.
    """
    flat:    Dict[str, float] = {}
    percent: Dict[str, float] = {}

    deck_class = state.card_class
    result     = state.last_result

    n_slots = 0
    if result is not None:
        for _pos, (slot_type, _color) in result.assignment.items():
            if is_assignable_slot(slot_type, deck_class):
                n_slots += 1

    n_assigned = 0
    for pos, (key, tier) in state.preview_assignments.items():
        card = get_card(key)
        if card is None:
            continue
        # Find the tier's pool value.
        tier_val = next((t.value for t in card.tiers if t.tier == tier), None)
        if tier_val is None:
            continue
        ndm = (result.per_slot_ndm.get(pos, 0.0) if result is not None else 0.0)
        contribution = tier_val * ndm
        bucket = percent if card.is_percent else flat
        bucket[card.attribute] = bucket.get(card.attribute, 0.0) + contribution
        n_assigned += 1

    return flat, percent, n_assigned, n_slots


# ──────────────────────────────────────────────────────────────────────────────
# Reset / migrate after Run
# ──────────────────────────────────────────────────────────────────────────────

def reset_assignments_on_run(state) -> int:
    """Drop preview assignments whose slot's class family has changed.

    Called immediately after a successful Run. Assignments survive if BOTH:
      - The new layout has a card placed at (row, col) — i.e. the slot still
        exists with a non-greed type.
      - The new slot's class family matches the previously assigned card's family
        (the card itself remembers its family, so we don't need old state).

    Returns the count of dropped assignments (for an optional UI notice).
    """
    new_result     = state.last_result
    new_class      = state.card_class
    new_assignment = new_result.assignment if new_result is not None else {}

    keepers: Dict[Position, Tuple[str, int]] = {}
    dropped = 0
    for pos, (key, tier) in state.preview_assignments.items():
        card = get_card(key)
        if card is None:
            dropped += 1
            continue
        placed = new_assignment.get(pos)
        if placed is None:
            dropped += 1
            continue
        new_family = slot_family(placed[0], new_class)
        if new_family != card.family:
            dropped += 1
            continue
        keepers[pos] = (key, tier)

    state.preview_assignments = keepers
    return dropped
