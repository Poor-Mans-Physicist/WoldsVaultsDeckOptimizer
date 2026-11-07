// Static list of core options in the picker (port of _CORE_OPTIONS in gui.py).
// The COLOR core gets one row per color (game allows one in inventory per color).

import { Color, CoreType, type CoreSpec, type GroupTag } from "./types";
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
  { coreType: CoreType.ARSENAL,     color: null },
  { coreType: CoreType.AEGIS,       color: null },
  { coreType: CoreType.TOOL,        color: null },
  { coreType: CoreType.NATURAL,     color: null },
  { coreType: CoreType.FAE,         color: null },
  { coreType: CoreType.BAZAAR,      color: null },
  { coreType: CoreType.TALENT,      color: null },
  { coreType: CoreType.TEMPORAL,    color: null },
  { coreType: CoreType.JUPITER,     color: null },
  { coreType: CoreType.PLUTO,       color: null },
  { coreType: CoreType.NITWIT,      color: null },
];

/** 0.34.1 addon cores scored through the kernel's implicit machinery (value always sent explicitly). */
export const ADDON_CORES: ReadonlySet<CoreType> = new Set([
  CoreType.ARSENAL, CoreType.AEGIS, CoreType.TOOL, CoreType.NATURAL, CoreType.FAE,
  CoreType.BAZAAR, CoreType.TALENT, CoreType.TEMPORAL,
  CoreType.JUPITER, CoreType.PLUTO, CoreType.NITWIT,
]);

/** Tag cores and the category group each one boosts. */
export const TAG_CORE_GROUP: Partial<Record<CoreType, GroupTag>> = {
  [CoreType.ARSENAL]: "Offensive",
  [CoreType.AEGIS]:   "Defensive",
  [CoreType.TOOL]:    "Utility",
  [CoreType.NATURAL]: "Physical",
  [CoreType.FAE]:     "Magical",
};

/** Cores with no stat effect: the player may reserve a slot for them; the optimizer never picks them. */
export const NONSTAT_CORE_OPTIONS: readonly CoreType[] = [
  CoreType.HARVEST, CoreType.FORTUNE, CoreType.BOUNTY, CoreType.ADEPT,
];

export const NONSTAT_CORE_INFO: Record<string, string> = {
  [CoreType.HARVEST]: "lowers resource card requirements",
  [CoreType.FORTUNE]: "chance for resource cards to double",
  [CoreType.BOUNTY]:  "raises crate tier with enough resource cards",
  [CoreType.ADEPT]:   "adds levels to one arcane slot",
};

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
    // Equilibrium stores the PER-COLOR roll (value = 1 + roll × unique
    // colors) — the item prints "+50% per unique deck color" → enter 50.
    case CoreType.EQUILIBRIUM:
    case CoreType.TALENT:
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
    case CoreType.ARSENAL:  return cfg.cores.arsenal;
    case CoreType.AEGIS:    return cfg.cores.aegis;
    case CoreType.TOOL:     return cfg.cores.tool;
    case CoreType.NATURAL:  return cfg.cores.natural;
    case CoreType.FAE:      return cfg.cores.fae;
    case CoreType.BAZAAR:   return cfg.cores.bazaar;
    case CoreType.TALENT:   return cfg.cores.talent;
    case CoreType.TEMPORAL: return cfg.cores.temporal;
    case CoreType.JUPITER:  return cfg.cores.jupiter;
    case CoreType.PLUTO:    return cfg.cores.pluto;
    case CoreType.NITWIT:   return cfg.cores.nitwit;
    case CoreType.HARVEST:
    case CoreType.FORTUNE:
    case CoreType.BOUNTY:
    case CoreType.ADEPT:
      return 0;
  }
}

/** The value the kernel receives for a core: the override, or for addon cores (which have no kernel
 *  default) the config value; -1 = the kernel's own config default. */
export function kernelCoreValue(spec: CoreSpec, cfg: ResolvedConfig): number {
  if (spec.override !== null) return spec.override;
  if (ADDON_CORES.has(spec.core_type)) return coreDefaultValue({ coreType: spec.core_type, color: null }, cfg);
  return -1.0;
}

/** Default rendered as placeholder text in the override input — in %, the
 *  same units the user types (e.g. Shiny 2.5 stored → "150"). */
export function coreDefaultPlaceholder(opt: CoreOption, cfg: ResolvedConfig): string {
  return String(storedToPct(coreDefaultValue(opt, cfg), coreValueKind(opt.coreType)));
}
