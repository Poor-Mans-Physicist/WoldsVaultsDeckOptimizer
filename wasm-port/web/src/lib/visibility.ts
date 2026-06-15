// Mode-aware option visibility. Hides UI controls that don't apply to the
// current (mode, card_class) combo. Mirrors `_hidden_inventory_types`,
// `_hidden_core_types`, and `_class_select_options` from src/gui.py.

import { CardClass, CardType, CoreType } from "./types";
import type { ResolvedConfig } from "./config";

/**
 * Inventory rows that should be hidden in the current setup.
 *
 * Vanilla drops Wold's-only mechanics (evo/surr greed, deluxe cards) and —
 * when the active class is "stat" (SHINY relabel) — also drops the positional
 * types (a vanilla shiny deck is purely typeless).
 */
export function hiddenInventoryTypes(
  mode:       string,
  cardClass:  CardClass,
  cfg:        ResolvedConfig,
): Set<CardType> {
  const hidden = new Set<CardType>();
  // Deluxe is always config-gated; vanilla disables it via cfg.deluxe.allow.
  if (!cfg.deluxe.allow) hidden.add(CardType.DELUXE);
  if (mode === "vanilla") {
    hidden.add(CardType.EVO_GREED);
    hidden.add(CardType.SURR_GREED);
    if (cardClass === CardClass.SHINY) {
      hidden.add(CardType.ROW);
      hidden.add(CardType.COL);
      hidden.add(CardType.SURR);
      hidden.add(CardType.DIAG);
    }
  }
  return hidden;
}

/**
 * Core picker rows that should be hidden in the current mode. Vanilla
 * disables deluxe / void cores via cfg flags. `_mode` is unused
 * today (every gate is cfg-derived) but kept in the signature so callers
 * don't need to reshape arguments if we ever add a mode-only rule.
 */
export function hiddenCoreTypes(_mode: string, cfg: ResolvedConfig): Set<CoreType> {
  const hidden = new Set<CoreType>();
  if (!cfg.deluxe.allow)     hidden.add(CoreType.DELUXE_CORE);
  if (!cfg.cores.void_allow) hidden.add(CoreType.VOID_CORE);
  return hidden;
}

/**
 * Display label for one CardClass option. Vanilla relabels SHINY → "Stat"
 * since vanilla shiny decks are pure-typeless / stat-card decks rather than
 * positional decks.
 */
export function classSelectLabel(mode: string, c: CardClass): string {
  if (c === CardClass.SHINY) return mode === "vanilla" ? "Stat" : "Shiny";
  return "Evolution";
}
