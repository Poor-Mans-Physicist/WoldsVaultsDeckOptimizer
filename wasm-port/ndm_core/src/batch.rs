//! Legacy batch optimizer — PyO3-only.
//!
//! Powers the CLI ``uv run optimize`` workflow (`src.simulate.sa_optimize`).
//! Color-blind, supports min_regular / max_greed constraints and the
//! shiny_positional / experimental-exponent flags. The wasm build does NOT
//! include this module; the browser only ships the inventory optimizer.

use pyo3::prelude::*;
use rand::prelude::*;
use rand::rngs::SmallRng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Card type constants (u8)
// Must match the CardType enum values in the Python code.
// To add a new card type: add a constant here, add an arm to is_greed_type()
// or is_regular_type(), add an arm to the greed loop in simulate(), and add
// a branch to the accumulation loop if it's a new scoring category.
// ─────────────────────────────────────────────────────────────────────────────

const ROW: u8            = 0;
const COL: u8            = 1;
const SURR: u8           = 2;
const DIAG: u8           = 3;
const DELUXE: u8         = 4;
const TYPELESS: u8       = 5;
const DIR_GREED_UP: u8   = 6;
const DIR_GREED_DOWN: u8 = 7;
const DIR_GREED_LEFT: u8 = 8;
const DIR_GREED_RIGHT: u8= 9;
const DIR_GREED_NE: u8   = 10;
const DIR_GREED_NW: u8   = 11;
const DIR_GREED_SE: u8   = 12;
const DIR_GREED_SW: u8   = 13;
const EVO_GREED: u8      = 14;
const SURR_GREED: u8     = 15;
const FILLER_GREED: u8   = 16;  // display-only, never placed

// Core type constants (u8)
const CORE_PURE: u8        = 0;
const CORE_EQUILIBRIUM: u8 = 1;
const CORE_STEADFAST: u8   = 2;
const CORE_COLOR: u8       = 3;
const CORE_FOIL: u8        = 4;
const CORE_DELUXE: u8      = 5;

// ─────────────────────────────────────────────────────────────────────────────
// String ↔ u8 conversions (used only at the Python boundary, not in hot path)
// ─────────────────────────────────────────────────────────────────────────────

fn card_type_from_str(s: &str) -> u8 {
    match s {
        "row"             => ROW,
        "col"             => COL,
        "surr"            => SURR,
        "diag"            => DIAG,
        "deluxe"          => DELUXE,
        "typeless"        => TYPELESS,
        "dir_greed_up"    => DIR_GREED_UP,
        "dir_greed_down"  => DIR_GREED_DOWN,
        "dir_greed_left"  => DIR_GREED_LEFT,
        "dir_greed_right" => DIR_GREED_RIGHT,
        "dir_greed_ne"    => DIR_GREED_NE,
        "dir_greed_nw"    => DIR_GREED_NW,
        "dir_greed_se"    => DIR_GREED_SE,
        "dir_greed_sw"    => DIR_GREED_SW,
        "evo_greed"       => EVO_GREED,
        "surr_greed"      => SURR_GREED,
        "filler_greed"    => FILLER_GREED,
        other             => panic!("Unknown card type string: {}", other),
    }
}

fn card_type_to_str(t: u8) -> &'static str {
    match t {
        ROW            => "row",
        COL            => "col",
        SURR           => "surr",
        DIAG           => "diag",
        DELUXE         => "deluxe",
        TYPELESS       => "typeless",
        DIR_GREED_UP   => "dir_greed_up",
        DIR_GREED_DOWN => "dir_greed_down",
        DIR_GREED_LEFT => "dir_greed_left",
        DIR_GREED_RIGHT=> "dir_greed_right",
        DIR_GREED_NE   => "dir_greed_ne",
        DIR_GREED_NW   => "dir_greed_nw",
        DIR_GREED_SE   => "dir_greed_se",
        DIR_GREED_SW   => "dir_greed_sw",
        EVO_GREED      => "evo_greed",
        SURR_GREED     => "surr_greed",
        FILLER_GREED   => "filler_greed",
        other          => panic!("Unknown card type u8: {}", other),
    }
}

