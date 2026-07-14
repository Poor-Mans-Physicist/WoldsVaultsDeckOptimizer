//! Optimizer 2.0 — the tag-aware SA kernel ("one kernel, three configurations").
//!
//! CANONICAL SOURCE. This file is included verbatim by BOTH crates:
//!   * outer `ndm_core` (PyO3, spreadsheet CLI) — `mod tagsim;`
//!   * `wasm-port/ndm_core` (wasm-bindgen, web app) — `#[path = ...] mod tagsim;`
//! Keep it pure Rust + `rand` only: no pyo3, no wasm-bindgen, no rayon.
//! Parallelism lives OUTSIDE the kernel (process-per-deck on the CLI,
//! worker-pool restart-chunks on the web).
//!
//! The three modes are supply/constraint configurations of this one kernel:
//!   Max      — unlimited mono-color supply, blanket favorable tags,
//!              colors_real=false. Reproduces the classic `lib.rs` numbers on
//!              vanilla / no-implicit runs (validation gate).
//!   Targeted — unlimited supply + per-tag min/max rules; capped
//!              implicit-relevant groups become per-slot SA variables;
//!              colors_real when any color rule or Complex Cards is active.
//!   Exact    — finite per-stack supply with real per-card tags,
//!              colors_real=true always.
//!
//! Card model (spec §2): type + card_color + scale_color + groups bitmask.
//! Foil and Stat live in the groups mask; Wild is its own card type.
//! Real greed = the 4 orthogonal directions only (spec §2.3).

use rand::prelude::*;
use rand::rngs::SmallRng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Card-type constants (2.0 vocabulary)
// ─────────────────────────────────────────────────────────────────────────────

pub const T_ROW: u8      = 0;
pub const T_COL: u8      = 1;
pub const T_SURR: u8     = 2;
pub const T_DIAG: u8     = 3;
pub const T_DELUXE: u8   = 4;
pub const T_TYPELESS: u8 = 5;
pub const T_G_UP: u8     = 6;
pub const T_G_DOWN: u8   = 7;
pub const T_G_LEFT: u8   = 8;
pub const T_G_RIGHT: u8  = 9;
pub const T_ARCANE: u8   = 10;
// Wild: 0 NDM itself; counts as ANY group and ANY color for neighbors'
// positional scaling (universal wildcard neighbor, spec §2.2).
pub const T_WILD: u8     = 11;
pub const T_DEAD: u8     = 12;

pub const N_TYPES: usize = 13;

pub const RED: u8 = 0;
pub const GREEN: u8 = 1;
pub const BLUE: u8 = 2;
pub const YELLOW: u8 = 3;
pub const N_COLORS: usize = 4;
pub const COLOR_NONE: u8 = 255;

// Group bits (u16). The 9 freeform category tags + Foil + Stat.
pub const G_OFFENSIVE: u16 = 1 << 0;
pub const G_DEFENSIVE: u16 = 1 << 1;
pub const G_PHYSICAL: u16  = 1 << 2;
pub const G_MAGICAL: u16   = 1 << 3;
pub const G_UTILITY: u16   = 1 << 4;
pub const G_RESOURCE: u16  = 1 << 5;
pub const G_KNACK: u16     = 1 << 6;
pub const G_TEMPORAL: u16  = 1 << 7;
pub const G_ESSENCE: u16   = 1 << 8;
pub const G_FOIL: u16      = 1 << 9;
pub const G_STAT: u16      = 1 << 10;
pub const N_GROUP_BITS: usize = 11;
pub const ALL_CATEGORY_GROUPS: u16 = G_OFFENSIVE | G_DEFENSIVE | G_PHYSICAL
    | G_MAGICAL | G_UTILITY | G_RESOURCE | G_KNACK | G_TEMPORAL | G_ESSENCE;
/// Non-stat categories (author-confirmed): a card carrying Resource or
/// Temporal provides no player stats, so it scores **0 NDM itself**. It
/// still fills its slot (row/col/peer counts, n_ns, chain connectivity)
/// and feeds implicits that read it (merchant's column Resource count).
/// Consequently these tags are never blanket-assigned — they are per-slot
/// SA decisions ("battery" cards) via the assignable-groups toggle moves.
pub const NONSTAT_GROUPS: u16 = G_RESOURCE | G_TEMPORAL;

// Cores (same u8 space as the 1.x kernels).
pub const CORE_PURE: u8        = 0;
pub const CORE_EQUILIBRIUM: u8 = 1;
pub const CORE_STEADFAST: u8   = 2;
pub const CORE_COLOR: u8       = 3;
pub const CORE_FOIL: u8        = 4;
pub const CORE_DELUXE: u8      = 5;
pub const CORE_VOID: u8        = 6;
pub const CORE_ARCHIVE: u8     = 7;
pub const CORE_SPARKLING: u8   = 8;

// ─────────────────────────────────────────────────────────────────────────────
// String ↔ id conversions (boundary only)
// ─────────────────────────────────────────────────────────────────────────────

pub fn tag_card_type_from_str(s: &str) -> u8 {
    match s {
        "row" => T_ROW, "col" => T_COL, "surr" => T_SURR, "diag" => T_DIAG,
        "deluxe" => T_DELUXE, "typeless" => T_TYPELESS,
        "dir_greed_up" => T_G_UP, "dir_greed_down" => T_G_DOWN,
        "dir_greed_left" => T_G_LEFT, "dir_greed_right" => T_G_RIGHT,
        "arcane" => T_ARCANE, "wild" => T_WILD, "dead" => T_DEAD,
        other => panic!("tagsim: unknown card type: {}", other),
    }
}

pub fn tag_card_type_to_str(t: u8) -> &'static str {
    match t {
        T_ROW => "row", T_COL => "col", T_SURR => "surr", T_DIAG => "diag",
        T_DELUXE => "deluxe", T_TYPELESS => "typeless",
        T_G_UP => "dir_greed_up", T_G_DOWN => "dir_greed_down",
        T_G_LEFT => "dir_greed_left", T_G_RIGHT => "dir_greed_right",
        T_ARCANE => "arcane", T_WILD => "wild", T_DEAD => "dead",
        other => panic!("tagsim: unknown card type u8: {}", other),
    }
}

pub fn tag_color_from_str(s: &str) -> u8 {
    match s {
        "red" => RED, "green" => GREEN, "blue" => BLUE, "yellow" => YELLOW,
        "" => COLOR_NONE,
        other => panic!("tagsim: unknown color: {}", other),
    }
}

pub fn tag_color_to_str(c: u8) -> &'static str {
    match c {
        RED => "red", GREEN => "green", BLUE => "blue", YELLOW => "yellow",
        COLOR_NONE => "",
        other => panic!("tagsim: unknown color u8: {}", other),
    }
}

pub fn group_bit_from_str(s: &str) -> u16 {
    match s {
        "Offensive" => G_OFFENSIVE, "Defensive" => G_DEFENSIVE,
        "Physical" => G_PHYSICAL,   "Magical" => G_MAGICAL,
        "Utility" => G_UTILITY,     "Resource" => G_RESOURCE,
        "Knack" => G_KNACK,         "Temporal" => G_TEMPORAL,
        "Essence" => G_ESSENCE,     "Foil" => G_FOIL, "Stat" => G_STAT,
        other => panic!("tagsim: unknown group: {}", other),
    }
}

pub fn group_bits_to_strs(mask: u16) -> Vec<&'static str> {
    const NAMES: [(&str, u16); 11] = [
        ("Offensive", G_OFFENSIVE), ("Defensive", G_DEFENSIVE),
        ("Physical", G_PHYSICAL), ("Magical", G_MAGICAL),
        ("Utility", G_UTILITY), ("Resource", G_RESOURCE),
        ("Knack", G_KNACK), ("Temporal", G_TEMPORAL), ("Essence", G_ESSENCE),
        ("Foil", G_FOIL), ("Stat", G_STAT),
    ];
    NAMES.iter().filter(|(_, b)| mask & b != 0).map(|(n, _)| *n).collect()
}

