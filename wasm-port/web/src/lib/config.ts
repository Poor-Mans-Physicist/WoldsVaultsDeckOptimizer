// Resolved-config loader. Reads the build-time bundle `public/config.json`
// (emitted by `scripts/build_data.py`) and exposes a typed view per mode.
//
// Mirrors the UPPERCASE constants in src/config.py — these are what the
// candidate-core enumerator and breakdown re-score need.

export interface ResolvedConfig {
  deckmod: number;
  excluded_decks: string[];

  greed: {
    dir_vert: number;
    dir_horiz: number;
    evo: number;
    surr: number;
    dir_diag_up: number;
    dir_diag_down: number;
  };

  cores: {
    pure_base: number;
    pure_scale: number;
    equilibrium: number;
    foil: number;
    steadfast: number;
    color: number;
    // Void core: variable multiplier (base + scale × n_dead). `void_allow`
    // gates whether the candidate enumerator considers it. Vanilla sets false.
    void_allow: boolean;
    void_base: number;
    void_scale: number;
    // Archive core: per-arcane base. Final mult = `archive_core ^ n_arcane_placed`.
    // Applied OUTSIDE the per-card core_mult (bypasses the additive_cores
    // switch). Only enumerated when the deck has any arcane slot.
    archive_core: number;
  };

  deluxe: {
    allow: boolean;
    flat: number;
    core_base: number;
    core_scale: number;
  };

  // Arcane behavior. `auto_place: true` pre-fills arcane slots with ARCANE
  // and locks them (SA may still pick color); `false` lets SA swap to DEAD.
  arcane: {
    auto_place: boolean;
  };

  // Each mode's chosen JSON deck file (read by build_data.py; the resolved
  // value is exposed here for completeness — the browser doesn't re-read it).
  decks: {
    json_file: string;
  };

  stacking: {
    greed_additive: boolean;
    additive_cores: boolean;
  };

  shiny: {
    positional: boolean;
  };

  constraints: {
    deluxe_counted_as_regular: boolean;
  };
}

export interface ConfigBundle {
  default_mode: string;
  modes: Record<string, ResolvedConfig>;
}

let _bundle: ConfigBundle | null = null;

/** Fetch and cache the build-time config bundle. */
export async function loadConfigBundle(baseUrl: string): Promise<ConfigBundle> {
  if (_bundle) return _bundle;
  // `no-cache`: revalidate with the server (ETag / If-Modified-Since) every
  // load. GitHub Pages serves long max-age on /config.json otherwise, so
  // Safari happily holds onto a stale copy that's missing new fields. The
  // 304 path makes the overhead negligible when nothing has changed.
  const res = await fetch(`${baseUrl}config.json`, { cache: "no-cache" });
  if (!res.ok) throw new Error(`Failed to load config.json: ${res.status}`);
  _bundle = await res.json();
  return _bundle!;
}

/** Pick one mode's resolved config; throws if unknown. */
export function getMode(bundle: ConfigBundle, mode: string): ResolvedConfig {
  const cfg = bundle.modes[mode];
  if (!cfg) throw new Error(`Unknown mode '${mode}' (have: ${Object.keys(bundle.modes).join(", ")})`);
  return cfg;
}
