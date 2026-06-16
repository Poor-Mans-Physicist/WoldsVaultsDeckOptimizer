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
  return opt.coreType
    .split("_")
    .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
    .join(" ");
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
    // Archive override replaces the per-arcane BASE (not the final mult). So
    // a user override of 1.5 → 1.5 ^ n_arcane_placed.
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

/** Numeric default rendered as placeholder text in the override input. */
export function coreDefaultPlaceholder(opt: CoreOption, cfg: ResolvedConfig): string {
  return coreDefaultValue(opt, cfg).toFixed(3);
}