fn core_from_str(s: &str) -> u8 {
    match s {
        "pure"        => CORE_PURE,
        "equilibrium" => CORE_EQUILIBRIUM,
        "steadfast"   => CORE_STEADFAST,
        "color"       => CORE_COLOR,
        "foil"        => CORE_FOIL,
        "deluxe_core" => CORE_DELUXE,
        other         => panic!("Unknown core type string: {}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Card category predicates (inlined for hot-path speed)
// ─────────────────────────────────────────────────────────────────────────────

#[inline(always)]
fn is_greed_type(t: u8) -> bool {
    matches!(
        t,
        DIR_GREED_UP | DIR_GREED_DOWN | DIR_GREED_LEFT | DIR_GREED_RIGHT
        | DIR_GREED_NE | DIR_GREED_NW | DIR_GREED_SE | DIR_GREED_SW
        | EVO_GREED | SURR_GREED | FILLER_GREED
    )
}

#[inline(always)]
fn is_regular_type(t: u8) -> bool {
    matches!(t, ROW | COL | SURR | DIAG)
}

// ─────────────────────────────────────────────────────────────────────────────
// Deck geometry (precomputed, passed in from Python — never recomputed in Rust)
// ─────────────────────────────────────────────────────────────────────────────

struct DeckData {
    n: usize,
    row_of: Vec<i32>,    // row coordinate of slot i
    col_of: Vec<i32>,    // col coordinate of slot i
    row_peers: Vec<Vec<usize>>,  // indices of same-row peers (excludes self)
    col_peers: Vec<Vec<usize>>,  // indices of same-col peers (excludes self)
    surr_peers: Vec<Vec<usize>>, // indices of Chebyshev-1 peers (excludes self)
    diag_peers: Vec<Vec<usize>>, // indices of diagonal peers (excludes self)
    // Precomputed directional neighbours: None if that direction is not a deck slot
    dir_up: Vec<Option<usize>>,
    dir_down: Vec<Option<usize>>,
    dir_left: Vec<Option<usize>>,
    dir_right: Vec<Option<usize>>,
    dir_ne: Vec<Option<usize>>,
    dir_nw: Vec<Option<usize>>,
    dir_se: Vec<Option<usize>>,
    dir_sw: Vec<Option<usize>>,
    // Geometry-optimal positional type per slot (precomputed from peer set sizes)
    best_positional: Vec<u8>,
    // Deck parameters
    n_arcane: usize,
    min_regular: i32,
    max_greed: i32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Simulation config (all multipliers and flags, passed from Python constants)
// ─────────────────────────────────────────────────────────────────────────────

struct SimConfig {
    mult_dir_vert: f64,
    mult_dir_horiz: f64,
    mult_evo_greed: f64,
    mult_surr_greed: f64,
    mult_dir_diag_up: f64,
    mult_dir_diag_down: f64,
    mult_pure_base: f64,
    mult_pure_scale: f64,
    mult_equilibrium: f64,
    mult_foil: f64,
    mult_steadfast: f64,
    mult_color: f64,
    mult_deluxe_flat: f64,
    mult_deluxe_core_base: f64,
    mult_deluxe_core_scale: f64,
    greed_additive: bool,
    additive_cores: bool,
    is_shiny: bool,
    foil_active: bool,     // FOIL is in cores for this run
    shiny_positional: bool,
    enable_experimental: bool,
    experimental_exponent: f64,
    experimental_boost: f64,
    deluxe_counted_as_regular: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Core simulation — mirrors Python simulate() exactly
// ─────────────────────────────────────────────────────────────────────────────

fn simulate(deck: &DeckData, asgn: &[u8], cores: &[u8], cfg: &SimConfig) -> f64 {
    let n = deck.n;

    // ── Partition slots into categories ──────────────────────────────────────
    let mut is_greed   = vec![false; n];
    let mut is_reg     = vec![false; n];
    let mut is_deluxe  = vec![false; n];
    let mut is_typeless= vec![false; n];
    let mut is_scorable= vec![false; n]; // reg | deluxe | typeless
    let mut is_filled  = vec![false; n]; // greed | reg | deluxe | typeless
    let mut n_greed: usize = 0;
    let mut n_regular: usize = 0;
    let mut n_deluxe: usize = 0;

    for i in 0..n {
        match asgn[i] {
            t if is_greed_type(t) => {
                is_greed[i]  = true;
                is_filled[i] = true;
                n_greed += 1;
            }
            t if is_regular_type(t) => {
                is_reg[i]     = true;
                is_scorable[i]= true;
                is_filled[i]  = true;
                n_regular += 1;
            }
            DELUXE => {
                is_deluxe[i]  = true;
                is_scorable[i]= true;
                is_filled[i]  = true;
                n_deluxe += 1;
            }
            TYPELESS => {
                is_typeless[i]= true;
                is_scorable[i]= true;
                is_filled[i]  = true;
            }
            _ => {}
        }
    }

    // ── n_ns: non-shiny count for Pure Core ──────────────────────────────────
    // EVO without FOIL: n_ns = regular + greed
    // EVO with FOIL:    n_ns = greed only
    // SHINY:            n_ns = greed only (regular cards are always shiny)
    let n_ns: usize = if cfg.is_shiny || cfg.foil_active {
        n_greed
    } else {
        n_regular + n_greed
    };

    // ── Core multipliers ──────────────────────────────────────────────────────
    // All cores fold into a single core_mult. DELUXE_CORE is gated per-card:
    // it applies to regulars and typeless cards but NOT to deluxes. So we
    // compute a baseline (everything except deluxe core) and an addend / factor
    // for the deluxe core, then build the two card-class core_mult variants.
    let mut baseline_c: Vec<f64> = Vec::with_capacity(5);
    let mut deluxe_core_value: Option<f64> = None;

    for &core in cores {
        match core {
            CORE_PURE => {
                baseline_c.push(cfg.mult_pure_base + cfg.mult_pure_scale * (n_ns + deck.n_arcane) as f64);
            }
            CORE_EQUILIBRIUM if cfg.is_shiny => { baseline_c.push(cfg.mult_equilibrium); }
            CORE_STEADFAST   if cfg.is_shiny => { baseline_c.push(cfg.mult_steadfast); }
            CORE_COLOR  => { baseline_c.push(cfg.mult_color); }
            CORE_FOIL   => { baseline_c.push(cfg.mult_foil); }
            CORE_DELUXE => {
                deluxe_core_value = Some(
                    cfg.mult_deluxe_core_base + cfg.mult_deluxe_core_scale * n_deluxe as f64,
                );
            }
            _ => {}
        }
    }

    let (non_deluxe_core_mult, deluxe_card_core_mult) = if cfg.additive_cores {
        let baseline_sum: f64 = baseline_c.iter().map(|v| v - 1.0).sum();
        let deluxe_addend: f64 = deluxe_core_value.map_or(0.0, |v| v - 1.0);
        (
            1.0 + baseline_sum + deluxe_addend, // non-deluxe (regular + typeless)
            1.0 + baseline_sum,                  // deluxe cards
        )
    } else {
        let baseline_prod: f64 = if baseline_c.is_empty() { 1.0 } else { baseline_c.iter().product() };
        let deluxe_factor: f64 = deluxe_core_value.unwrap_or(1.0);
        (
            baseline_prod * deluxe_factor,
            baseline_prod,
        )
    };

    // ── Row / col counts for positional multipliers ───────────────────────────
    // Counts ALL filled cards (including greed) — same as Python.
    let mut row_count: HashMap<i32, usize> = HashMap::new();
    let mut col_count: HashMap<i32, usize> = HashMap::new();
    for i in 0..n {
        if is_filled[i] {
            *row_count.entry(deck.row_of[i]).or_insert(0) += 1;
            *col_count.entry(deck.col_of[i]).or_insert(0) += 1;
        }
    }

    // ── Greed boosts ──────────────────────────────────────────────────────────
    // init = 1.0 for both additive and multiplicative modes.
    // Additive:       boost[j] += (amount - 1.0)
    // Multiplicative: boost[j] *= amount
    let mut boost = vec![1.0f64; n];

    macro_rules! apply_greed {
        ($target:expr, $amount:expr) => {
            if cfg.greed_additive {
                boost[$target] += $amount - 1.0;
            } else {
                boost[$target] *= $amount;
            }
        };
    }

    for i in 0..n {
        if !is_greed[i] { continue; }
        match asgn[i] {
            DIR_GREED_UP => {
                if let Some(j) = deck.dir_up[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_dir_vert); }
                }
            }
            DIR_GREED_DOWN => {
                if let Some(j) = deck.dir_down[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_dir_vert); }
                }
            }
            DIR_GREED_LEFT => {
                if let Some(j) = deck.dir_left[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_dir_horiz); }
                }
            }
            DIR_GREED_RIGHT => {
                if let Some(j) = deck.dir_right[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_dir_horiz); }
                }
            }
            DIR_GREED_NE => {
                if let Some(j) = deck.dir_ne[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_dir_diag_up); }
                }
            }
            DIR_GREED_NW => {
                if let Some(j) = deck.dir_nw[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_dir_diag_up); }
                }
            }
            DIR_GREED_SE => {
                if let Some(j) = deck.dir_se[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_dir_diag_down); }
                }
            }
            DIR_GREED_SW => {
                if let Some(j) = deck.dir_sw[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_dir_diag_down); }
                }
            }
            EVO_GREED => {
                // Only buffs REGULAR EVO cards below it — not deluxe or typeless
                if !cfg.is_shiny {
                    if let Some(j) = deck.dir_down[i] {
                        if is_reg[j] { apply_greed!(j, cfg.mult_evo_greed); }
                    }
                }
            }
            SURR_GREED => {
                for &j in &deck.surr_peers[i] {
                    if is_scorable[j] { apply_greed!(j, cfg.mult_surr_greed); }
                }
            }
            _ => {} // FILLER_GREED and other greed types do nothing
        }
    }

    // ── NDM accumulation ──────────────────────────────────────────────────────
    let contrib = |val: f64| -> f64 {
        if cfg.enable_experimental {
            (val * cfg.experimental_boost).powf(cfg.experimental_exponent)
        } else {
            val
        }
    };

    let mut ndm = 0.0f64;

    for i in 0..n {
        if is_reg[i] {
            let pos: usize = match asgn[i] {
                ROW  => *row_count.get(&deck.row_of[i]).unwrap_or(&0),
                COL  => *col_count.get(&deck.col_of[i]).unwrap_or(&0),
                DIAG => deck.diag_peers[i].iter().filter(|&&j| is_filled[j]).count() + 1,
                SURR => deck.surr_peers[i].iter().filter(|&&j| is_filled[j]).count(),
                _    => 0,
            };
            let b = if cfg.greed_additive { boost[i].max(1.0) } else { boost[i] };
            ndm += contrib(pos as f64 * non_deluxe_core_mult * b);
        } else if is_deluxe[i] {
            let b = if cfg.greed_additive { boost[i].max(1.0) } else { boost[i] };
            ndm += contrib(cfg.mult_deluxe_flat * deluxe_card_core_mult * b);
        } else if is_typeless[i] {
            let b = if cfg.greed_additive { boost[i].max(1.0) } else { boost[i] };
            ndm += contrib(1.0_f64 * non_deluxe_core_mult * b);
        }
    }

    ndm
}

