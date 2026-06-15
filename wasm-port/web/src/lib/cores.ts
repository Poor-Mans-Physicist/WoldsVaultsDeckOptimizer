// Core math + candidate-core enumeration (inventory-aware).
// Ports `_static_mult`, `_pure_mult`, `_deluxe_core_mult`, `_void_core_mult`,
// `_classify_cores`, and `candidate_cores_inventory` from
// src/inventory_optimize.py.

import { CardClass, CoreType, type Color, type CoreSpec } from "./types";
import type { ResolvedConfig } from "./config";

// ── Per-core multiplier lookups (override-aware) ─────────────────────────────

export function staticMult(spec: CoreSpec, cfg: ResolvedConfig): number {
  if (spec.override !== null) return spec.override;
  switch (spec.core_type) {
    case CoreType.EQUILIBRIUM: return cfg.cores.equilibrium;
    case CoreType.STEADFAST:   return cfg.cores.steadfast;
    case CoreType.COLOR:       return cfg.cores.color;
    case CoreType.FOIL:        return cfg.cores.foil;
    default:
      throw new Error(`staticMult called with non-static core ${spec.core_type}`);
  }
}

export function pureMult(spec: CoreSpec, n_ns: number, cfg: ResolvedConfig): number {
  const scale = spec.override !== null ? spec.override : cfg.cores.pure_scale;
  return cfg.cores.pure_base + scale * n_ns;
}

export function deluxeCoreMult(spec: CoreSpec, n_deluxe: number, cfg: ResolvedConfig): number {
  const scale = spec.override !== null ? spec.override : cfg.deluxe.core_scale;
  return cfg.deluxe.core_base + scale * n_deluxe;
}

/** VOID_CORE multiplier given runtime `n_dead`. `override` replaces scale. */
export function voidCoreMult(spec: CoreSpec, n_dead: number, cfg: ResolvedConfig): number {
  const scale = spec.override !== null ? spec.override : cfg.cores.void_scale;
  return cfg.cores.void_base + scale * n_dead;
}

/** ARCHIVE_CORE multiplier given runtime `n_arcane_placed`. `override`
 *  replaces the per-arcane *base*; final mult = `base ^ n_arcane_placed`. */
export function archiveCoreMult(spec: CoreSpec, n_arcane_placed: number, cfg: ResolvedConfig): number {
  const base = spec.override !== null ? spec.override : cfg.cores.archive_core;
  return Math.pow(base, n_arcane_placed);
}

// ── Classified-core output (for breakdown re-score) ───────────────────────────

export interface CoreComponent {
  core_type: CoreType;
  color:     Color | null;     // only set for COLOR cores
  value:     number;
  override:  boolean;
}

export interface ExcludedCore {
  core_type: CoreType;
  color:     Color | null;
  reason:    string;
}

/**
 * Mirror `_classify_cores`: split cores into baseline / color / deluxe / void
 * / archive / excluded buckets.
 *
 * `n_arcane_placed` feeds Archive Core's `base ^ n` formula. `n_dead` is used
 * by the void core. The old Pure-core fudge is gone — arcane cards count via
 * the assignment-derived `n_ns` instead.
 */
