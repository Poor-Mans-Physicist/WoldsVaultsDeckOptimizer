// Static list of core options in the picker (port of _CORE_OPTIONS in gui.py).
// The COLOR core gets one row per color (game allows one in inventory per color).

import { Color, CoreType } from "./types";
import type { ResolvedConfig } from "./config";

export interface CoreOption {
  coreType: CoreType;
  color:    Color | null;
}

export const CORE_OPTIONS: readonly CoreOption[] = [
  { coreType: CoreType.PURE,        color: null },
  { coreType: CoreType.EQUILIBRIUM, color: null },
  { coreType: CoreType.STEADFAST,   color: null },
  { coreType: CoreType.SPARKLING,   color: null },
  { coreType: CoreType.FOIL,        color: null },
  { coreType: CoreType.DELUXE_CORE, color: null },
  { coreType: CoreType.VOID_CORE,   color: null },
  { coreType: CoreType.ARCHIVE_CORE, color: null },
  { coreType: CoreType.COLOR,       color: Color.RED },
  { coreType: CoreType.COLOR,       color: Color.GREEN },
  { coreType: CoreType.COLOR,       color: Color.BLUE },
  { coreType: CoreType.COLOR,       color: Color.YELLOW },
];

export function coreLabel(opt: CoreOption): string {
  if (opt.coreType === CoreType.COLOR && opt.color !== null) {
    return `Color · ${opt.color.charAt(0).toUpperCase() + opt.color.slice(1)}`;
  }
  // In-game this core is the SHINY core (it boosts Foil-group cards; the
  // separate Sparkling core is the one that boosts shiny cards). Display
  // follows the game; `foil` stays the internal key (config / kernel /
  // snapshots) everywhere.
  if (opt.coreType === CoreType.FOIL) return "Shiny";
  return opt.coreType
    .split("_")
    .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
    .join(" ");
}

/** Display form of a raw `core_type` key (result chip etc.) — identity except
 *  the foil→Shiny rename above. */
export function coreKeyDisplay(coreType: string): string {
  return coreType === CoreType.FOIL ? "shiny" : coreType;
}

/**
 * How a core's number is entered and displayed (always as a % — matching the
 * in-game item tooltip), while config / kernel / snapshots keep the raw
 * stored units:
 *  - "increment": stored value IS the per-unit bonus fraction (the scale term
 *    of Pure / Deluxe Core / Void). Stored 0.063 ⇄ entered 6.3 (%).
 *  - "flat": stored value is a whole multiplier; the item shows the bonus
 *    above 1. Stored 2.5 ⇄ entered 150 (%). Archive's per-arcane base is the
 *    same shape (stored 1.2 ⇄ 20% per arcane card).
 */
export type CoreValueKind = "increment" | "flat";

export function coreValueKind(coreType: CoreType): CoreValueKind {
  switch (coreType) {
    case CoreType.PURE:
    case CoreType.DELUXE_CORE:
    case CoreType.VOID_CORE:
      return "increment";
    default:
      return "flat";
  }
}

/** Stored units → the % shown in the input (float dust stripped so
 *  0.07 renders as 7, not 7.000000000000001). */
export function storedToPct(v: number, kind: CoreValueKind): number {
  const pct = kind === "increment" ? v * 100 : (v - 1) * 100;
  return Number(pct.toFixed(6));
}

/** The % the user typed → stored units. */
export function pctToStored(pct: number, kind: CoreValueKind): number {
  return kind === "increment" ? pct / 100 : 1 + pct / 100;
}

/**
 * Numeric default the optimizer would use when the user leaves the override
 * field blank. For PURE / DELUXE_CORE / VOID_CORE the override replaces only
 * the scale term (formula stays `base + scale × n`), so we surface the scale.
 * Static cores (EQUI / STEAD / FOIL / COLOR) surface their multiplier.
 */
export function coreDefaultValue(opt: CoreOption, cfg: ResolvedConfig): number {
  switch (opt.coreType) {
    case CoreType.PURE:        return cfg.cores.pure_scale;
    case CoreType.DELUXE_CORE: return cfg.deluxe.core_scale;
    case CoreType.VOID_CORE:   return cfg.cores.void_scale;
    case CoreType.EQUILIBRIUM: return cfg.cores.equilibrium;
    case CoreType.STEADFAST:   return cfg.cores.steadfast;
    case CoreType.SPARKLING:   return cfg.cores.sparkling ?? 2.5;
    case CoreType.FOIL:        return cfg.cores.foil;
    case CoreType.COLOR:       return cfg.cores.color;
    // Archive override replaces the rolled base (1 + v), not the final value:
    // per-card stack term = base ^ n_arcane_placed — the live game formula.
    // Fallback `?? 1.2` defends against a stale cached `config.json` from
    // before Archive Core shipped — without it `undefined.toFixed(3)` in
    // coreDefaultPlaceholder would brick the entire page on first render.
    case CoreType.ARCHIVE_CORE: return cfg.cores.archive_core ?? 1.2;
    // Structural cores never appear in CORE_OPTIONS (they have their own UI
    // subsection in CorePicker, not a multiplier slider). Return 0 just to
    // keep the switch exhaustive — call sites never hit these branches.
    case CoreType.CONSTRUCTION_CORE: return 0;
    case CoreType.ARCANE_CORE:       return 0;
  }
}

/** Default rendered as placeholder text in the override input — in %, the
 *  same units the user types (e.g. Shiny 2.5 stored → "150"). */
export function coreDefaultPlaceholder(opt: CoreOption, cfg: ResolvedConfig): string {
  return String(storedToPct(coreDefaultValue(opt, cfg), coreValueKind(opt.coreType)));
}