// ─────────────────────────────────────────────────────────────────────────────
// Simulated annealing — mirrors Python sa_optimize() exactly
// ─────────────────────────────────────────────────────────────────────────────

fn sa_optimize_inner(
    deck: &DeckData,
    cores: &[u8],
    placeable: &[u8],
    cfg: &SimConfig,
    n_iter: usize,
    t_start: f64,
    t_end: f64,
) -> (Vec<u8>, f64) {
    let n = deck.n;
    let mut rng = SmallRng::from_entropy();

    // Default starting card type
    let default_t: u8 = if cfg.is_shiny && !cfg.shiny_positional {
        TYPELESS
    } else {
        SURR
    };

    // Initialise assignment
    let mut asgn: Vec<u8> = if deck.min_regular > 0 && (deck.min_regular as usize) < n {
        // Seed with min_regular default_t cards, rest SURR_GREED
        let mut indices: Vec<usize> = (0..n).collect();
        indices.shuffle(&mut rng);
        let mut a = vec![SURR_GREED; n];
        for &i in indices.iter().take(deck.min_regular as usize) {
            a[i] = default_t;
        }
        a
    } else {
        vec![default_t; n]
    };

    // Apply best_positional to initial regular cards
    for i in 0..n {
        if is_regular_type(asgn[i]) {
            asgn[i] = deck.best_positional[i];
        }
    }

    let mut score = simulate(deck, &asgn, cores, cfg);
    let mut best_score = score;
    let mut best_asgn = asgn.clone();

    // Cooling schedule
    let log_cool = (t_end / t_start).ln();

    // O(1) constraint counters
    let is_scoring = |t: u8| -> bool {
        is_regular_type(t) || t == TYPELESS || (cfg.deluxe_counted_as_regular && t == DELUXE)
    };

    let mut n_greed_cur: i32 = asgn.iter().filter(|&&t| is_greed_type(t)).count() as i32;
    let mut n_reg_cur:   i32 = asgn.iter().filter(|&&t| is_scoring(t)).count() as i32;

    // Whether min_regular constraint is actually active (may be overridden by max_greed conflict)
    let min_reg_active: bool = deck.min_regular >= 0
        && !(deck.max_greed >= 0
             && deck.min_regular + deck.max_greed > n as i32);

    let valid = |ng: i32, nr: i32| -> bool {
        if deck.max_greed >= 0 && ng > deck.max_greed  { return false; }
        if min_reg_active      && nr < deck.min_regular { return false; }
        true
    };

    // Resolve: if a positional type is chosen, redirect to the geometry-optimal one
    let resolve = |slot: usize, t: u8| -> u8 {
        if is_regular_type(t) { deck.best_positional[slot] } else { t }
    };

    for iter in 0..n_iter {
        let temp = {
            let raw = t_start * (log_cool * iter as f64 / n_iter as f64).exp();
            if raw < 1e-10 { 1e-10 } else { raw }
        };

        if n < 2 || rng.gen::<f64>() < 0.80 {
            // ── Move: change one slot ─────────────────────────────────────
            let slot = rng.gen_range(0..n);
            let old  = asgn[slot];
            let new  = resolve(slot, placeable[rng.gen_range(0..placeable.len())]);
            if new == old { continue; }

            // Incremental counter update
            let dg: i32 = is_greed_type(new) as i32 - is_greed_type(old) as i32;
            let dr: i32 = is_scoring(new)    as i32 - is_scoring(old)    as i32;
            let new_ng  = n_greed_cur + dg;
            let new_nr  = n_reg_cur   + dr;

            if !valid(new_ng, new_nr) { continue; }

            asgn[slot]   = new;
            n_greed_cur  = new_ng;
            n_reg_cur    = new_nr;

            let new_score = simulate(deck, &asgn, cores, cfg);
            let delta     = new_score - score;

            if delta >= 0.0 || rng.gen::<f64>() < (delta / temp).exp() {
                score = new_score;
                if score > best_score {
                    best_score = score;
                    best_asgn  = asgn.clone();
                }
            } else {
                // Revert
                asgn[slot]  = old;
                n_greed_cur -= dg;
                n_reg_cur   -= dr;
            }
        } else {
            // ── Move: swap two slots ──────────────────────────────────────
            // Swapping never changes constraint counts (totals are preserved),
            // so we skip the validity check entirely.
            let i1 = rng.gen_range(0..n);
            let i2 = {
                let raw = rng.gen_range(0..n - 1);
                if raw >= i1 { raw + 1 } else { raw }
            };
            if asgn[i1] == asgn[i2] { continue; }

            asgn.swap(i1, i2);

            let new_score = simulate(deck, &asgn, cores, cfg);
            let delta     = new_score - score;

            if delta >= 0.0 || rng.gen::<f64>() < (delta / temp).exp() {
                score = new_score;
                if score > best_score {
                    best_score = score;
                    best_asgn  = asgn.clone();
                }
            } else {
                asgn.swap(i1, i2); // revert
            }
        }
    }

    (best_asgn, best_score)
}

