//! Inventory-based optimizer (color-aware, single-deck, single-run).
//!
//! Parallel sibling to ``run_sa_optimize`` in ``lib.rs``. Differences:
//!   * Cards are ``(CardType, Color)``; only inventory-present stacks may be placed.
//!   * Positional bonuses count only same-color cards in scan range.
//!   * COLOR core is per-color, only boosts matching-color cards.
//!   * Core multipliers may carry per-run overrides.
//!   * Empty slots after inventory exhaustion become transparent DEAD cards.
//!   * Restarts run in parallel via rayon.

use pyo3::prelude::*;
use rand::prelude::*;
use rand::rngs::SmallRng;
use rayon::prelude::*;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Card-type constants (must match the Python CardType.value strings)
// ─────────────────────────────────────────────────────────────────────────────

const ROW:             u8 = 0;
const COL:             u8 = 1;
const SURR:            u8 = 2;
const DIAG:            u8 = 3;
const DELUXE:          u8 = 4;
const TYPELESS:        u8 = 5;
const DIR_GREED_UP:    u8 = 6;
const DIR_GREED_DOWN:  u8 = 7;
const DIR_GREED_LEFT:  u8 = 8;
const DIR_GREED_RIGHT: u8 = 9;
const DIR_GREED_NE:    u8 = 10;
const DIR_GREED_NW:    u8 = 11;
const DIR_GREED_SE:    u8 = 12;
const DIR_GREED_SW:    u8 = 13;
const EVO_GREED:       u8 = 14;
const SURR_GREED:      u8 = 15;
const DEAD:            u8 = 16;
const ARCANE:          u8 = 17;

const N_TYPES: usize = 18;

// Colors
const RED:    u8 = 0;
const GREEN:  u8 = 1;
const BLUE:   u8 = 2;
const YELLOW: u8 = 3;
const N_COLORS: usize = 4;

// Sentinel — DEAD cards and "no color core" both use this.
const COLOR_NONE: u8 = 255;

// Cores
const CORE_PURE:        u8 = 0;
const CORE_EQUILIBRIUM: u8 = 1;
const CORE_STEADFAST:   u8 = 2;
const CORE_COLOR:       u8 = 3;
const CORE_FOIL:        u8 = 4;
const CORE_DELUXE:      u8 = 5;
const CORE_VOID:        u8 = 6;
// Archive core: base ** n_arcane_placed, applied *outside* the per-card core_mult.
const CORE_ARCHIVE:     u8 = 7;
// Sparkling core: flat SHINY-only multiplier — Wold's-only.
const CORE_SPARKLING:   u8 = 8;

// ─────────────────────────────────────────────────────────────────────────────
// String ↔ u8 conversions (Python boundary only — never on the hot path)
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
        "dead"            => DEAD,
        "arcane"          => ARCANE,
        other             => panic!("Unknown card type: {}", other),
    }
}

fn card_type_to_str(t: u8) -> &'static str {
    match t {
        ROW             => "row",
        COL             => "col",
        SURR            => "surr",
        DIAG            => "diag",
        DELUXE          => "deluxe",
        TYPELESS        => "typeless",
        DIR_GREED_UP    => "dir_greed_up",
        DIR_GREED_DOWN  => "dir_greed_down",
        DIR_GREED_LEFT  => "dir_greed_left",
        DIR_GREED_RIGHT => "dir_greed_right",
        DIR_GREED_NE    => "dir_greed_ne",
        DIR_GREED_NW    => "dir_greed_nw",
        DIR_GREED_SE    => "dir_greed_se",
        DIR_GREED_SW    => "dir_greed_sw",
        EVO_GREED       => "evo_greed",
        SURR_GREED      => "surr_greed",
        DEAD            => "dead",
        ARCANE          => "arcane",
        other           => panic!("Unknown card type u8: {}", other),
    }
}

fn color_from_str(s: &str) -> u8 {
    match s {
        "red"    => RED,
        "green"  => GREEN,
        "blue"   => BLUE,
        "yellow" => YELLOW,
        ""       => COLOR_NONE,
        other    => panic!("Unknown color: {}", other),
    }
}

fn color_to_str(c: u8) -> &'static str {
    match c {
        RED        => "red",
        GREEN      => "green",
        BLUE       => "blue",
        YELLOW     => "yellow",
        COLOR_NONE => "",
        other      => panic!("Unknown color u8: {}", other),
    }
}