pub fn tag_core_from_str(s: &str) -> u8 {
    match s {
        "pure" => CORE_PURE, "equilibrium" => CORE_EQUILIBRIUM,
        "steadfast" => CORE_STEADFAST, "sparkling" => CORE_SPARKLING,
        "color" => CORE_COLOR, "foil" => CORE_FOIL,
        "deluxe_core" => CORE_DELUXE, "void_core" => CORE_VOID,
        "archive_core" => CORE_ARCHIVE,
        other => panic!("tagsim: unknown core type: {}", other),
    }
}

#[inline(always)]
fn is_positional(t: u8) -> bool { t <= T_DIAG }
#[inline(always)]
fn is_scorable(t: u8) -> bool { t <= T_TYPELESS }
#[inline(always)]
fn is_greed(t: u8) -> bool { (T_G_UP..=T_G_RIGHT).contains(&t) }
/// Stat-giving set for the min-stat floor: positional + typeless, with
/// deluxe included only when the config says so (see `floor_counts_deluxe`).
#[inline(always)]
pub fn is_stat_giving(t: u8) -> bool { t <= T_TYPELESS }
/// Floor predicate over a placed CARD: stat-giving type AND not carrying a
/// non-stat tag (a Resource/Temporal card gives no stats — see
/// `NONSTAT_GROUPS`).
#[inline(always)]
fn counts_toward_floor(c: &SlotCard, cfg: &TagSimConfig) -> bool {
    if c.t == T_DEAD { return false; }
    let type_ok = if c.t == T_DELUXE { cfg.floor_counts_deluxe } else { is_stat_giving(c.t) };
    type_ok && c.groups & NONSTAT_GROUPS == 0
}

// ─────────────────────────────────────────────────────────────────────────────
// Implicits (spec §4, values from the woldsvaults datagen)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Implicit {
    /// GlobalDeckModifier: +value additive to cards carrying ALL `groups`
    /// bits AND (colors mask empty OR card color in mask).
    GlobalFlat { value: f64, groups: u16, colors: u8 },
    /// CardNeighborTypeDeckModifier: positional count of cards of type
    /// `ptype` is multiplied by round(value) (the MixinCardScaler hook).
    Freq { mult: f64, ptype: u8 },
    /// AdjacencyBonusDeckModifier: +value × (cards in range carrying `group`).
    /// `surrounding=false` → whole column (excl. self); true → 8-neighborhood.
    Adjacency { value: f64, group: u16, surrounding: bool },
    /// ColorMismatchAdjacencyModifier (puzzle): +value × (orthogonal
    /// neighbors with a different color). Blanket best-case when !colors_real.
    ColorMismatch { value: f64 },
    /// RowPositionDeckModifier (cake): +value × (rows from bottom, bottom=1).
    RowPos { value: f64 },
    /// ChainReactionDeckModifier (snake): +value × (same-color orthogonally
    /// connected component size − 1).
    Chain { value: f64 },
    /// EmptySlotDeckModifier (shadow): +value × dead-slot count.
    EmptySlots { value: f64 },
    /// UniqueGroupsDeckModifier (mutant): Stat cards get +value × (count of
    /// unique groups across all placed cards).
    UniqueGroups { value: f64 },
    /// SymmetricBalanceDeckModifier (runic): MULTIPLIES the whole per-card
    /// value by `value` when the horizontal-mirror slot holds a same-color
    /// card (center column auto-passes). Not additive.
    Mirror { value: f64 },
}

// ─────────────────────────────────────────────────────────────────────────────
// Run description
// ─────────────────────────────────────────────────────────────────────────────

/// One placeable card spec. `count == u32::MAX` means unlimited supply.
#[derive(Clone, Copy)]
pub struct CardSpec {
    pub t: u8,
    pub color: u8,        // COLOR_NONE only for DEAD
    pub scale: u8,        // == color unless Complex Cards
    pub groups: u16,      // per-stack REAL groups (Exact); 0 for Max/Targeted
    pub count: u32,       // supply cap; u32::MAX = unlimited
    pub min_place: u32,   // lower bound ("must place" / forced)
}

/// A per-tag constraint rule (Targeted). `max == u32::MAX` = uncapped.
#[derive(Clone, Copy)]
pub struct TagRule {
    pub axis: TagAxis,
    pub key: u16,         // color id, type id, or group bit per axis
    pub min: u32,
    pub max: u32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum TagAxis {
    Color,     // key = color id
    CardType,  // key = card type id
    Group,     // key = group bit
    GreedTotal, // key unused — total greed-card count
}

#[derive(Clone, Copy)]
pub struct CoreSpecIn {
    pub core_type: u8,
    pub color: u8,
    pub override_: f64,   // < 0 = none
}

pub struct TagSimConfig {
    pub mult_dir_vert: f64,
    pub mult_dir_horiz: f64,
    pub mult_pure_base: f64,
    pub mult_pure_scale: f64,
    pub mult_equilibrium: f64,
    pub mult_foil: f64,
    pub mult_steadfast: f64,
    pub mult_sparkling: f64,
    pub mult_color: f64,
    pub mult_deluxe_flat: f64,
    pub mult_deluxe_core_base: f64,
    pub mult_deluxe_core_scale: f64,
    pub mult_void_core_base: f64,
    pub mult_void_core_scale: f64,
    pub mult_archive_core: f64,
    pub greed_additive: bool,
    pub additive_cores: bool,
    pub is_shiny: bool,
    pub auto_place_arcane: bool,
    /// false → Max-style blanket color assumptions: positional counting is
    /// color-blind (all filled count), COLOR core + color-keyed implicits
    /// apply to every eligible card, puzzle assumes max mismatches, snake
    /// chains over filled connectivity, runic mirrors pass when filled.
    /// true → real per-card colors drive all of the above.
    pub colors_real: bool,
    /// Complex Cards (§7): greed boosts only targets whose card_color ==
    /// the greed's scale_color; positional cards count neighbors whose
    /// card_color == their OWN scale_color. Ignored unless colors_real.
    pub complex: bool,
    /// Wold's foil rule: shiny ⇒ foil. Vanilla sets false (§5).
    pub wv_foil_rules: bool,
    /// Whether DELUXE counts toward the min-stat floor. The web app's
    /// user-facing floor counts it (deluxe gives stats); the spreadsheet
    /// CLI mirrors classic `deluxe_counted_as_regular` (false by default).
    pub floor_counts_deluxe: bool,
}

pub struct TagRun<'a> {
    pub slots: &'a [(i32, i32)],
    pub row_peers: Vec<Vec<usize>>,
    pub col_peers: Vec<Vec<usize>>,
    pub surr_peers: Vec<Vec<usize>>,
    pub diag_peers: Vec<Vec<usize>>,
    pub arcane_slot_indices: Vec<usize>,
    pub stacks: Vec<CardSpec>,
    pub tag_rules: Vec<TagRule>,
    /// Groups every placed non-greed card carries for free (Max/Targeted
    /// blanket — the favorable assignment of NDM-inert tags).
    pub blanket_groups: u16,
    /// Capped implicit-relevant groups the SA may toggle per slot (Targeted).
    pub assignable_groups: u16,
    pub implicits: Vec<Implicit>,
    pub cores: Vec<CoreSpecIn>,
    pub min_stat_placed: u32,
    /// §6: after SA, replace wasted greeds with non-foil evo positionals
    /// (set only on EVO runs where the FOIL core is active).
    pub final_pass_nonfoil_evo: bool,
    /// Exact mode: per-card real groups come from stacks; no blanket.
    pub exact_groups: bool,
    pub n_iter: usize,
    pub restarts: usize,
    pub seed: Option<u64>,
    pub cfg: TagSimConfig,
}