// ─────────────────────────────────────────────────────────────────────────────
// PyO3 entry point — called from Python sa_optimize()
//
// Peer sets are passed as index arrays (slot indices, same order as `slots`).
// Strings are used at the boundary for readability; conversion happens once
// per call before the hot path, not inside the loop.
// Returns (assignment_as_strings, score).
// ─────────────────────────────────────────────────────────────────────────────

#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub fn run_sa_optimize(
    // Deck geometry
    slots:      Vec<(i32, i32)>,
    row_peers:  Vec<Vec<usize>>,
    col_peers:  Vec<Vec<usize>>,
    surr_peers: Vec<Vec<usize>>,
    diag_peers: Vec<Vec<usize>>,
    n_arcane:   usize,
    min_regular: i32,
    max_greed:   i32,
    // Run parameters
    is_shiny:   bool,
    cores:      Vec<String>,     // e.g. ["pure", "color"]
    placeable:  Vec<String>,     // e.g. ["surr", "col", "surr_greed", ...]
    n_iter:     usize,
    t_start:    f64,
    t_end:      f64,
    // Multiplier constants (passed from Python globals)
    mult_dir_vert: f64,
    mult_dir_horiz: f64,
    mult_evo_greed: f64,
    mult_surr_greed: f64,
    mult_dir_diag_up: f64,
    mult_dir_diag_down: f64,
    mult_pure_base: f64,
    mult_pure_scale: f64,
    mult_equilibrium: f64,
    mult_foil: f64,
    mult_steadfast: f64,
    mult_color: f64,
    mult_deluxe_flat: f64,
    mult_deluxe_core_base: f64,
    mult_deluxe_core_scale: f64,
    // Flags
    greed_additive: bool,
    additive_cores: bool,
    shiny_positional: bool,
    enable_experimental: bool,
    experimental_exponent: f64,
    experimental_boost: f64,
    deluxe_counted_as_regular: bool,
) -> PyResult<(Vec<String>, f64)> {
    let n = slots.len();
    if n == 0 {
        return Ok((Vec::new(), 0.0));
    }

    // Build slot index map for directional lookups
    let slot_map: HashMap<(i32, i32), usize> = slots
        .iter()
        .enumerate()
        .map(|(i, &pos)| (pos, i))
        .collect();

    let row_of: Vec<i32> = slots.iter().map(|&(r, _)| r).collect();
    let col_of: Vec<i32> = slots.iter().map(|&(_, c)| c).collect();

    // Directional neighbours (1-step, diagonal, etc.)
    let dir = |i: usize, dr: i32, dc: i32| -> Option<usize> {
        slot_map.get(&(slots[i].0 + dr, slots[i].1 + dc)).copied()
    };

    let dir_up:    Vec<Option<usize>> = (0..n).map(|i| dir(i, -1,  0)).collect();
    let dir_down:  Vec<Option<usize>> = (0..n).map(|i| dir(i,  1,  0)).collect();
    let dir_left:  Vec<Option<usize>> = (0..n).map(|i| dir(i,  0, -1)).collect();
    let dir_right: Vec<Option<usize>> = (0..n).map(|i| dir(i,  0,  1)).collect();
    let dir_ne:    Vec<Option<usize>> = (0..n).map(|i| dir(i, -1,  1)).collect();
    let dir_nw:    Vec<Option<usize>> = (0..n).map(|i| dir(i, -1, -1)).collect();
    let dir_se:    Vec<Option<usize>> = (0..n).map(|i| dir(i,  1,  1)).collect();
    let dir_sw:    Vec<Option<usize>> = (0..n).map(|i| dir(i,  1, -1)).collect();

    // Geometry-optimal positional card per slot (from peer set sizes, same logic as Python)
    // ROW/COL/DIAG: counts self (+1); SURR: does not count self
    let best_positional: Vec<u8> = (0..n)
        .map(|i| {
            let row_cnt  = row_peers[i].len()  + 1;
            let col_cnt  = col_peers[i].len()  + 1;
            let surr_cnt = surr_peers[i].len();
            let diag_cnt = diag_peers[i].len() + 1;
            let best = *[
                (ROW,  row_cnt),
                (COL,  col_cnt),
                (SURR, surr_cnt),
                (DIAG, diag_cnt),
            ]
            .iter()
            .max_by_key(|&&(_, c)| c)
            .map(|(t, _)| t)
            .unwrap_or(&ROW);
            best
        })
        .collect();

    let cores_u8:    Vec<u8> = cores.iter().map(|s| core_from_str(s)).collect();
    let placeable_u8: Vec<u8> = placeable.iter().map(|s| card_type_from_str(s)).collect();
    let foil_active = cores_u8.contains(&CORE_FOIL);

    let deck = DeckData {
        n,
        row_of,
        col_of,
        row_peers,
        col_peers,
        surr_peers,
        diag_peers,
        dir_up,
        dir_down,
        dir_left,
        dir_right,
        dir_ne,
        dir_nw,
        dir_se,
        dir_sw,
        best_positional,
        n_arcane,
        min_regular,
        max_greed,
    };

    let cfg = SimConfig {
        mult_dir_vert,
        mult_dir_horiz,
        mult_evo_greed,
        mult_surr_greed,
        mult_dir_diag_up,
        mult_dir_diag_down,
        mult_pure_base,
        mult_pure_scale,
        mult_equilibrium,
        mult_foil,
        mult_steadfast,
        mult_color,
        mult_deluxe_flat,
        mult_deluxe_core_base,
        mult_deluxe_core_scale,
        greed_additive,
        additive_cores,
        is_shiny,
        foil_active,
        shiny_positional,
        enable_experimental,
        experimental_exponent,
        experimental_boost,
        deluxe_counted_as_regular,
    };

    let (best_asgn, best_score) =
        sa_optimize_inner(&deck, &cores_u8, &placeable_u8, &cfg, n_iter, t_start, t_end);

    // Convert u8 assignment back to strings for Python
    let asgn_strs: Vec<String> = best_asgn.iter().map(|&t| card_type_to_str(t).to_owned()).collect();

    Ok((asgn_strs, best_score))
}