export function classifyCores(
  cores:      readonly CoreSpec[],
  card_class: CardClass,
  n_ns:       number,
  n_deluxe:   number,
  n_dead:     number,
  n_arcane_placed: number,
  cfg:        ResolvedConfig,
): {
  baseline:   CoreComponent[];
  colorComp:  CoreComponent | null;
  deluxeComp: CoreComponent | null;
  voidComp:   CoreComponent | null;
  archiveComp: CoreComponent | null;
  classExcluded: ExcludedCore[];
} {
  const baseline: CoreComponent[] = [];
  let colorComp:  CoreComponent | null = null;
  let deluxeComp: CoreComponent | null = null;
  let voidComp:   CoreComponent | null = null;
  let archiveComp: CoreComponent | null = null;
  const classExcluded: ExcludedCore[] = [];

  for (const spec of cores) {
    const isOverride = spec.override !== null;
    switch (spec.core_type) {
      case CoreType.PURE: {
        // n_ns already includes placed ARCANE; no fudge addend.
        const v = pureMult(spec, n_ns, cfg);
        baseline.push({ core_type: CoreType.PURE, color: null, value: v, override: isOverride });
        break;
      }
      case CoreType.EQUILIBRIUM:
        if (card_class === CardClass.SHINY) {
          baseline.push({ core_type: CoreType.EQUILIBRIUM, color: null, value: staticMult(spec, cfg), override: isOverride });
        } else {
          classExcluded.push({ core_type: CoreType.EQUILIBRIUM, color: null,
            reason: "equilibrium only applies to SHINY decks (this run is EVO)" });
        }
        break;
      case CoreType.STEADFAST:
        if (card_class === CardClass.SHINY) {
          baseline.push({ core_type: CoreType.STEADFAST, color: null, value: staticMult(spec, cfg), override: isOverride });
        } else {
          classExcluded.push({ core_type: CoreType.STEADFAST, color: null,
            reason: "steadfast only applies to SHINY decks (this run is EVO)" });
        }
        break;
      case CoreType.FOIL:
        baseline.push({ core_type: CoreType.FOIL, color: null, value: staticMult(spec, cfg), override: isOverride });
        break;
      case CoreType.COLOR:
        colorComp = { core_type: CoreType.COLOR, color: spec.color, value: staticMult(spec, cfg), override: isOverride };
        break;
      case CoreType.DELUXE_CORE: {
        const v = deluxeCoreMult(spec, n_deluxe, cfg);
        deluxeComp = { core_type: CoreType.DELUXE_CORE, color: null, value: v, override: isOverride };
        break;
      }
      case CoreType.VOID_CORE: {
        const v = voidCoreMult(spec, n_dead, cfg);
        voidComp = { core_type: CoreType.VOID_CORE, color: null, value: v, override: isOverride };
        break;
      }
      case CoreType.ARCHIVE_CORE: {
        const v = archiveCoreMult(spec, n_arcane_placed, cfg);
        archiveComp = { core_type: CoreType.ARCHIVE_CORE, color: null, value: v, override: isOverride };
        break;
      }
    }
  }
  return { baseline, colorComp, deluxeComp, voidComp, archiveComp, classExcluded };
}

// ── Set-like dedup for CoreSpec collections ──────────────────────────────────
// CoreSpec is a structural type; we key on (core_type|color|override) for set
// semantics. Frozensets aren't a thing in JS, so we hash to a string key.

export function coreSpecKey(s: CoreSpec): string {
  return `${s.core_type}|${s.color ?? ""}|${s.override === null ? "_" : s.override}`;
}

function comboKey(combo: readonly CoreSpec[]): string {
  return combo.map(coreSpecKey).sort().join(";");
}

// ── Candidate cores enumeration (inventory-aware) ────────────────────────────
//
// Port of candidate_cores_inventory() in src/inventory_optimize.py. Same
// SHINY / EVO grouping, color-core enumeration, and inventory restriction.

function combinations<T>(arr: readonly T[], size: number): T[][] {
  if (size === 0) return [[]];
  if (size > arr.length) return [];
  const out: T[][] = [];
  const recur = (start: number, acc: T[]) => {
    if (acc.length === size) { out.push(acc.slice()); return; }
    for (let i = start; i <= arr.length - (size - acc.length); i++) {
      acc.push(arr[i]);
      recur(i + 1, acc);
      acc.pop();
    }
  };
  recur(0, []);
  return out;
}