// ─────────────────────────────────────────────────────────────────────────────
// Geometry
// ─────────────────────────────────────────────────────────────────────────────

struct Geom {
    n: usize,
    row_of: Vec<i32>,
    col_of: Vec<i32>,
    row_peers: Vec<Vec<usize>>,
    col_peers: Vec<Vec<usize>>,
    surr_peers: Vec<Vec<usize>>,
    diag_peers: Vec<Vec<usize>>,
    orth: Vec<[Option<usize>; 4]>,          // up, down, left, right
    is_arcane_slot: Vec<bool>,
    row_min: i32,
    row_span: usize,
    col_min: i32,
    col_span: usize,
    /// Horizontal mirror per slot (runic): mirrored col within the bbox,
    /// same row. None if the mirrored cell isn't a slot. `mirror_self` =
    /// the slot IS its own mirror (center column → auto-pass).
    mirror_of: Vec<Option<usize>>,
    mirror_self: Vec<bool>,
    /// Rows-from-bottom per slot (cake): bottom row = 1.
    rows_from_bottom: Vec<f64>,
}

fn build_geom(run: &TagRun<'_>) -> Geom {
    let n = run.slots.len();
    let slot_map: HashMap<(i32, i32), usize> =
        run.slots.iter().enumerate().map(|(i, &p)| (p, i)).collect();
    let row_of: Vec<i32> = run.slots.iter().map(|&(r, _)| r).collect();
    let col_of: Vec<i32> = run.slots.iter().map(|&(_, c)| c).collect();

    let dir = |i: usize, dr: i32, dc: i32| -> Option<usize> {
        slot_map.get(&(run.slots[i].0 + dr, run.slots[i].1 + dc)).copied()
    };
    let orth: Vec<[Option<usize>; 4]> = (0..n)
        .map(|i| [dir(i, -1, 0), dir(i, 1, 0), dir(i, 0, -1), dir(i, 0, 1)])
        .collect();

    let row_min = *row_of.iter().min().unwrap_or(&0);
    let row_max = *row_of.iter().max().unwrap_or(&0);
    let col_min = *col_of.iter().min().unwrap_or(&0);
    let col_max = *col_of.iter().max().unwrap_or(&0);

    let mut is_arcane_slot = vec![false; n];
    for &idx in &run.arcane_slot_indices {
        if idx < n { is_arcane_slot[idx] = true; }
    }

    // Runic mirror: game formula mirrors x (our COLUMN) across the deck bbox.
    let mut mirror_of = vec![None; n];
    let mut mirror_self = vec![false; n];
    for i in 0..n {
        let mc = col_max - (col_of[i] - col_min);
        if mc == col_of[i] {
            mirror_self[i] = true;
        } else {
            mirror_of[i] = slot_map.get(&(row_of[i], mc)).copied();
        }
    }

    // Cake row distance: game rows grow downward (y); our row index grows
    // downward too, so bottom row = row_max → distance row_max - r + 1.
    let rows_from_bottom: Vec<f64> =
        (0..n).map(|i| (row_max - row_of[i] + 1) as f64).collect();

    Geom {
        n, row_of, col_of,
        row_peers: run.row_peers.clone(),
        col_peers: run.col_peers.clone(),
        surr_peers: run.surr_peers.clone(),
        diag_peers: run.diag_peers.clone(),
        orth, is_arcane_slot,
        row_min, row_span: (row_max - row_min + 1) as usize,
        col_min, col_span: (col_max - col_min + 1) as usize,
        mirror_of, mirror_self, rows_from_bottom,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-slot card state
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub struct SlotCard {
    pub t: u8,
    pub color: u8,
    pub scale: u8,
    pub groups: u16,
    /// Index into `stacks` this card was drawn from (u16::MAX for DEAD).
    pub stack: u16,
}

pub const DEAD_CARD: SlotCard = SlotCard {
    t: T_DEAD, color: COLOR_NONE, scale: COLOR_NONE, groups: 0, stack: u16::MAX,
};

// Packed cores (mirrors CoresPack in inventory.rs). `has_color_core` is
// tracked separately from the color: in Max (colors_real=false) the COLOR
// core is color-blind — it applies to every scorable card even when the
// spec carries no concrete color.
struct Cores {
    list: Vec<CoreSpecIn>,
    color_core_color: u8,
    has_color_core: bool,
    foil_active: bool,
}

impl Cores {
    fn build(specs: &[CoreSpecIn]) -> Self {
        let mut color_core_color = COLOR_NONE;
        let mut has_color_core = false;
        let mut foil_active = false;
        for s in specs {
            match s.core_type {
                CORE_COLOR => { color_core_color = s.color; has_color_core = true; }
                CORE_FOIL => foil_active = true,
                _ => {}
            }
        }
        Cores { list: specs.to_vec(), color_core_color, has_color_core, foil_active }
    }
}

// Scratch buffers reused across simulate() calls (zero-alloc hot path).
struct Scratch {
    row_color: Vec<u32>,     // row_span × N_COLORS same-color counts
    col_color: Vec<u32>,     // col_span × N_COLORS
    row_fill: Vec<u32>,      // row_span — color-blind filled counts
    col_fill: Vec<u32>,      // col_span
    boost: Vec<f64>,
    chain_id: Vec<u32>,      // snake component labels (0 = unlabeled)
    chain_size: Vec<u32>,    // per component id (1-indexed)
    chain_stack: Vec<usize>,
}

impl Scratch {
    fn new(geom: &Geom) -> Self {
        Scratch {
            row_color: vec![0; geom.row_span * N_COLORS],
            col_color: vec![0; geom.col_span * N_COLORS],
            row_fill: vec![0; geom.row_span],
            col_fill: vec![0; geom.col_span],
            boost: vec![1.0; geom.n],
            chain_id: vec![0; geom.n],
            chain_size: vec![0; geom.n + 1],
            chain_stack: Vec::with_capacity(geom.n),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// simulate() — full-assignment scoring with tags + implicits
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn simulate(
    geom: &Geom,
    asgn: &[SlotCard],
    cores: &Cores,
    implicits: &[Implicit],
    cfg: &TagSimConfig,
    s: &mut Scratch,
) -> f64 {
    let n = geom.n;

    for v in s.row_color.iter_mut() { *v = 0; }
    for v in s.col_color.iter_mut() { *v = 0; }
    for v in s.row_fill.iter_mut() { *v = 0; }
    for v in s.col_fill.iter_mut() { *v = 0; }

    // ── Pass 1: classify + row/col counts ────────────────────────────────
    let mut n_deluxe = 0usize;
    let mut n_arcane = 0usize;
    let mut n_greed = 0usize;
    let mut n_dead = 0usize;
    let mut n_ns_positional = 0usize;   // NON-FOIL positionals (per-card §5)
    let mut groups_union: u16 = 0;      // for mutant unique-groups
    let mut any_wild = false;

    for i in 0..n {
        let c = asgn[i];
        if c.t == T_DEAD { n_dead += 1; continue; }
        let r = (geom.row_of[i] - geom.row_min) as usize;
        let cc = (geom.col_of[i] - geom.col_min) as usize;
        s.row_fill[r] += 1;
        s.col_fill[cc] += 1;
        if c.t == T_WILD {
            // Wild counts as every color for neighbors' same-color scans.
            for k in 0..N_COLORS {
                s.row_color[r * N_COLORS + k] += 1;
                s.col_color[cc * N_COLORS + k] += 1;
            }
            any_wild = true;
            groups_union |= ALL_CATEGORY_GROUPS | G_STAT;
        } else if c.color != COLOR_NONE {
            s.row_color[r * N_COLORS + c.color as usize] += 1;
            s.col_color[cc * N_COLORS + c.color as usize] += 1;
        }
        groups_union |= c.groups;
        if is_positional(c.t) {
            if c.groups & G_FOIL == 0 { n_ns_positional += 1; }
        } else if c.t == T_DELUXE { n_deluxe += 1; }
        else if c.t == T_ARCANE { n_arcane += 1; }
        else if is_greed(c.t) { n_greed += 1; }
    }

    // n_ns (Pure): greed + arcane always; positionals only while non-foil.
    // Typeless / deluxe never (classic-kernel design, preserved).
    let n_ns = n_greed + n_arcane + n_ns_positional;

    // ── Cores → baseline + gated addends (identical math to 1.x) ─────────
    let mut baseline_sum = 0.0f64;
    let mut baseline_prod = 1.0f64;
    let mut color_addend = 0.0f64;
    let mut color_factor = 1.0f64;
    let mut deluxe_addend = 0.0f64;
    let mut deluxe_factor = 1.0f64;
    let mut deluxe_present = false;
    let mut void_addend = 0.0f64;
    let mut void_factor = 1.0f64;
    let mut void_present = false;
    let mut archive_base = 1.0f64;
    let mut archive_present = false;

    for spec in &cores.list {
        let ov = spec.override_ >= 0.0;
        match spec.core_type {
            CORE_PURE => {
                let scale = if ov { spec.override_ } else { cfg.mult_pure_scale };
                let v = cfg.mult_pure_base + scale * n_ns as f64;
                baseline_sum += v - 1.0;
                baseline_prod *= v;
            }
            CORE_EQUILIBRIUM if cfg.is_shiny => {
                let v = if ov { spec.override_ } else { cfg.mult_equilibrium };
                baseline_sum += v - 1.0; baseline_prod *= v;
            }
            CORE_STEADFAST if cfg.is_shiny => {
                let v = if ov { spec.override_ } else { cfg.mult_steadfast };
                baseline_sum += v - 1.0; baseline_prod *= v;
            }
            CORE_SPARKLING if cfg.is_shiny => {
                let v = if ov { spec.override_ } else { cfg.mult_sparkling };
                baseline_sum += v - 1.0; baseline_prod *= v;
            }
            CORE_FOIL => {
                let v = if ov { spec.override_ } else { cfg.mult_foil };
                baseline_sum += v - 1.0; baseline_prod *= v;
            }
            CORE_COLOR => {
                let v = if ov { spec.override_ } else { cfg.mult_color };
                color_addend = v - 1.0; color_factor = v;
            }
            CORE_DELUXE => {
                let scale = if ov { spec.override_ } else { cfg.mult_deluxe_core_scale };
                let v = cfg.mult_deluxe_core_base + scale * n_deluxe as f64;
                deluxe_addend = v - 1.0; deluxe_factor = v; deluxe_present = true;
            }
            CORE_VOID => {
                let scale = if ov { spec.override_ } else { cfg.mult_void_core_scale };
                let v = cfg.mult_void_core_base + scale * n_dead as f64;
                void_addend = v - 1.0; void_factor = v; void_present = true;
            }
            CORE_ARCHIVE => {
                archive_base = if ov { spec.override_ } else { cfg.mult_archive_core };
                archive_present = true;
            }
            _ => {}
        }
    }
    let archive_mult = if archive_present { archive_base.powi(n_arcane as i32) } else { 1.0 };
    let color_core_color = cores.color_core_color;

    // ── Implicit precompute (deck-global parts) ──────────────────────────
    // Positional frequency multipliers (rook/pillager/bishop).
    let mut freq_mult = [1.0f64; 4];  // per positional type
    // Additive global-flat implicits, filtered per card at accumulate time.
    // Chain labeling (snake): flood-fill same-color orthogonal components.
    let mut chain_value = 0.0f64;
    let mut have_chain = false;
    let mut empty_addend = 0.0f64;
    let mut unique_value = 0.0f64;
    let mut have_unique = false;
    let mut mirror_value = 1.0f64;
    let mut have_mirror = false;

    for imp in implicits {
        match *imp {
            Implicit::Freq { mult, ptype } => {
                freq_mult[ptype as usize] *= mult.round().max(1.0);
            }
            Implicit::Chain { value } => { chain_value = value; have_chain = true; }
            Implicit::EmptySlots { value } => { empty_addend += value * n_dead as f64; }
            Implicit::UniqueGroups { value } => { unique_value = value; have_unique = true; }
            Implicit::Mirror { value } => { mirror_value = value; have_mirror = true; }
            _ => {}
        }
    }

    // Unique-groups count (mutant): union of category/Foil/Stat bits on
    // placed cards + class markers implied by the placement.
    let unique_groups_count = if have_unique {
        let mut cnt = groups_union.count_ones() as f64;
        cnt += 1.0;                                  // Shiny or Evolution marker
        if n_deluxe > 0 { cnt += 1.0; }              // Deluxe marker
        if n_arcane > 0 { cnt += 1.0; }              // Arcane marker
        if n_greed > 0 { cnt += 1.0; }               // Greed marker
        if any_wild { cnt += 1.0; }                  // Evolution+Stat combo card
        cnt
    } else { 0.0 };

    // Snake chain labeling — one flood-fill pass over non-dead cards.
    // colors_real: components are same-color (wild bridges any color).
    // blanket (Max): components are filled-connectivity (mono assumption).
    if have_chain {
        for v in s.chain_id.iter_mut() { *v = 0; }
        let mut next_id = 0u32;
        for start in 0..n {
            if asgn[start].t == T_DEAD || s.chain_id[start] != 0 { continue; }
            next_id += 1;
            let mut size = 0u32;
            s.chain_stack.clear();
            s.chain_stack.push(start);
            s.chain_id[start] = next_id;
            while let Some(i) = s.chain_stack.pop() {
                size += 1;
                for d in 0..4 {
                    if let Some(j) = geom.orth[i][d] {
                        if asgn[j].t == T_DEAD || s.chain_id[j] != 0 { continue; }
                        let same = if !cfg.colors_real {
                            true
                        } else {
                            let a = asgn[i]; let b = asgn[j];
                            a.t == T_WILD || b.t == T_WILD
                                || (a.color != COLOR_NONE && a.color == b.color)
                        };
                        if same {
                            s.chain_id[j] = next_id;
                            s.chain_stack.push(j);
                        }
                    }
                }
            }
            s.chain_size[next_id as usize] = size;
        }
    }

    // ── Greed boost pass (orthogonal only, §2.3) ─────────────────────────
    for v in s.boost[..n].iter_mut() { *v = 1.0; }
    for i in 0..n {
        let g = asgn[i];
        if !is_greed(g.t) { continue; }
        let (target, amount) = match g.t {
            T_G_UP => (geom.orth[i][0], cfg.mult_dir_vert),
            T_G_DOWN => (geom.orth[i][1], cfg.mult_dir_vert),
            T_G_LEFT => (geom.orth[i][2], cfg.mult_dir_horiz),
            T_G_RIGHT => (geom.orth[i][3], cfg.mult_dir_horiz),
            _ => (None, 0.0),
        };
        if let Some(j) = target {
            if is_scorable(asgn[j].t) {
                // Complex Cards: greed boosts only its scale_color (§7).
                // Non-Complex: color-agnostic (current behavior, gate-safe).
                let color_ok = !(cfg.colors_real && cfg.complex)
                    || asgn[j].t == T_WILD
                    || asgn[j].color == g.scale;
                if color_ok {
                    if cfg.greed_additive { s.boost[j] += amount; }
                    else { s.boost[j] *= amount; }
                }
            }
        }
    }

    // ── Accumulate NDM ───────────────────────────────────────────────────
    let mut ndm = 0.0f64;
    for i in 0..n {
        let c = asgn[i];
        if !is_scorable(c.t) { continue; }
        // Non-stat card (Resource/Temporal): 0 NDM itself. It was already
        // counted as a filled neighbor / n_ns / implicit-feeder above.
        if c.groups & NONSTAT_GROUPS != 0 { continue; }

        // Positional base value.
        let base = if is_positional(c.t) {
            let scan_color = if cfg.complex && cfg.colors_real { c.scale } else { c.color };
            let raw = match c.t {
                T_ROW => {
                    let r = (geom.row_of[i] - geom.row_min) as usize;
                    if cfg.colors_real {
                        // Self counts iff its card_color matches the scan
                        // color (under Complex, scale≠color ⇒ no self-count —
                        // mirrors the game's entry filter).
                        s.row_color[r * N_COLORS + scan_color as usize] as f64
                    } else {
                        s.row_fill[r] as f64
                    }
                }
                T_COL => {
                    let cc = (geom.col_of[i] - geom.col_min) as usize;
                    if cfg.colors_real {
                        s.col_color[cc * N_COLORS + scan_color as usize] as f64
                    } else {
                        s.col_fill[cc] as f64
                    }
                }
                T_DIAG => {
                    let mut count = 0.0;
                    for &q in &geom.diag_peers[i] {
                        let qc = asgn[q];
                        if qc.t == T_DEAD { continue; }
                        let m = if !cfg.colors_real { true }
                            else { qc.t == T_WILD || qc.color == scan_color };
                        if m { count += 1.0; }
                    }
                    count
                }
                T_SURR => {
                    let mut count = 0.0;
                    for &q in &geom.surr_peers[i] {
                        let qc = asgn[q];
                        if qc.t == T_DEAD { continue; }
                        let m = if !cfg.colors_real { true }
                            else { qc.t == T_WILD || qc.color == scan_color };
                        if m { count += 1.0; }
                    }
                    count
                }
                _ => 0.0,
            };
            // Frequency implicit (rook/pillager/bishop) multiplies the count;
            // DIAG keeps its lone-card ≥1 floor after the multiplier.
            let scaled = raw * freq_mult[c.t as usize];
            if c.t == T_DIAG { scaled.max(1.0) } else { scaled }
        } else if c.t == T_DELUXE {
            cfg.mult_deluxe_flat
        } else {
            1.0   // TYPELESS
        };

        // Core multiplier + additive implicits. Color-blind runs apply the
        // COLOR core flat (classic behavior — the parity gate depends on it);
        // color-real runs gate on the card's own color.
        let color_applies = cores.has_color_core
            && (!cfg.colors_real
                || (color_core_color != COLOR_NONE
                    && (c.t == T_WILD || c.color == color_core_color)));
        let deluxe_applies = deluxe_present && c.t != T_DELUXE;
        let void_applies = void_present;

        // Additive implicit addends for THIS card.
        let mut imp_addend = 0.0f64;
        for imp in implicits {
            match *imp {
                Implicit::GlobalFlat { value, groups, colors } => {
                    let group_ok = c.groups & groups == groups;
                    let color_ok = colors == 0
                        || !cfg.colors_real
                        || (c.color != COLOR_NONE && colors & (1 << c.color) != 0);
                    if group_ok && color_ok { imp_addend += value; }
                }
                Implicit::Adjacency { value, group, surrounding } => {
                    let peers: &Vec<usize> =
                        if surrounding { &geom.surr_peers[i] } else { &geom.col_peers[i] };
                    let mut matches = 0u32;
                    for &q in peers {
                        let qc = asgn[q];
                        if qc.t == T_DEAD || is_greed(qc.t) { continue; }
                        if qc.t == T_WILD || qc.groups & group != 0 { matches += 1; }
                    }
                    imp_addend += value * matches as f64;
                }
                Implicit::ColorMismatch { value } => {
                    let mut mism = 0u32;
                    for d in 0..4 {
                        if let Some(j) = geom.orth[i][d] {
                            let qc = asgn[j];
                            if qc.t == T_DEAD { continue; }
                            if !cfg.colors_real {
                                mism += 1;   // blanket best case (§4)
                            } else if qc.t == T_WILD
                                || (qc.color != COLOR_NONE && qc.color != c.color) {
                                mism += 1;   // wild counts favorably
                            }
                        }
                    }
                    imp_addend += value * mism as f64;
                }
                Implicit::RowPos { value } => {
                    imp_addend += value * geom.rows_from_bottom[i];
                }
                Implicit::Chain { .. } => {
                    let id = s.chain_id[i] as usize;
                    if id != 0 {
                        imp_addend += chain_value * (s.chain_size[id] - 1) as f64;
                    }
                }
                Implicit::UniqueGroups { .. } => {
                    if c.groups & G_STAT != 0 {
                        imp_addend += unique_value * unique_groups_count;
                    }
                }
                _ => {}
            }
        }
        imp_addend += empty_addend;   // shadow applies to every scoring card

        let core_mult = if cfg.additive_cores {
            1.0 + baseline_sum
                + if color_applies { color_addend } else { 0.0 }
                + if deluxe_applies { deluxe_addend } else { 0.0 }
                + if void_applies { void_addend } else { 0.0 }
                + imp_addend
        } else {
            // Vanilla path — implicits never active there; keep the pure
            // multiplicative composition identical to 1.x.
            let mut m = baseline_prod;
            if color_applies { m *= color_factor; }
            if deluxe_applies { m *= deluxe_factor; }
            if void_applies { m *= void_factor; }
            m
        };

        // Runic (multiplicative mirror).
        let mirror_factor = if have_mirror {
            let pass = if geom.mirror_self[i] {
                true
            } else if let Some(mi) = geom.mirror_of[i] {
                let mc = asgn[mi];
                if mc.t == T_DEAD { false }
                else if !cfg.colors_real { true }
                else { mc.t == T_WILD || (mc.color != COLOR_NONE && mc.color == c.color) }
            } else { false };
            if pass { mirror_value } else { 1.0 }
        } else { 1.0 };

        let b = if cfg.greed_additive { s.boost[i].max(1.0) } else { s.boost[i] };
        ndm += base * core_mult * b * archive_mult * mirror_factor;
    }

    ndm
}

// ─────────────────────────────────────────────────────────────────────────────
// Tag accounting (Targeted rules)
// ─────────────────────────────────────────────────────────────────────────────

struct RuleBook {
    rules: Vec<TagRule>,
    counts: Vec<i64>,
}

impl RuleBook {
    fn new(rules: &[TagRule]) -> Self {
        RuleBook { rules: rules.to_vec(), counts: vec![0; rules.len()] }
    }

    /// Which rules a card contributes to (multi-tag counting, spec §3.2).
    /// DEAD contributes to nothing. Wild counts toward its own type + its
    /// literal color only (documented modeling choice).
    fn card_delta(&self, c: &SlotCard, out: &mut Vec<usize>) {
        out.clear();
        if c.t == T_DEAD { return; }
        for (ri, r) in self.rules.iter().enumerate() {
            let hit = match r.axis {
                TagAxis::Color => c.color != COLOR_NONE && r.key == c.color as u16,
                TagAxis::CardType => r.key == c.t as u16,
                TagAxis::Group => c.t != T_WILD && c.groups & r.key != 0,
                TagAxis::GreedTotal => is_greed(c.t),
            };
            if hit { out.push(ri); }
        }
    }

    fn seed(&mut self, asgn: &[SlotCard]) {
        for v in self.counts.iter_mut() { *v = 0; }
        let mut tmp = Vec::new();
        for c in asgn {
            self.card_delta(c, &mut tmp);
            for &ri in &tmp { self.counts[ri] += 1; }
        }
    }

    /// Can we remove `old` and add `new`? (either may be DEAD)
    fn move_ok(&self, old: &SlotCard, new: &SlotCard, tmp_old: &mut Vec<usize>, tmp_new: &mut Vec<usize>) -> bool {
        self.card_delta(old, tmp_old);
        self.card_delta(new, tmp_new);
        for (ri, r) in self.rules.iter().enumerate() {
            let mut delta = 0i64;
            if tmp_old.contains(&ri) { delta -= 1; }
            if tmp_new.contains(&ri) { delta += 1; }
            if delta == 0 { continue; }
            let next = self.counts[ri] + delta;
            if next > r.max as i64 { return false; }
            if next < r.min as i64 { return false; }
        }
        true
    }

    fn apply(&mut self, old: &SlotCard, new: &SlotCard, tmp: &mut Vec<usize>) {
        self.card_delta(old, tmp);
        for &ri in tmp.iter() { self.counts[ri] -= 1; }
        self.card_delta(new, tmp);
        for &ri in tmp.iter() { self.counts[ri] += 1; }
    }

    /// For group-toggle moves: check + apply a single group-bit flip.
    fn toggle_ok(&self, bit: u16, adding: bool) -> bool {
        for (ri, r) in self.rules.iter().enumerate() {
            if r.axis != TagAxis::Group || r.key != bit { continue; }
            let next = self.counts[ri] + if adding { 1 } else { -1 };
            if next > r.max as i64 || next < r.min as i64 { return false; }
        }
        true
    }

    fn toggle_apply(&mut self, bit: u16, adding: bool) {
        for (ri, r) in self.rules.iter().enumerate() {
            if r.axis != TagAxis::Group || r.key != bit { continue; }
            self.counts[ri] += if adding { 1 } else { -1 };
        }
    }

    /// Group-rule minimum deficit (used by init fill for group mins).
    fn group_min_deficit(&self, bit: u16) -> i64 {
        for (ri, r) in self.rules.iter().enumerate() {
            if r.axis == TagAxis::Group && r.key == bit {
                return (r.min as i64 - self.counts[ri]).max(0);
            }
        }
        0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Card materialization — build the SlotCard a stack places on a slot
// ─────────────────────────────────────────────────────────────────────────────

/// Run-level foil rule (§5): which placed cards carry the Foil bit.
/// Applied on top of stack groups unless `exact_groups` (Exact mode builds
/// foil per card, with shiny⇒foil enforced in the UI builder).
fn run_foil_groups(t: u8, cfg: &TagSimConfig, foil_core_active: bool, foil_banned: bool) -> u16 {
    if foil_banned { return 0; }
    let foil = if cfg.is_shiny { cfg.wv_foil_rules } else { foil_core_active };
    if foil && (is_scorable(t) || t == T_ARCANE) { G_FOIL } else { 0 }
}

fn stat_groups(t: u8, cfg: &TagSimConfig) -> u16 {
    // Stat is shiny-only (spec §2.2); Max/Targeted assume ideal stat-bearing
    // shiny cards. Only stat-giving types carry it (greed/arcane never).
    if cfg.is_shiny && is_stat_giving(t) { G_STAT } else { 0 }
}

fn materialize(
    spec: &CardSpec,
    stack_idx: usize,
    run: &TagRun<'_>,
    foil_core_active: bool,
    foil_banned: bool,
) -> SlotCard {
    let mut groups = spec.groups;
    if !run.exact_groups {
        if is_scorable(spec.t) || spec.t == T_ARCANE {
            groups |= run.blanket_groups & !run.assignable_groups;
        }
        groups |= run_foil_groups(spec.t, &run.cfg, foil_core_active, foil_banned);
        if is_greed(spec.t) { groups = 0; }   // greed cards carry no groups
    }
    // Stat is run-derived in EVERY mode (shiny ⇒ stat-giving cards carry it,
    // evo ⇒ never) — it stopped being a user-facing tag.
    groups |= stat_groups(spec.t, &run.cfg);
    SlotCard {
        t: spec.t,
        color: spec.color,
        scale: if run.cfg.complex { spec.scale } else { spec.color },
        groups,
        stack: stack_idx as u16,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Initial fill
// ─────────────────────────────────────────────────────────────────────────────

const FILL_ORDER: [u8; 6] = [T_SURR, T_ROW, T_COL, T_DIAG, T_DELUXE, T_TYPELESS];

fn slot_ranking(geom: &Geom, t: u8) -> Vec<usize> {
    let peer_count = |i: usize| -> usize {
        match t {
            T_ROW => geom.row_peers[i].len(),
            T_COL => geom.col_peers[i].len(),
            T_SURR => geom.surr_peers[i].len(),
            T_DIAG => geom.diag_peers[i].len(),
            _ => 0,
        }
    };
    let mut idx: Vec<usize> = (0..geom.n).collect();
    idx.sort_by(|&a, &b| peer_count(b).cmp(&peer_count(a)));
    idx
}

/// Greedy initial fill honoring: arcane slots first, per-stack min_place,
/// then priority fill from remaining supply, all under the rule book.
fn initial_fill(
    geom: &Geom,
    run: &TagRun<'_>,
    materialized: &[SlotCard],   // per-stack template cards
    remaining: &mut [u32],       // per-stack remaining supply
    rules: &mut RuleBook,
) -> Vec<SlotCard> {
    let n = geom.n;
    let mut asgn = vec![DEAD_CARD; n];
    let mut filled = vec![false; n];
    let mut tmp_old = Vec::new();
    let mut tmp_new = Vec::new();

    let mut try_place = |slot: usize,
                         stack_idx: usize,
                         asgn: &mut Vec<SlotCard>,
                         filled: &mut Vec<bool>,
                         remaining: &mut [u32],
                         rules: &mut RuleBook| -> bool {
        if filled[slot] || remaining[stack_idx] == 0 { return false; }
        let card = materialized[stack_idx];
        if !rules.move_ok(&DEAD_CARD, &card, &mut tmp_old, &mut tmp_new) { return false; }
        rules.apply(&DEAD_CARD, &card, &mut tmp_old);
        asgn[slot] = card;
        filled[slot] = true;
        if remaining[stack_idx] != u32::MAX { remaining[stack_idx] -= 1; }
        true
    };

    // Phase 0: arcane slots — biggest-supply ARCANE stack first.
    for slot in 0..n {
        if !geom.is_arcane_slot[slot] { continue; }
        let mut best: Option<usize> = None;
        let mut best_avail = 0u32;
        for (si, spec) in run.stacks.iter().enumerate() {
            if spec.t == T_ARCANE && remaining[si] > best_avail {
                best_avail = remaining[si];
                best = Some(si);
            }
        }
        if let Some(si) = best {
            try_place(slot, si, &mut asgn, &mut filled, remaining, rules);
        }
        filled[slot] = true;   // arcane slots never fall through to Phase 1/2
    }

    // Phase 1: per-stack min_place (forced / must-place), geometric ranking
    // for positionals, plain order otherwise.
    for (si, spec) in run.stacks.iter().enumerate() {
        let mut need = spec.min_place;
        if need == 0 || spec.t == T_ARCANE { continue; }
        let ranking = if is_positional(spec.t) {
            slot_ranking(geom, spec.t)
        } else {
            (0..n).collect()
        };
        for &slot in &ranking {
            if need == 0 { break; }
            if geom.is_arcane_slot[slot] { continue; }
            if try_place(slot, si, &mut asgn, &mut filled, remaining, rules) {
                need -= 1;
            }
        }
    }

    // Phase 2: priority fill — positional/deluxe/typeless from the biggest
    // stacks first (mirror of the 1.x initial_fill heuristic).
    for &t in &FILL_ORDER {
        let ranking = slot_ranking(geom, t);
        let mut stack_order: Vec<usize> = (0..run.stacks.len())
            .filter(|&si| run.stacks[si].t == t)
            .collect();
        stack_order.sort_by(|&a, &b| remaining[b].cmp(&remaining[a]));
        for si in stack_order {
            for &slot in &ranking {
                if remaining[si] == 0 { break; }
                if geom.is_arcane_slot[slot] || filled[slot] { continue; }
                if !try_place(slot, si, &mut asgn, &mut filled, remaining, rules) { break; }
            }
        }
    }

    // Phase 3: group minimums (Targeted Min column) — toggle assignable
    // bits onto placed scorable cards until each group min is met. Free
    // because the bits are NDM-inert (they only matter via implicits).
    if run.assignable_groups != 0 {
        for bit_idx in 0..N_GROUP_BITS {
            let bit = 1u16 << bit_idx;
            if run.assignable_groups & bit == 0 { continue; }
            let mut deficit = rules.group_min_deficit(bit);
            for i in 0..n {
                if deficit <= 0 { break; }
                let c = asgn[i];
                if !is_scorable(c.t) || c.groups & bit != 0 { continue; }
                if rules.toggle_ok(bit, true) {
                    asgn[i].groups |= bit;
                    rules.toggle_apply(bit, true);
                    deficit -= 1;
                }
            }
        }
    }

    asgn
}

// ─────────────────────────────────────────────────────────────────────────────
// §6 final pass — replace wasted greeds with non-foil evo positionals
// ─────────────────────────────────────────────────────────────────────────────

fn final_pass_nonfoil_evo(
    geom: &Geom,
    run: &TagRun<'_>,
    asgn: &mut Vec<SlotCard>,
    score: &mut f64,
    cores: &Cores,
    rules: &mut RuleBook,
    remaining: &mut [u32],
    s: &mut Scratch,
) {
    // Wasted greed (§11-H): the orthogonal target is off-deck or holds a
    // non-scorable card (greed / arcane / wild / dead).
    let mut tmp_old = Vec::new();
    let mut tmp_new = Vec::new();
    for i in 0..geom.n {
        let g = asgn[i];
        if !is_greed(g.t) { continue; }
        let target = match g.t {
            T_G_UP => geom.orth[i][0],
            T_G_DOWN => geom.orth[i][1],
            T_G_LEFT => geom.orth[i][2],
            T_G_RIGHT => geom.orth[i][3],
            _ => None,
        };
        let wasted = match target {
            None => true,
            Some(j) => !is_scorable(asgn[j].t),
        };
        if !wasted { continue; }

        // Try each positional type from supply, WITHOUT the foil bit; keep
        // the best strict improvement.
        let mut best: Option<(SlotCard, f64, usize)> = None;
        for (si, spec) in run.stacks.iter().enumerate() {
            if !is_positional(spec.t) || remaining[si] == 0 { continue; }
            let mut card = materialize(spec, si, run, cores.foil_active, false);
            card.groups &= !G_FOIL;   // the whole point: non-foil evo feeds Pure
            if !rules.move_ok(&g, &card, &mut tmp_old, &mut tmp_new) { continue; }
            let old = asgn[i];
            asgn[i] = card;
            let sc = simulate(geom, asgn, cores, &run.implicits, &run.cfg, s);
            asgn[i] = old;
            if sc > *score && best.as_ref().map_or(true, |b| sc > b.1) {
                best = Some((card, sc, si));
            }
        }
        if let Some((card, sc, si)) = best {
            rules.apply(&g, &card, &mut tmp_old);
            if remaining[si] != u32::MAX { remaining[si] -= 1; }
            // Freed greed goes back to its stack when finite.
            let gsi = g.stack as usize;
            if gsi < remaining.len() && remaining[gsi] != u32::MAX {
                remaining[gsi] += 1;
            }
            asgn[i] = card;
            *score = sc;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SA — one restart
// ─────────────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn sa_one_restart(
    geom: &Geom,
    run: &TagRun<'_>,
    cores: &Cores,
    materialized: &[SlotCard],
    t_start: f64,
    t_end: f64,
    seed: u64,
) -> (Vec<SlotCard>, f64) {
    let mut rng = SmallRng::seed_from_u64(seed);
    let n = geom.n;
    let cfg = &run.cfg;

    let mut remaining: Vec<u32> = run.stacks.iter().map(|sp| sp.count).collect();
    let mut rules = RuleBook::new(&run.tag_rules);
    let mut asgn = initial_fill(geom, run, materialized, &mut remaining, &mut rules);

    // Stat floor bookkeeping (min_stat_placed).
    let mut stat_placed: u32 = asgn.iter()
        .filter(|c| counts_toward_floor(c, cfg)).count() as u32;
    let effective_min_stat = run.min_stat_placed.min(stat_placed);

    // Per-stack min accounting: placed count per stack (for min_place floors).
    let mut placed_per_stack: Vec<u32> = vec![0; run.stacks.len()];
    for c in &asgn {
        if c.t != T_DEAD && (c.stack as usize) < placed_per_stack.len() {
            placed_per_stack[c.stack as usize] += 1;
        }
    }

    let mut s = Scratch::new(geom);
    let mut score = simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s);
    let mut best_score = score;
    let mut best_asgn = asgn.clone();

    // Proposal alphabets.
    let mut regular_options: Vec<usize> = Vec::new();
    let mut arcane_options: Vec<usize> = Vec::new();
    for (si, spec) in run.stacks.iter().enumerate() {
        if spec.t == T_ARCANE { arcane_options.push(si); }
        else { regular_options.push(si); }
    }
    let dead_option = usize::MAX;

    // Assignable group bits (Targeted toggle moves).
    let assignable_bits: Vec<u16> = (0..N_GROUP_BITS)
        .map(|b| 1u16 << b)
        .filter(|b| run.assignable_groups & b != 0)
        .collect();
    let toggle_prob = if assignable_bits.is_empty() { 0.0 } else { 0.15 };

    // Locked arcane slots under auto-place (same rule as 1.x).
    let mut locked: Vec<bool> = vec![false; n];
    if cfg.auto_place_arcane {
        for i in 0..n {
            if geom.is_arcane_slot[i] { locked[i] = true; }
        }
    }

    let log_cool = (t_end / t_start).ln();
    let mut tmp_old = Vec::new();
    let mut tmp_new = Vec::new();

    for it in 0..run.n_iter {
        let temperature = {
            let raw = t_start * (log_cool * it as f64 / run.n_iter as f64).exp();
            if raw < 1e-10 { 1e-10 } else { raw }
        };
        let roll: f64 = rng.gen();

        if roll < toggle_prob {
            // ── Toggle move: flip an assignable group bit on a slot ──────
            let p = rng.gen_range(0..n);
            let c = asgn[p];
            if !is_scorable(c.t) { continue; }
            let bit = assignable_bits[rng.gen_range(0..assignable_bits.len())];
            let adding = c.groups & bit == 0;
            if !rules.toggle_ok(bit, adding) { continue; }
            // Non-stat bits (Resource/Temporal) flip the card's floor status.
            let mut flipped = c;
            flipped.groups ^= bit;
            let stat_delta = (counts_toward_floor(&flipped, cfg) as i32)
                - (counts_toward_floor(&c, cfg) as i32);
            if effective_min_stat > 0 && stat_delta < 0
                && (stat_placed as i32 + stat_delta) < effective_min_stat as i32 {
                continue;
            }
            asgn[p].groups ^= bit;
            let new_score = simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s);
            let delta = new_score - score;
            if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                rules.toggle_apply(bit, adding);
                stat_placed = (stat_placed as i32 + stat_delta) as u32;
                score = new_score;
                if score > best_score { best_score = score; best_asgn = asgn.clone(); }
            } else {
                asgn[p].groups ^= bit;
            }
        } else if n < 2 || roll < 0.80 + toggle_prob * 0.25 {
            // ── Replace move ─────────────────────────────────────────────
            let p = rng.gen_range(0..n);
            if locked[p] {
                // Auto-place ON: arcane slots may still swap colors among
                // arcane stacks (mirrors 1.x behavior).
                if arcane_options.len() < 2 { continue; }
                let si = arcane_options[rng.gen_range(0..arcane_options.len())];
                let old = asgn[p];
                if old.t != T_ARCANE { continue; }   // stayed DEAD from init (no supply)
                let new = materialize(&run.stacks[si], si, run, cores.foil_active, false);
                if new == old { continue; }
                if remaining[si] == 0 { continue; }
                if !rules.move_ok(&old, &new, &mut tmp_old, &mut tmp_new) { continue; }
                let old_si = old.stack as usize;
                rules.apply(&old, &new, &mut tmp_old);
                if remaining[si] != u32::MAX { remaining[si] -= 1; }
                if old_si < remaining.len() && remaining[old_si] != u32::MAX { remaining[old_si] += 1; }
                asgn[p] = new;
                let new_score = simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s);
                let delta = new_score - score;
                if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                    score = new_score;
                    if score > best_score { best_score = score; best_asgn = asgn.clone(); }
                } else {
                    rules.apply(&new, &old, &mut tmp_old);
                    if remaining[si] != u32::MAX { remaining[si] += 1; }
                    if old_si < remaining.len() && remaining[old_si] != u32::MAX { remaining[old_si] -= 1; }
                    asgn[p] = old;
                }
                continue;
            }

            let old = asgn[p];
            // Choose from: regular stacks (or arcane stacks for arcane
            // slots when auto-place OFF) + DEAD.
            let (opts, allow_dead) = if geom.is_arcane_slot[p] {
                (&arcane_options, true)
            } else {
                (&regular_options, true)
            };
            let pick = rng.gen_range(0..opts.len() + usize::from(allow_dead));
            let si = if pick == opts.len() { dead_option } else { opts[pick] };

            let new = if si == dead_option {
                DEAD_CARD
            } else {
                if remaining[si] == 0 { continue; }
                materialize(&run.stacks[si], si, run, cores.foil_active, false)
            };
            if new == old { continue; }

            // Per-stack min_place floor on the outgoing card.
            let old_si = old.stack as usize;
            if old.t != T_DEAD && old_si < run.stacks.len()
                && placed_per_stack[old_si] <= run.stacks[old_si].min_place {
                continue;
            }
            // Stat floor.
            let old_stat = counts_toward_floor(&old, cfg);
            let new_stat = counts_toward_floor(&new, cfg);
            let stat_delta = (new_stat as i32) - (old_stat as i32);
            if effective_min_stat > 0 && stat_delta < 0
                && (stat_placed as i32 + stat_delta) < effective_min_stat as i32 {
                continue;
            }
            if !rules.move_ok(&old, &new, &mut tmp_old, &mut tmp_new) { continue; }

            // Apply.
            rules.apply(&old, &new, &mut tmp_old);
            if si != dead_option && remaining[si] != u32::MAX { remaining[si] -= 1; }
            if old.t != T_DEAD && old_si < remaining.len() && remaining[old_si] != u32::MAX {
                remaining[old_si] += 1;
            }
            if old.t != T_DEAD && old_si < placed_per_stack.len() { placed_per_stack[old_si] -= 1; }
            if si != dead_option { placed_per_stack[si] += 1; }
            stat_placed = (stat_placed as i32 + stat_delta) as u32;
            asgn[p] = new;

            let new_score = simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s);
            let delta = new_score - score;
            if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                score = new_score;
                if score > best_score { best_score = score; best_asgn = asgn.clone(); }
            } else {
                rules.apply(&new, &old, &mut tmp_old);
                if si != dead_option {
                    if remaining[si] != u32::MAX { remaining[si] += 1; }
                    placed_per_stack[si] -= 1;
                }
                if old.t != T_DEAD && old_si < remaining.len() {
                    if remaining[old_si] != u32::MAX { remaining[old_si] -= 1; }
                    placed_per_stack[old_si] += 1;
                }
                stat_placed = (stat_placed as i32 - stat_delta) as u32;
                asgn[p] = old;
            }
        } else {
            // ── Pair swap ────────────────────────────────────────────────
            let p1 = rng.gen_range(0..n);
            let mut p2 = rng.gen_range(0..n);
            while p2 == p1 { p2 = rng.gen_range(0..n); }
            if asgn[p1] == asgn[p2] { continue; }
            if locked[p1] || locked[p2] { continue; }
            let a1 = geom.is_arcane_slot[p1];
            let a2 = geom.is_arcane_slot[p2];
            let v1 = asgn[p2].t;
            let v2 = asgn[p1].t;
            let legal = |arc: bool, t: u8| -> bool {
                if arc { t == T_ARCANE || t == T_DEAD } else { t != T_ARCANE }
            };
            if !legal(a1, v1) || !legal(a2, v2) { continue; }

            asgn.swap(p1, p2);
            let new_score = simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s);
            let delta = new_score - score;
            if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                score = new_score;
                if score > best_score { best_score = score; best_asgn = asgn.clone(); }
            } else {
                asgn.swap(p1, p2);
            }
        }
    }

    // §6 final pass — deterministic post-step on the best assignment.
    if run.final_pass_nonfoil_evo && !cfg.is_shiny && cores.foil_active {
        // Rebuild rule counts + remaining for the BEST assignment.
        let mut rules2 = RuleBook::new(&run.tag_rules);
        rules2.seed(&best_asgn);
        let mut remaining2: Vec<u32> = run.stacks.iter().map(|sp| sp.count).collect();
        for c in &best_asgn {
            let si = c.stack as usize;
            if c.t != T_DEAD && si < remaining2.len() && remaining2[si] != u32::MAX {
                remaining2[si] = remaining2[si].saturating_sub(1);
            }
        }
        final_pass_nonfoil_evo(
            geom, run, &mut best_asgn, &mut best_score, cores,
            &mut rules2, &mut remaining2, &mut s,
        );
    }

    (best_asgn, best_score)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry points
// ─────────────────────────────────────────────────────────────────────────────

/// Run the full SA (serial restarts; callers parallelize outside).
pub fn run_sa_tagged(run: TagRun<'_>) -> (Vec<SlotCard>, f64) {
    let geom = build_geom(&run);
    let cores = Cores::build(&run.cores);

    // Foil ban (Targeted foil max = 0) suppresses the run-level foil bit.
    let foil_banned = run.tag_rules.iter().any(|r|
        r.axis == TagAxis::Group && r.key == G_FOIL && r.max == 0);

    // Shiny + WV rules + foil banned ⇒ nothing but DEAD is placeable (§5).
    // Materialize with the ban so the SA sees non-foil cards; the degenerate
    // shiny case is enforced by clearing supply.
    let mut stacks = run.stacks.clone();
    if foil_banned && run.cfg.is_shiny && run.cfg.wv_foil_rules && !run.exact_groups {
        for sp in stacks.iter_mut() { sp.count = 0; sp.min_place = 0; }
    }
    let run = TagRun { stacks, ..run };

    let materialized: Vec<SlotCard> = run.stacks.iter().enumerate()
        .map(|(si, sp)| {
            let mut c = materialize(sp, si, &run, cores.foil_active, foil_banned);
            if foil_banned { c.groups &= !G_FOIL; }
            c
        })
        .collect();

    let restarts = run.restarts.max(1);
    let mut best: Option<(Vec<SlotCard>, f64)> = None;
    for i in 0..restarts {
        let seed = match run.seed {
            Some(s0) => s0 ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
            None => {
                let mut seeder = SmallRng::from_entropy();
                seeder.gen::<u64>() ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            }
        };
        let (asgn, score) = sa_one_restart(&geom, &run, &cores, &materialized, 100.0, 0.5, seed);
        if best.as_ref().map_or(true, |b| score > b.1) {
            best = Some((asgn, score));
        }
    }
    best.expect("at least one restart")
}

/// Score a FIXED assignment (no SA). Used by the parity harness, the TS
/// what-if popup verification, and snapshot restores.
pub fn score_tagged(run: &TagRun<'_>, asgn: &[SlotCard]) -> f64 {
    let geom = build_geom(run);
    let cores = Cores::build(&run.cores);
    let mut s = Scratch::new(&geom);
    simulate(&geom, asgn, &cores, &run.implicits, &run.cfg, &mut s)
}