fn core_from_str(s: &str) -> u8 {
    match s {
        "pure"        => CORE_PURE,
        "equilibrium" => CORE_EQUILIBRIUM,
        "steadfast"   => CORE_STEADFAST,
        "sparkling"   => CORE_SPARKLING,
        "color"       => CORE_COLOR,
        "foil"        => CORE_FOIL,
        "deluxe_core" => CORE_DELUXE,
        "void_core"   => CORE_VOID,
        "archive_core" => CORE_ARCHIVE,
        other         => panic!("Unknown core type: {}", other),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Category predicates
// ─────────────────────────────────────────────────────────────────────────────

#[inline(always)]
fn is_positional(t: u8) -> bool {
    matches!(t, ROW | COL | SURR | DIAG)
}

#[inline(always)]
fn is_greed(t: u8) -> bool {
    matches!(
        t,
        DIR_GREED_UP | DIR_GREED_DOWN | DIR_GREED_LEFT | DIR_GREED_RIGHT
        | DIR_GREED_NE | DIR_GREED_NW | DIR_GREED_SE | DIR_GREED_SW
        | EVO_GREED | SURR_GREED
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Deck geometry + sim config
// ─────────────────────────────────────────────────────────────────────────────

struct DeckGeom {
    n: usize,
    row_of: Vec<i32>,
    col_of: Vec<i32>,
    row_peers:  Vec<Vec<usize>>,
    col_peers:  Vec<Vec<usize>>,
    surr_peers: Vec<Vec<usize>>,
    diag_peers: Vec<Vec<usize>>,
    dir_up:    Vec<Option<usize>>,
    dir_down:  Vec<Option<usize>>,
    dir_left:  Vec<Option<usize>>,
    dir_right: Vec<Option<usize>>,
    dir_ne:    Vec<Option<usize>>,
    dir_nw:    Vec<Option<usize>>,
    dir_se:    Vec<Option<usize>>,
    dir_sw:    Vec<Option<usize>>,
    // Per-slot arcane flag. Arcane slots accept only ARCANE or DEAD.
    is_arcane_slot: Vec<bool>,
    // Row/col offset machinery for dense per-color counters.
    row_min: i32,
    row_span: usize,
    col_min: i32,
    col_span: usize,
}

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
    mult_sparkling: f64,
    mult_color: f64,
    mult_deluxe_flat: f64,
    mult_deluxe_core_base: f64,
    mult_deluxe_core_scale: f64,
    mult_void_core_base: f64,
    mult_void_core_scale: f64,
    /// Archive core per-arcane base; final mult = `base ^ n_arcane_placed`.
    mult_archive_core: f64,
    greed_additive: bool,
    additive_cores: bool,
    is_shiny: bool,
    // Arcane behaviour. true → arcane slots locked to their initial fill (color
    // swaps within ARCANE-locked slots still allowed). false → SA may swap
    // ARCANE ↔ DEAD per arcane slot.
    auto_place_arcane: bool,
}

#[derive(Clone, Copy)]
struct CoreData {
    core_type: u8,
    color:     u8,        // COLOR_NONE unless core_type == CORE_COLOR
    override_: f64,       // -1.0 == no override
}

impl CoreData {
    fn has_override(&self) -> bool { self.override_ >= 0.0 }
}

// Cores list packed for fast iteration. simulate() reads `list` and
// `color_core_color` / `foil_active` directly; per-card combination math is
// done inside the kernel.
struct CoresPack {
    list:             Vec<CoreData>,
    color_core_color: u8,    // COLOR_NONE if absent
    foil_active:      bool,
}

impl CoresPack {
    fn build(specs: &[CoreData], _cfg: &SimConfig) -> Self {
        let mut color_core_color = COLOR_NONE;
        let mut foil_active = false;
        for s in specs {
            match s.core_type {
                CORE_COLOR => { color_core_color = s.color; }
                CORE_FOIL  => { foil_active = true; }
                _ => {}
            }
        }
        Self {
            list: specs.to_vec(),
            color_core_color,
            foil_active,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scoring kernel — full-assignment evaluation (no delta yet)
// ─────────────────────────────────────────────────────────────────────────────

fn simulate(
    geom:       &DeckGeom,
    asgn:       &[(u8, u8)],
    cores:      &CoresPack,
    cfg:        &SimConfig,
    // Scratch buffers — caller-allocated for zero-allocation hot path.
    row_color:  &mut [u32],   // len = row_span * N_COLORS
    col_color:  &mut [u32],   // len = col_span * N_COLORS
    boost:      &mut [f64],   // len = n
) -> f64 {
    let n = geom.n;

    // Reset scratch.
    for v in row_color.iter_mut() { *v = 0; }
    for v in col_color.iter_mut() { *v = 0; }

    // Same-color counts per row / col (all non-DEAD colored cards count
    // regardless of type — ARCANE counts here too so it boosts neighbors).
    let mut n_positional = 0usize;
    let mut n_deluxe     = 0usize;
    let mut n_typeless   = 0usize;
    let mut n_greed      = 0usize;
    let mut n_arcane     = 0usize;
    let mut n_dead       = 0usize;

    for i in 0..n {
        let (t, c) = asgn[i];
        if t == DEAD {
            n_dead += 1;
            continue;
        }
        if c == COLOR_NONE { continue; }
        let r = (geom.row_of[i] - geom.row_min) as usize;
        let cc = (geom.col_of[i] - geom.col_min) as usize;
        row_color[r * N_COLORS + c as usize] += 1;
        col_color[cc * N_COLORS + c as usize] += 1;
        if is_positional(t)         { n_positional += 1; }
        else if t == DELUXE         { n_deluxe     += 1; }
        else if t == TYPELESS       { n_typeless   += 1; }
        else if t == ARCANE         { n_arcane     += 1; }
        else if is_greed(t)         { n_greed      += 1; }
    }

    // n_ns for PURE. ARCANE placements always count (preserving the
    // pre-arcane `+ geom.n_arcane` fudge with real placements). The class /
    // FOIL rule decides which OTHER placements count on top:
    //   EVO-no-FOIL  → positional + deluxe + typeless + greed (+arcane)
    //   EVO-with-FOIL → greed (+arcane)
    //   SHINY         → greed (+arcane)
    let n_ns = if cfg.is_shiny || cores.foil_active {
        n_greed + n_arcane
    } else {
        n_positional + n_deluxe + n_typeless + n_arcane + n_greed
    };

    // All cores fold into a single per-card core_mult. Precompute the baseline
    // (cores that apply to every non-greed card regardless of color), plus the
    // color-, deluxe-, void-core gated addends, so the per-card combination at
    // accumulation time is constant-time.
    let mut baseline_sum  = 0.0f64;
    let mut baseline_prod = 1.0f64;
    let mut color_addend  = 0.0f64;
    let mut color_factor_val = 1.0f64;
    let mut deluxe_addend = 0.0f64;
    let mut deluxe_factor = 1.0f64;
    let mut deluxe_present = false;
    let mut void_addend   = 0.0f64;
    let mut void_factor   = 1.0f64;
    let mut void_present  = false;
    // Archive core: per-arcane base; final multiplier is base ** n_arcane_placed.
    // Applied *outside* the per-card core_mult, so it bypasses the additive_cores
    // stacking switch.
    let mut archive_base    = 1.0f64;
    let mut archive_present = false;
    let color_core_color = cores.color_core_color;

    for s in &cores.list {
        match s.core_type {
            CORE_PURE => {
                let scale = if s.has_override() { s.override_ } else { cfg.mult_pure_scale };
                // n_ns already includes placed ARCANE cards; no fudge addend.
                let v = cfg.mult_pure_base + scale * n_ns as f64;
                baseline_sum  += v - 1.0;
                baseline_prod *= v;
            }
            CORE_EQUILIBRIUM if cfg.is_shiny => {
                let v = if s.has_override() { s.override_ } else { cfg.mult_equilibrium };
                baseline_sum  += v - 1.0;
                baseline_prod *= v;
            }
            CORE_STEADFAST if cfg.is_shiny => {
                let v = if s.has_override() { s.override_ } else { cfg.mult_steadfast };
                baseline_sum  += v - 1.0;
                baseline_prod *= v;
            }
            CORE_SPARKLING if cfg.is_shiny => {
                let v = if s.has_override() { s.override_ } else { cfg.mult_sparkling };
                baseline_sum  += v - 1.0;
                baseline_prod *= v;
            }
            CORE_FOIL => {
                let v = if s.has_override() { s.override_ } else { cfg.mult_foil };
                baseline_sum  += v - 1.0;
                baseline_prod *= v;
            }
            CORE_COLOR => {
                let v = if s.has_override() { s.override_ } else { cfg.mult_color };
                color_addend     = v - 1.0;
                color_factor_val = v;
            }
            CORE_DELUXE => {
                let scale = if s.has_override() { s.override_ } else { cfg.mult_deluxe_core_scale };
                let v = cfg.mult_deluxe_core_base + scale * n_deluxe as f64;
                deluxe_addend  = v - 1.0;
                deluxe_factor  = v;
                deluxe_present = true;
            }
            CORE_VOID => {
                let scale = if s.has_override() { s.override_ } else { cfg.mult_void_core_scale };
                let v = cfg.mult_void_core_base + scale * n_dead as f64;
                void_addend   = v - 1.0;
                void_factor   = v;
                void_present  = true;
            }
            CORE_ARCHIVE => {
                archive_base    = if s.has_override() { s.override_ } else { cfg.mult_archive_core };
                archive_present = true;
            }
            _ => {}
        }
    }

    // Archive multiplier — applied outside the per-card core_mult.
    let archive_mult: f64 = if archive_present {
        archive_base.powi(n_arcane as i32)
    } else {
        1.0
    };

    // Per-card core multiplier — picks color/deluxe/void addends per
    // applicability. (Dead cards are skipped before this is called; gating is
    // left here for symmetry and breakdown correctness.)
    let card_core_mult = |t: u8, c: u8| -> f64 {
        let color_applies  =
            color_core_color != COLOR_NONE && c != COLOR_NONE && c == color_core_color;
        let deluxe_applies = deluxe_present && t != DELUXE;
        let void_applies   = void_present   && t != DEAD;
        if cfg.additive_cores {
            1.0 + baseline_sum
                + if color_applies  { color_addend  } else { 0.0 }
                + if deluxe_applies { deluxe_addend } else { 0.0 }
                + if void_applies   { void_addend   } else { 0.0 }
        } else {
            let mut m = baseline_prod;
            if color_applies  { m *= color_factor_val; }
            if deluxe_applies { m *= deluxe_factor; }
            if void_applies   { m *= void_factor; }
            m
        }
    };
    // n_positional is tracked in the classifier for symmetry with future
    // per-card gating; silence the unused-variable warning until then.
    let _ = n_positional;

    // Greed → boost pass (same semantics as classic optimizer).
    // Additive: reset to 0 and accumulate raw multipliers (use-site clamps to ≥ 1).
    // Multiplicative: reset to 1 and multiply.
    // Additive boost starts at 1.0 so the formula is `1 + Σ greeds` for any
    // number of greeds (including 0). Multiplicative still starts at 1.0 so
    // multiplying by zero greeds leaves boost = 1.
    let reset_boost = 1.0;
    for v in boost[..n].iter_mut() { *v = reset_boost; }

    let scorable = |i: usize| -> bool {
        let t = asgn[i].0;
        is_positional(t) || t == DELUXE || t == TYPELESS
    };

    let apply = |boost: &mut [f64], pos: usize, amount: f64| {
        if cfg.greed_additive { boost[pos] += amount; }
        else                  { boost[pos] *= amount; }
    };

    for i in 0..n {
        let (t, _c) = asgn[i];
        if !is_greed(t) { continue; }
        match t {
            DIR_GREED_UP    => if let Some(j) = geom.dir_up[i]    { if scorable(j) { apply(boost, j, cfg.mult_dir_vert); } }
            DIR_GREED_DOWN  => if let Some(j) = geom.dir_down[i]  { if scorable(j) { apply(boost, j, cfg.mult_dir_vert); } }
            DIR_GREED_LEFT  => if let Some(j) = geom.dir_left[i]  { if scorable(j) { apply(boost, j, cfg.mult_dir_horiz); } }
            DIR_GREED_RIGHT => if let Some(j) = geom.dir_right[i] { if scorable(j) { apply(boost, j, cfg.mult_dir_horiz); } }
            DIR_GREED_NE    => if let Some(j) = geom.dir_ne[i]    { if scorable(j) { apply(boost, j, cfg.mult_dir_diag_up); } }
            DIR_GREED_NW    => if let Some(j) = geom.dir_nw[i]    { if scorable(j) { apply(boost, j, cfg.mult_dir_diag_up); } }
            DIR_GREED_SE    => if let Some(j) = geom.dir_se[i]    { if scorable(j) { apply(boost, j, cfg.mult_dir_diag_down); } }
            DIR_GREED_SW    => if let Some(j) = geom.dir_sw[i]    { if scorable(j) { apply(boost, j, cfg.mult_dir_diag_down); } }
            EVO_GREED => {
                if !cfg.is_shiny {
                    if let Some(j) = geom.dir_down[i] {
                        if is_positional(asgn[j].0) {
                            apply(boost, j, cfg.mult_evo_greed);
                        }
                    }
                }
            }
            SURR_GREED => {
                for &j in &geom.surr_peers[i] {
                    if scorable(j) { apply(boost, j, cfg.mult_surr_greed); }
                }
            }
            _ => {}
        }
    }

    // NDM accumulation — uses the per-card combined card_core_mult above.
    let mut ndm = 0.0f64;
    for i in 0..n {
        let (t, c) = asgn[i];
        if t == DEAD { continue; }
        let b = if cfg.greed_additive { boost[i].max(1.0) } else { boost[i] };

        if is_positional(t) {
            let cu = c as usize;
            let pos_val = if c == COLOR_NONE {
                0.0
            } else {
                match t {
                    ROW => {
                        let r = (geom.row_of[i] - geom.row_min) as usize;
                        row_color[r * N_COLORS + cu] as f64
                    }
                    COL => {
                        let cc = (geom.col_of[i] - geom.col_min) as usize;
                        col_color[cc * N_COLORS + cu] as f64
                    }
                    DIAG => {
                        // Same-color peers on either diagonal, NOT counting
                        // self. Clamped to a minimum of 1.0 so a lone DIAG
                        // card still contributes its base value instead of 0×.
                        let mut count: f64 = 0.0;
                        for &q in &geom.diag_peers[i] {
                            let (qt, qc) = asgn[q];
                            if qt != DEAD && qc == c { count += 1.0; }
                        }
                        count.max(1.0)
                    }
                    SURR => {
                        let mut count = 0.0; // SURR excludes self
                        for &q in &geom.surr_peers[i] {
                            let (qt, qc) = asgn[q];
                            if qt != DEAD && qc == c { count += 1.0; }
                        }
                        count
                    }
                    _ => 0.0,
                }
            };
            ndm += pos_val * card_core_mult(t, c) * b * archive_mult;
        } else if t == DELUXE {
            ndm += cfg.mult_deluxe_flat * card_core_mult(t, c) * b * archive_mult;
        } else if t == TYPELESS {
            ndm += 1.0 * card_core_mult(t, c) * b * archive_mult;
        }
        // GREED / DEAD contribute nothing.
    }

    ndm
}

// ─────────────────────────────────────────────────────────────────────────────
// Initial fill — mirrors initial_fill() in src/inventory_optimize.py
// ─────────────────────────────────────────────────────────────────────────────

const FILL_ORDER: [u8; 6] = [SURR, ROW, COL, DIAG, DELUXE, TYPELESS];

fn slot_ranking(geom: &DeckGeom, t: u8) -> Vec<usize> {
    let peer_count = |i: usize| -> usize {
        match t {
            ROW  => geom.row_peers[i].len(),
            COL  => geom.col_peers[i].len(),
            SURR => geom.surr_peers[i].len(),
            DIAG => geom.diag_peers[i].len(),
            _    => 0,
        }
    };
    let mut idx: Vec<usize> = (0..geom.n).collect();
    idx.sort_by(|&a, &b| peer_count(b).cmp(&peer_count(a)));
    idx
}

fn initial_fill(
    geom:      &DeckGeom,
    inventory: &[u32],     // regular pool, flat N_TYPES * N_COLORS counts
    forced:    &[u32],     // forced pool, same shape — must all be placed
) -> Vec<(u8, u8)> {
    let mut asgn: Vec<(u8, u8)> = vec![(DEAD, COLOR_NONE); geom.n];
    let mut filled = vec![false; geom.n];

    // Combined budget for phase 2; we decrement as phase 1 consumes its share.
    let mut remaining: Vec<u32> = inventory
        .iter().zip(forced.iter())
        .map(|(&r, &f)| r + f)
        .collect();
    let mut forced_remaining: Vec<u32> = forced.to_vec();

    // ── Phase 0: arcane slots ────────────────────────────────────────────────
    // Fill every arcane slot with ARCANE drawn from the biggest color bucket
    // first. When inventory runs out, the remaining arcane slots stay DEAD
    // (the only other legal placement). Honors forced ARCANE counts implicitly.
    for s in 0..geom.n {
        if !geom.is_arcane_slot[s] { continue; }
        // Pick the color with the most ARCANE cards still available.
        let mut best_color: Option<u8> = None;
        let mut best_avail = 0u32;
        for c in 0..N_COLORS as u8 {
            let idx = ARCANE as usize * N_COLORS + c as usize;
            if remaining[idx] > best_avail {
                best_avail = remaining[idx];
                best_color = Some(c);
            }
        }
        match best_color {
            Some(c) => {
                asgn[s] = (ARCANE, c);
                filled[s] = true;
                let idx = ARCANE as usize * N_COLORS + c as usize;
                remaining[idx] -= 1;
                if forced_remaining[idx] > 0 {
                    forced_remaining[idx] -= 1;
                }
            }
            None => {
                // No ARCANE inventory left — fall back to DEAD (still legal).
                // asgn[s] is already (DEAD, COLOR_NONE) from the default fill.
                filled[s] = true;
            }
        }
    }

    // ── Phase 1: place all forced cards ──────────────────────────────────────
    // Positional / deluxe / typeless first via the geometric rankings.
    for &t in &FILL_ORDER {
        let ranking = slot_ranking(geom, t);
        let mut color_order: [u8; N_COLORS] = [RED, GREEN, BLUE, YELLOW];
        color_order.sort_by(|&a, &b| {
            forced_remaining[t as usize * N_COLORS + b as usize]
                .cmp(&forced_remaining[t as usize * N_COLORS + a as usize])
        });
        let mut cursor = 0usize;
        for &c in &color_order {
            let idx = t as usize * N_COLORS + c as usize;
            while forced_remaining[idx] > 0 {
                // Skip filled slots AND arcane slots (no non-arcane allowed there).
                while cursor < ranking.len()
                    && (filled[ranking[cursor]] || geom.is_arcane_slot[ranking[cursor]])
                {
                    cursor += 1;
                }
                if cursor >= ranking.len() { break; }
                let slot = ranking[cursor];
                asgn[slot] = (t, c);
                filled[slot] = true;
                forced_remaining[idx] -= 1;
                remaining[idx]       -= 1;
                cursor += 1;
            }
            if cursor >= ranking.len() { break; }
        }
    }
    // Catch any forced types NOT in FILL_ORDER (greeds) via plain slot iteration.
    // Skip ARCANE (already handled in Phase 0) and arcane slots.
    for t_idx in 0..N_TYPES {
        if t_idx as u8 == ARCANE { continue; }
        for c_idx in 0..N_COLORS {
            let idx = t_idx * N_COLORS + c_idx;
            while forced_remaining[idx] > 0 {
                // Find any open NON-arcane slot.
                let mut placed = false;
                for s in 0..geom.n {
                    if !filled[s] && !geom.is_arcane_slot[s] {
                        asgn[s] = (t_idx as u8, c_idx as u8);
                        filled[s] = true;
                        forced_remaining[idx] -= 1;
                        remaining[idx]       -= 1;
                        placed = true;
                        break;
                    }
                }
                if !placed { break; }
            }
        }
    }

    // ── Phase 2: place regular cards using existing priority ────────────────
    // Skip ARCANE (already placed in Phase 0) and arcane slots.
    for &t in &FILL_ORDER {
        let ranking = slot_ranking(geom, t);
        let mut color_order: [u8; N_COLORS] = [RED, GREEN, BLUE, YELLOW];
        color_order.sort_by(|&a, &b| {
            remaining[t as usize * N_COLORS + b as usize]
                .cmp(&remaining[t as usize * N_COLORS + a as usize])
        });
        let mut cursor = 0usize;
        for &c in &color_order {
            let idx = t as usize * N_COLORS + c as usize;
            while remaining[idx] > 0 {
                while cursor < ranking.len()
                    && (filled[ranking[cursor]] || geom.is_arcane_slot[ranking[cursor]])
                {
                    cursor += 1;
                }
                if cursor >= ranking.len() { break; }
                let slot = ranking[cursor];
                asgn[slot] = (t, c);
                filled[slot] = true;
                remaining[idx] -= 1;
                cursor += 1;
            }
            if cursor >= ranking.len() { break; }
        }
    }

    asgn
}

// ─────────────────────────────────────────────────────────────────────────────
// SA — one restart
// ─────────────────────────────────────────────────────────────────────────────

fn sa_one_restart(
    geom:       &DeckGeom,
    cores:      &CoresPack,
    cfg:        &SimConfig,
    inventory:  &[u32],          // regular pool, N_TYPES * N_COLORS
    forced:     &[u32],          // forced pool, same shape — per-(t,c) lower bound
    cap:        &[u32],          // = inventory + forced, per-(t,c) upper bound
    options:    &[(u8, u8)],     // stacks the user owns + (DEAD, COLOR_NONE) at end
    n_iter:     usize,
    t_start:    f64,
    t_end:      f64,
    seed:       u64,
) -> (Vec<(u8, u8)>, f64) {
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut asgn = initial_fill(geom, inventory, forced);

    // Placed counters (flat N_TYPES * N_COLORS).
    let mut placed = vec![0u32; N_TYPES * N_COLORS];
    for &(t, c) in &asgn {
        if t == DEAD || c == COLOR_NONE { continue; }
        placed[t as usize * N_COLORS + c as usize] += 1;
    }

    // Scratch buffers reused across simulate() calls.
    let mut row_color = vec![0u32; geom.row_span * N_COLORS];
    let mut col_color = vec![0u32; geom.col_span * N_COLORS];
    let mut boost     = vec![1.0f64; geom.n];

    let mut score = simulate(geom, &asgn, cores, cfg, &mut row_color, &mut col_color, &mut boost);
    let mut best_score = score;
    let mut best_asgn  = asgn.clone();

    let log_cool = (t_end / t_start).ln();
    let n = geom.n;

    // Arcane-restricted proposal alphabet: every ARCANE color with cap > 0,
    // plus DEAD when auto_place is off. When auto_place is on, ARCANE-locked
    // slots can only swap colors among other ARCANE choices.
    let mut arcane_options: Vec<(u8, u8)> = Vec::new();
    for c in 0..N_COLORS as u8 {
        let idx = ARCANE as usize * N_COLORS + c as usize;
        if cap[idx] > 0 {
            arcane_options.push((ARCANE, c));
        }
    }
    if !cfg.auto_place_arcane {
        arcane_options.push((DEAD, COLOR_NONE));
    }

    // Under auto_place_arcane, ARCANE-slot positions filled with DEAD at init
    // (inventory exhausted) are locked — SA cannot change them at all. Track
    // their indices so the move kernel can skip them.
    let mut locked_arcane: Vec<bool> = vec![false; n];
    if cfg.auto_place_arcane {
        for i in 0..n {
            if geom.is_arcane_slot[i] && asgn[i] == (DEAD, COLOR_NONE) {
                locked_arcane[i] = true;
            }
        }
    }

    for i in 0..n_iter {
        let temperature = t_start * (log_cool * i as f64 / n_iter as f64).exp();

        if n < 2 || rng.gen::<f64>() < 0.80 {
            // ── Replace move ─────────────────────────────────────────────────
            let p   = rng.gen_range(0..n);
            // Skip locked arcane slots entirely.
            if locked_arcane[p] { continue; }
            let old = asgn[p];

            // Pick `new` from the appropriate alphabet for this slot type.
            let new = if geom.is_arcane_slot[p] {
                if arcane_options.is_empty() { continue; }
                arcane_options[rng.gen_range(0..arcane_options.len())]
            } else {
                let candidate = options[rng.gen_range(0..options.len())];
                // Regular slot may never receive ARCANE.
                if candidate.0 == ARCANE { continue; }
                candidate
            };
            if new == old { continue; }

            // Upper bound on `new`: placed[new] < cap[new] = inv + forced.
            if !(new.0 == DEAD || new.1 == COLOR_NONE) {
                let idx = new.0 as usize * N_COLORS + new.1 as usize;
                if placed[idx] >= cap[idx] { continue; }
            }
            // Lower bound on `old`: removing one must leave placed[old] >= forced[old].
            if !(old.0 == DEAD || old.1 == COLOR_NONE) {
                let idx = old.0 as usize * N_COLORS + old.1 as usize;
                if placed[idx] <= forced[idx] { continue; }
            }

            // Apply
            if old.0 != DEAD && old.1 != COLOR_NONE {
                placed[old.0 as usize * N_COLORS + old.1 as usize] -= 1;
            }
            if new.0 != DEAD && new.1 != COLOR_NONE {
                placed[new.0 as usize * N_COLORS + new.1 as usize] += 1;
            }
            asgn[p] = new;

            let new_score = simulate(geom, &asgn, cores, cfg, &mut row_color, &mut col_color, &mut boost);
            let delta = new_score - score;
            if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                score = new_score;
                if score > best_score { best_score = score; best_asgn = asgn.clone(); }
            } else {
                // Revert
                if new.0 != DEAD && new.1 != COLOR_NONE {
                    placed[new.0 as usize * N_COLORS + new.1 as usize] -= 1;
                }
                if old.0 != DEAD && old.1 != COLOR_NONE {
                    placed[old.0 as usize * N_COLORS + old.1 as usize] += 1;
                }
                asgn[p] = old;
            }
        } else {
            // ── Pair-swap move ───────────────────────────────────────────────
            let p1 = rng.gen_range(0..n);
            let mut p2 = rng.gen_range(0..n);
            while p2 == p1 { p2 = rng.gen_range(0..n); }
            if asgn[p1] == asgn[p2] { continue; }
            // Reject swaps involving locked arcane slots.
            if locked_arcane[p1] || locked_arcane[p2] { continue; }
            // Reject swaps that would place ARCANE in a regular slot, or
            // place a non-{ARCANE, DEAD} card in an arcane slot.
            let a1 = geom.is_arcane_slot[p1];
            let a2 = geom.is_arcane_slot[p2];
            let v1 = asgn[p2].0;   // type that lands at p1 after swap
            let v2 = asgn[p1].0;   // type that lands at p2 after swap
            let legal = |is_arc: bool, t: u8| -> bool {
                if is_arc { t == ARCANE || t == DEAD } else { t != ARCANE }
            };
            if !legal(a1, v1) || !legal(a2, v2) { continue; }

            asgn.swap(p1, p2);
            let new_score = simulate(geom, &asgn, cores, cfg, &mut row_color, &mut col_color, &mut boost);
            let delta = new_score - score;
            if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                score = new_score;
                if score > best_score { best_score = score; best_asgn = asgn.clone(); }
            } else {
                asgn.swap(p1, p2);
            }
        }
    }

    (best_asgn, best_score)
}

// ─────────────────────────────────────────────────────────────────────────────
// PyO3 entry point
// ─────────────────────────────────────────────────────────────────────────────

#[pyfunction]
#[pyo3(signature = (
    slots, row_peers, col_peers, surr_peers, diag_peers,
    arcane_slot_indices, auto_place_arcane,
    is_shiny, inventory, forced_inventory, cores,
    n_iter, restarts,
    mult_dir_vert, mult_dir_horiz, mult_evo_greed, mult_surr_greed,
    mult_dir_diag_up, mult_dir_diag_down,
    mult_pure_base, mult_pure_scale,
    mult_equilibrium, mult_foil, mult_steadfast, mult_sparkling, mult_color,
    mult_deluxe_flat, mult_deluxe_core_base, mult_deluxe_core_scale,
    mult_void_core_base, mult_void_core_scale,
    mult_archive_core,
    greed_additive, additive_cores,
))]
pub fn run_sa_inventory(
    slots:                  Vec<(i32, i32)>,
    row_peers:              Vec<Vec<usize>>,
    col_peers:              Vec<Vec<usize>>,
    surr_peers:             Vec<Vec<usize>>,
    diag_peers:             Vec<Vec<usize>>,
    arcane_slot_indices:    Vec<usize>,
    auto_place_arcane:      bool,
    is_shiny:               bool,
    inventory:              Vec<(String, String, u32)>,  // regular: (type, color, count)
    forced_inventory:       Vec<(String, String, u32)>,  // forced: same shape
    cores:                  Vec<(String, String, f64)>,  // (type, color, override<0 = none)
    n_iter:                 usize,
    restarts:               usize,
    mult_dir_vert:          f64,
    mult_dir_horiz:         f64,
    mult_evo_greed:         f64,
    mult_surr_greed:        f64,
    mult_dir_diag_up:       f64,
    mult_dir_diag_down:     f64,
    mult_pure_base:         f64,
    mult_pure_scale:        f64,
    mult_equilibrium:       f64,
    mult_foil:              f64,
    mult_steadfast:         f64,
    mult_sparkling:         f64,
    mult_color:             f64,
    mult_deluxe_flat:       f64,
    mult_deluxe_core_base:  f64,
    mult_deluxe_core_scale: f64,
    mult_void_core_base:    f64,
    mult_void_core_scale:   f64,
    mult_archive_core:      f64,
    greed_additive:         bool,
    additive_cores:         bool,
) -> PyResult<(Vec<(String, String)>, f64)> {
    let n = slots.len();

    let slot_map: HashMap<(i32, i32), usize> =
        slots.iter().enumerate().map(|(i, &p)| (p, i)).collect();
    let row_of: Vec<i32> = slots.iter().map(|&(r, _)| r).collect();
    let col_of: Vec<i32> = slots.iter().map(|&(_, c)| c).collect();

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

    let row_min = *row_of.iter().min().unwrap_or(&0);
    let row_max = *row_of.iter().max().unwrap_or(&0);
    let col_min = *col_of.iter().min().unwrap_or(&0);
    let col_max = *col_of.iter().max().unwrap_or(&0);

    // Build per-slot arcane flag from the index list.
    let mut is_arcane_slot = vec![false; n];
    for &idx in &arcane_slot_indices {
        if idx < n { is_arcane_slot[idx] = true; }
    }

    let geom = DeckGeom {
        n,
        row_of,
        col_of,
        row_peers,
        col_peers,
        surr_peers,
        diag_peers,
        dir_up, dir_down, dir_left, dir_right,
        dir_ne, dir_nw, dir_se, dir_sw,
        is_arcane_slot,
        row_min,
        row_span: (row_max - row_min + 1) as usize,
        col_min,
        col_span: (col_max - col_min + 1) as usize,
    };

    let cfg = SimConfig {
        mult_dir_vert, mult_dir_horiz, mult_evo_greed, mult_surr_greed,
        mult_dir_diag_up, mult_dir_diag_down,
        mult_pure_base, mult_pure_scale,
        mult_equilibrium, mult_foil, mult_steadfast, mult_sparkling, mult_color,
        mult_deluxe_flat, mult_deluxe_core_base, mult_deluxe_core_scale,
        mult_void_core_base, mult_void_core_scale,
        mult_archive_core,
        greed_additive, additive_cores, is_shiny,
        auto_place_arcane,
    };

    // Pack cores
    let core_specs: Vec<CoreData> = cores
        .iter()
        .map(|(t, c, o)| CoreData {
            core_type: core_from_str(t),
            color:     color_from_str(c),
            override_: *o,
        })
        .collect();
    let cores_pack = CoresPack::build(&core_specs, &cfg);

    // Pack inventory (regular) into flat array
    let mut inv_flat = vec![0u32; N_TYPES * N_COLORS];
    for (t_s, c_s, n) in &inventory {
        let t = card_type_from_str(t_s) as usize;
        let c = color_from_str(c_s);
        if c == COLOR_NONE { continue; }   // skip malformed entries
        inv_flat[t * N_COLORS + c as usize] += *n;
    }
    // Pack forced into a parallel flat array (same shape as inv_flat).
    let mut forced_flat = vec![0u32; N_TYPES * N_COLORS];
    for (t_s, c_s, n) in &forced_inventory {
        let t = card_type_from_str(t_s) as usize;
        let c = color_from_str(c_s);
        if c == COLOR_NONE { continue; }
        forced_flat[t * N_COLORS + c as usize] += *n;
    }
    // Cap = regular + forced for the SA's upper-bound check.
    let cap_flat: Vec<u32> = inv_flat
        .iter().zip(forced_flat.iter())
        .map(|(&a, &b)| a + b)
        .collect();

    // Build proposal options list: every owned (type, color) in EITHER pool
    // plus DEAD sentinel. Driven by cap_flat so neither bucket is forgotten.
    let mut options: Vec<(u8, u8)> = Vec::with_capacity(inventory.len() + forced_inventory.len() + 1);
    for t_idx in 0..N_TYPES {
        for c_idx in 0..N_COLORS {
            if cap_flat[t_idx * N_COLORS + c_idx] > 0 {
                options.push((t_idx as u8, c_idx as u8));
            }
        }
    }
    options.push((DEAD, COLOR_NONE));

    // Restarts in parallel via rayon. Seeds: hash of (i, n_iter) so runs are
    // deterministic per restart but diverge across restarts.
    let restarts = restarts.max(1);
    let results: Vec<(Vec<(u8, u8)>, f64)> = (0..restarts)
        .into_par_iter()
        .map(|i| {
            // Use thread-local entropy XOR'd with restart index for a robust seed.
            let mut seed_rng = SmallRng::from_entropy();
            let seed: u64 = seed_rng.gen::<u64>() ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            sa_one_restart(&geom, &cores_pack, &cfg, &inv_flat, &forced_flat, &cap_flat, &options, n_iter, 100.0, 0.5, seed)
        })
        .collect();

    // Pick best result.
    let (best_asgn, best_score) = results
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .expect("at least one restart");

    let result_strs: Vec<(String, String)> = best_asgn
        .iter()
        .map(|&(t, c)| (card_type_to_str(t).to_owned(), color_to_str(c).to_owned()))
        .collect();

    Ok((result_strs, best_score))
}