export function candidateCoresInventory(
  available:   readonly CoreSpec[],
  card_class:  CardClass,
  core_slots:  number,
  deck_n_slots: number,
  deck_n_arcane_slots: number,
  cfg:         ResolvedConfig,
): CoreSpec[][] {
  const k = core_slots;
  const byType = new Map<CoreType, CoreSpec[]>();
  for (const spec of available) {
    const bucket = byType.get(spec.core_type) ?? [];
    bucket.push(spec);
    byType.set(spec.core_type, bucket);
  }

  const pureSpec       = byType.get(CoreType.PURE)?.[0]        ?? null;
  const deluxeCoreSpec = byType.get(CoreType.DELUXE_CORE)?.[0] ?? null;
  // Void core is mode-gated. Vanilla disables it via cfg.cores.void_allow.
  let voidCoreSpec  = byType.get(CoreType.VOID_CORE)?.[0]  ?? null;
  if (!cfg.cores.void_allow) voidCoreSpec = null;
  // Archive core: mode-gated by cfg.cores.archive_allow (vanilla off), then
  // geometry-gated on the deck having any arcane slot (otherwise n_arcane_placed
  // is permanently 0 and the multiplier is permanently 1.0).
  let archiveCoreSpec = byType.get(CoreType.ARCHIVE_CORE)?.[0] ?? null;
  if (!cfg.cores.archive_allow)  archiveCoreSpec = null;
  if (deck_n_arcane_slots === 0) archiveCoreSpec = null;
  const foilSpec       = byType.get(CoreType.FOIL)?.[0]        ?? null;
  const equiSpec       = byType.get(CoreType.EQUILIBRIUM)?.[0] ?? null;
  const steadSpec      = byType.get(CoreType.STEADFAST)?.[0]   ?? null;
  const colorSpecs     = byType.get(CoreType.COLOR)            ?? [];

  const colorChoices: (CoreSpec | null)[] = [null, ...colorSpecs];

  const candidates: CoreSpec[][] = [];
  const seen = new Set<string>();
  const add = (combo: CoreSpec[]) => {
    const key = comboKey(combo);
    if (!seen.has(key)) { seen.add(key); candidates.push(combo); }
  };

  // Union-by-key (avoids duplicates when fillers re-pick color_pick).
  const unionUnique = (a: readonly CoreSpec[], b: readonly CoreSpec[]): CoreSpec[] => {
    const keys = new Set<string>();
    const out: CoreSpec[] = [];
    for (const x of [...a, ...b]) {
      const k = coreSpecKey(x);
      if (!keys.has(k)) { keys.add(k); out.push(x); }
    }
    return out;
  };

  // ── SHINY ───────────────────────────────────────────────────────────────
  if (card_class === CardClass.SHINY) {
    const nonVarStatic: CoreSpec[] = [equiSpec, steadSpec, foilSpec].filter((s): s is CoreSpec => s !== null);

    const bestShinyFillers = (slotsLeft: number, colorPick: CoreSpec | null): CoreSpec[] => {
      const pool: CoreSpec[] = [...nonVarStatic];
      if (colorPick !== null) pool.push(colorPick);
      const cap = Math.min(slotsLeft, pool.length);
      let bestM = 0.0;
      let bestC: CoreSpec[] = [];
      for (let size = 0; size <= cap; size++) {
        for (const combo of combinations(pool, size)) {
          let m = 1.0;
          for (const c of combo) m *= staticMult(c, cfg);
          if (m > bestM) { bestM = m; bestC = combo; }
        }
      }
      return bestC;
    };

    const varPool: CoreSpec[] = [];
    if (pureSpec)        varPool.push(pureSpec);
    if (deluxeCoreSpec)  varPool.push(deluxeCoreSpec);
    if (voidCoreSpec)    varPool.push(voidCoreSpec);
    if (archiveCoreSpec) varPool.push(archiveCoreSpec);

    for (const colorPick of colorChoices) {
      for (let size = 0; size <= varPool.length; size++) {
        for (const varCombo of combinations(varPool, size)) {
          const pre: CoreSpec[] = [...varCombo];
          if (colorPick !== null) pre.push(colorPick);
          const preKeys = new Set(pre.map(coreSpecKey));
          if (preKeys.size > k) continue;
          const fillers = bestShinyFillers(k - preKeys.size, colorPick);
          const combo = unionUnique(pre, fillers);
          add(combo);
        }
      }
    }
    return candidates;
  }

  // ── EVO ─────────────────────────────────────────────────────────────────
  // deck.slots already includes arcane positions under the new model; the
  // old `+ n_arcane` fudge is gone.
  const n_ns_full = deck_n_slots;

  const evoNoFoilStaticMult = (spec: CoreSpec): number => {
    if (spec.core_type === CoreType.PURE) return pureMult(spec, n_ns_full, cfg);
    return staticMult(spec, cfg);
  };

  const bestFixedEvoNoFoil = (slotsLeft: number, colorPick: CoreSpec | null): CoreSpec[] => {
    const pool: CoreSpec[] = [];
    if (pureSpec)   pool.push(pureSpec);
    if (colorPick)  pool.push(colorPick);
    const cap = Math.min(slotsLeft, pool.length);
    let bestM = -1.0;
    let bestC: CoreSpec[] = [];
    for (let size = 0; size <= cap; size++) {
      for (const combo of combinations(pool, size)) {
        let m = 1.0;
        for (const c of combo) m *= evoNoFoilStaticMult(c);
        if (m > bestM) { bestM = m; bestC = combo; }
      }
    }
    return bestC;
  };

  const bestFixedEvoWithFoil = (slotsLeft: number, colorPick: CoreSpec | null): CoreSpec[] => {
    if (slotsLeft >= 1 && colorPick !== null && staticMult(colorPick, cfg) > 1.0) {
      return [colorPick];
    }
    return [];
  };

  const deluxeVar: CoreSpec[] = deluxeCoreSpec ? [deluxeCoreSpec] : [];
  // Void joins the EVO variable pool too (its multiplier depends on n_dead,
  // unknown pre-SA). Gated by cfg.cores.void_allow above.
  const voidVar:   CoreSpec[] = voidCoreSpec ? [voidCoreSpec] : [];
  // Archive joins both EVO groups when the deck has any arcane slots.
  const archiveVar: CoreSpec[] = archiveCoreSpec ? [archiveCoreSpec] : [];

  // Group A: no FOIL (PURE is static)
  const varPoolA: CoreSpec[] = [...deluxeVar, ...voidVar, ...archiveVar];
  for (const colorPick of colorChoices) {
    for (let size = 0; size <= varPoolA.length; size++) {
      for (const varCombo of combinations(varPoolA, size)) {
        const pre: CoreSpec[] = [...varCombo];
        if (colorPick !== null) pre.push(colorPick);
        const preKeys = new Set(pre.map(coreSpecKey));
        if (preKeys.size > k) continue;
        const fillers = bestFixedEvoNoFoil(k - preKeys.size, colorPick);
        const combo = unionUnique(pre, fillers);
        add(combo);
      }
    }
  }

  // Group B: with FOIL (PURE is variable)
  if (foilSpec) {
    const varPoolB: CoreSpec[] = [];
    if (pureSpec)        varPoolB.push(pureSpec);
    if (deluxeCoreSpec)  varPoolB.push(deluxeCoreSpec);
    if (voidCoreSpec)    varPoolB.push(voidCoreSpec);
    if (archiveCoreSpec) varPoolB.push(archiveCoreSpec);

    for (const colorPick of colorChoices) {
      for (let size = 0; size <= varPoolB.length; size++) {
        for (const varCombo of combinations(varPoolB, size)) {
          const pre: CoreSpec[] = [...varCombo, foilSpec];
          if (colorPick !== null) pre.push(colorPick);
          const preKeys = new Set(pre.map(coreSpecKey));
          if (preKeys.size > k) continue;
          const fillers = bestFixedEvoWithFoil(k - preKeys.size, colorPick);
          const combo = unionUnique(pre, fillers);
          add(combo);
        }
      }
    }
  }

  return candidates;
}
