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
    /// Category-tag combinations that exist on REAL cards (masks over the 9
    /// category bits, extracted from the game's card data). A card's category
    /// set must be a SUBSET of one of these — no optimizer surface may invent
    /// a tag combination no buildable card has (Wild excepted; it never goes
    /// through tag moves). Empty = unconstrained (back-compat).
    pub legal_combos: Vec<u16>,
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
    /// Present orthogonal neighbors, ascending — boost_fold() iterates these
    /// so per-slot re-folds match boost_pass()'s source order exactly.
    orth_sorted: Vec<Vec<usize>>,
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
    let orth_sorted: Vec<Vec<usize>> = (0..n)
        .map(|i| {
            let mut v: Vec<usize> = orth[i].iter().flatten().copied().collect();
            v.sort_unstable();
            v
        })
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
        orth, orth_sorted, is_arcane_slot,
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

// ─────────────────────────────────────────────────────────────────────────────
// Deck-composition counts — the integer state everything global derives from.
// Rebuilt from scratch by simulate(); maintained incrementally (one card at a
// time) by the SA delta evaluator. Integer state, so both paths agree exactly.
// ─────────────────────────────────────────────────────────────────────────────

struct Counts {
    row_color: Vec<u32>,     // row_span × N_COLORS same-color counts
    col_color: Vec<u32>,     // col_span × N_COLORS
    row_fill: Vec<u32>,      // row_span — color-blind filled counts
    col_fill: Vec<u32>,      // col_span
    n_deluxe: u32,
    n_arcane: u32,
    n_greed: u32,
    n_dead: u32,
    /// Every placed card WITHOUT the Foil group (any type — the game's
    /// NonFoilEfficiencyDeckModifier counts all of them; audited 2026-08-01).
    n_nonfoil: u32,
    n_wild: u32,
    group_refs: [u32; N_GROUP_BITS],   // per-bit placed-card counts → union
    /// Placed cards per color (wild counts as every color, mirroring the
    /// game's Card::getColors) → EQUILIBRIUM's unique-color count.
    color_cards: [u32; N_COLORS],
}

impl Counts {
    fn new(geom: &Geom) -> Self {
        Counts {
            row_color: vec![0; geom.row_span * N_COLORS],
            col_color: vec![0; geom.col_span * N_COLORS],
            row_fill: vec![0; geom.row_span],
            col_fill: vec![0; geom.col_span],
            n_deluxe: 0, n_arcane: 0, n_greed: 0, n_dead: 0,
            n_nonfoil: 0, n_wild: 0,
            group_refs: [0; N_GROUP_BITS],
            color_cards: [0; N_COLORS],
        }
    }

    /// Union-contribution mask of one placed card. Wild counts as every
    /// category + Stat on top of its literal groups; DEAD contributes none.
    #[inline(always)]
    fn union_mask(c: &SlotCard) -> u16 {
        if c.t == T_WILD { c.groups | ALL_CATEGORY_GROUPS | G_STAT } else { c.groups }
    }

    /// Add (`sign`=+1) or remove (`sign`=-1) one card's contribution.
    fn apply(&mut self, geom: &Geom, i: usize, c: &SlotCard, sign: i32) {
        let s = i64::from(sign);
        #[inline(always)]
        fn bump(v: &mut u32, s: i64) { *v = (*v as i64 + s) as u32; }
        if c.t == T_DEAD { bump(&mut self.n_dead, s); return; }
        let r = (geom.row_of[i] - geom.row_min) as usize;
        let cc = (geom.col_of[i] - geom.col_min) as usize;
        bump(&mut self.row_fill[r], s);
        bump(&mut self.col_fill[cc], s);
        if c.t == T_WILD {
            // Wild counts as every color for neighbors' same-color scans —
            // and toward EQUILIBRIUM's unique-color set.
            for k in 0..N_COLORS {
                bump(&mut self.row_color[r * N_COLORS + k], s);
                bump(&mut self.col_color[cc * N_COLORS + k], s);
                bump(&mut self.color_cards[k], s);
            }
            bump(&mut self.n_wild, s);
        } else if c.color != COLOR_NONE {
            bump(&mut self.row_color[r * N_COLORS + c.color as usize], s);
            bump(&mut self.col_color[cc * N_COLORS + c.color as usize], s);
            bump(&mut self.color_cards[c.color as usize], s);
        }
        let mut mask = Self::union_mask(c);
        while mask != 0 {
            bump(&mut self.group_refs[mask.trailing_zeros() as usize], s);
            mask &= mask - 1;
        }
        if c.groups & G_FOIL == 0 { bump(&mut self.n_nonfoil, s); }
        if c.t == T_DELUXE { bump(&mut self.n_deluxe, s); }
        else if c.t == T_ARCANE { bump(&mut self.n_arcane, s); }
        else if is_greed(c.t) { bump(&mut self.n_greed, s); }
    }

    fn rebuild(&mut self, geom: &Geom, asgn: &[SlotCard]) {
        for v in self.row_color.iter_mut() { *v = 0; }
        for v in self.col_color.iter_mut() { *v = 0; }
        for v in self.row_fill.iter_mut() { *v = 0; }
        for v in self.col_fill.iter_mut() { *v = 0; }
        self.n_deluxe = 0; self.n_arcane = 0; self.n_greed = 0;
        self.n_dead = 0; self.n_nonfoil = 0; self.n_wild = 0;
        self.group_refs = [0; N_GROUP_BITS];
        self.color_cards = [0; N_COLORS];
        for (i, c) in asgn.iter().enumerate() {
            self.apply(geom, i, c, 1);
        }
    }

    #[inline(always)]
    fn groups_union(&self) -> u16 {
        let mut u = 0u16;
        for b in 0..N_GROUP_BITS {
            if self.group_refs[b] > 0 { u |= 1u16 << b; }
        }
        u
    }

    /// n_ns (Pure): EVERY placed card without the Foil group. The game's
    /// NonFoilEfficiencyDeckModifier streams all deck cards and filters on
    /// !hasGroup("Foil") — typeless, deluxe and wild count too (audited
    /// 2026-08-01; the old greed+arcane+non-foil-positional definition was
    /// a 1.x simplification).
    #[inline(always)]
    fn n_ns(&self) -> usize {
        self.n_nonfoil as usize
    }

    /// EQUILIBRIUM's unique-color count: distinct colors over all placed
    /// cards (StatEfficiencyDeckModifier.getUniqueColorCount).
    #[inline(always)]
    fn n_distinct_colors(&self) -> usize {
        (0..N_COLORS).filter(|&k| self.color_cards[k] > 0).count()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Derived deck-global scalars — the "cores → baseline + gated addends" block
// and the implicit precompute, shared verbatim by full and delta evaluation.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Derived {
    baseline_sum: f64,
    baseline_prod: f64,
    color_addend: f64,
    color_factor: f64,
    deluxe_addend: f64,
    deluxe_factor: f64,
    deluxe_present: bool,
    void_addend: f64,
    void_factor: f64,
    void_present: bool,
    archive_addend: f64,
    archive_factor: f64,
    freq_mult: [f64; 4],
    chain_value: f64,
    have_chain: bool,
    empty_addend: f64,
    unique_value: f64,
    unique_groups_count: f64,
    mirror_value: f64,
    have_mirror: bool,
}

impl Derived {
    /// Do any count-DEPENDENT scalars differ (bitwise)? The constant-by-
    /// construction fields (color, freq, chain/unique/mirror values) are
    /// excluded; `unique_groups_count` is handled separately (it invalidates
    /// per-slot implicit folds, not just the finish arithmetic).
    fn tail_bits_differ(&self, other: &Derived) -> bool {
        self.baseline_sum.to_bits() != other.baseline_sum.to_bits()
            || self.baseline_prod.to_bits() != other.baseline_prod.to_bits()
            || self.deluxe_addend.to_bits() != other.deluxe_addend.to_bits()
            || self.deluxe_factor.to_bits() != other.deluxe_factor.to_bits()
            || self.void_addend.to_bits() != other.void_addend.to_bits()
            || self.void_factor.to_bits() != other.void_factor.to_bits()
            || self.archive_addend.to_bits() != other.archive_addend.to_bits()
            || self.archive_factor.to_bits() != other.archive_factor.to_bits()
            || self.empty_addend.to_bits() != other.empty_addend.to_bits()
    }
}

fn derive_scalars(
    cores: &Cores,
    implicits: &[Implicit],
    cfg: &TagSimConfig,
    counts: &Counts,
) -> Derived {
    let n_ns = counts.n_ns();
    let n_deluxe = counts.n_deluxe as usize;
    let n_arcane = counts.n_arcane as usize;
    let n_greed = counts.n_greed as usize;
    let n_dead = counts.n_dead as usize;
    let n_colors = counts.n_distinct_colors();
    let any_wild = counts.n_wild > 0;

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
                // StatEfficiencyDeckModifier (audited 2026-08-01): value =
                // 1 + roll × unique deck colors, Stat cards only (≡ shiny
                // runs in this model). `mult_equilibrium` is the PER-COLOR
                // roll (best 0.5 wolds / 0.7 vanilla).
                let scale = if ov { spec.override_ } else { cfg.mult_equilibrium };
                let v = 1.0 + scale * n_colors as f64;
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
    // Archive (live semantics, woldsvaults GroupSynergyMultiplierModifier as
    // of aa5e7b39): per-card modifier value = base^n_arcane, aggregated
    // ADDITIVELY with the other cores by MixinCardDeck (value += mod − 1).
    // No longer an IMultiplicativeDeckModifier — runic alone multiplies the
    // whole card. On the multiplicative (vanilla) path it folds into the
    // per-card product like every other core. No per-card gate.
    let (archive_addend, archive_factor) = if archive_present {
        let f = archive_base.powf(n_arcane as f64);
        (f - 1.0, f)
    } else {
        (0.0, 1.0)
    };

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
        let groups_union = counts.groups_union();
        let mut cnt = groups_union.count_ones() as f64;
        cnt += 1.0;                                  // Shiny or Evolution marker
        if n_deluxe > 0 { cnt += 1.0; }              // Deluxe marker
        if n_arcane > 0 { cnt += 1.0; }              // Arcane marker
        if n_greed > 0 { cnt += 1.0; }               // Greed marker
        if any_wild { cnt += 1.0; }                  // Evolution+Stat combo card
        cnt
    } else { 0.0 };

    Derived {
        baseline_sum, baseline_prod, color_addend, color_factor,
        deluxe_addend, deluxe_factor, deluxe_present,
        void_addend, void_factor, void_present,
        archive_addend, archive_factor,
        freq_mult, chain_value, have_chain, empty_addend,
        unique_value, unique_groups_count,
        mirror_value, have_mirror,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared per-slot evaluation — one code path for full AND delta scoring
// ─────────────────────────────────────────────────────────────────────────────

/// Scan-derived inputs of one slot (peer/grid reads). Cached by the delta
/// evaluator; recomputed via slot_scan() when the slot or a peer changes.
#[derive(Clone, Copy)]
struct SlotInputs {
    contributes: bool,
    base: f64,
    /// Implicit-loop fold EXCLUDING the empty-slots addend — finish_term()
    /// adds `empty_addend` last, mirroring the 1.x accumulation order.
    imp_local: f64,
    color_applies: bool,
    deluxe_applies: bool,
}

const SKIP_INPUTS: SlotInputs = SlotInputs {
    contributes: false, base: 0.0, imp_local: 0.0,
    color_applies: false, deluxe_applies: false,
};

/// Borrowed evaluation context — everything slot_scan()/finish_term() read.
struct EvalCtx<'a> {
    geom: &'a Geom,
    cores: &'a Cores,
    implicits: &'a [Implicit],
    cfg: &'a TagSimConfig,
    counts: &'a Counts,
    der: &'a Derived,
    chain_id: &'a [u32],
    chain_size: &'a [u32],
}

/// Target slot + boost amount of a greed card (§2.3, orthogonal only).
#[inline(always)]
fn greed_target(geom: &Geom, i: usize, t: u8, cfg: &TagSimConfig) -> (Option<usize>, f64) {
    match t {
        T_G_UP => (geom.orth[i][0], cfg.mult_dir_vert),
        T_G_DOWN => (geom.orth[i][1], cfg.mult_dir_vert),
        T_G_LEFT => (geom.orth[i][2], cfg.mult_dir_horiz),
        T_G_RIGHT => (geom.orth[i][3], cfg.mult_dir_horiz),
        _ => (None, 0.0),
    }
}

/// Full greed boost pass (§2.3): iterate greed SOURCES in ascending slot
/// order, accumulating into each target. boost_fold() must visit sources in
/// this same ascending order to stay bit-identical.
fn boost_pass(geom: &Geom, asgn: &[SlotCard], cfg: &TagSimConfig, boost: &mut [f64]) {
    let n = geom.n;
    for v in boost[..n].iter_mut() { *v = 1.0; }
    for i in 0..n {
        let g = asgn[i];
        if !is_greed(g.t) { continue; }
        let (target, amount) = greed_target(geom, i, g.t, cfg);
        if let Some(j) = target {
            if is_scorable(asgn[j].t) {
                // Complex Cards: greed boosts only its scale_color (§7).
                // Non-Complex: color-agnostic (current behavior, gate-safe).
                let color_ok = !(cfg.colors_real && cfg.complex)
                    || asgn[j].t == T_WILD
                    || asgn[j].color == g.scale;
                if color_ok {
                    if cfg.greed_additive { boost[j] += amount; }
                    else { boost[j] *= amount; }
                }
            }
        }
    }
}

/// Recompute ONE slot's boost by folding its orthogonal greed sources in
/// ascending slot order — the same contribution order boost_pass() produces,
/// so the result is bit-identical to a full pass.
fn boost_fold(geom: &Geom, asgn: &[SlotCard], cfg: &TagSimConfig, j: usize) -> f64 {
    let mut b = 1.0f64;
    if !is_scorable(asgn[j].t) { return b; }
    for &si in &geom.orth_sorted[j] {
        let g = asgn[si];
        if !is_greed(g.t) { continue; }
        let (target, amount) = greed_target(geom, si, g.t, cfg);
        if target != Some(j) { continue; }
        let color_ok = !(cfg.colors_real && cfg.complex)
            || asgn[j].t == T_WILD
            || asgn[j].color == g.scale;
        if color_ok {
            if cfg.greed_additive { b += amount; } else { b *= amount; }
        }
    }
    b
}

/// Snake chain labeling — one flood-fill pass over non-dead cards.
/// colors_real: components are same-color (wild bridges any color).
/// blanket (Max): components are filled-connectivity (mono assumption).
fn chain_flood_fill(
    geom: &Geom,
    asgn: &[SlotCard],
    cfg: &TagSimConfig,
    chain_id: &mut [u32],
    chain_size: &mut [u32],
    chain_stack: &mut Vec<usize>,
) {
    let n = geom.n;
    for v in chain_id.iter_mut() { *v = 0; }
    let mut next_id = 0u32;
    for start in 0..n {
        if asgn[start].t == T_DEAD || chain_id[start] != 0 { continue; }
        next_id += 1;
        let mut size = 0u32;
        chain_stack.clear();
        chain_stack.push(start);
        chain_id[start] = next_id;
        while let Some(i) = chain_stack.pop() {
            size += 1;
            for d in 0..4 {
                if let Some(j) = geom.orth[i][d] {
                    if asgn[j].t == T_DEAD || chain_id[j] != 0 { continue; }
                    let same = if !cfg.colors_real {
                        true
                    } else {
                        let a = asgn[i]; let b = asgn[j];
                        a.t == T_WILD || b.t == T_WILD
                            || (a.color != COLOR_NONE && a.color == b.color)
                    };
                    if same {
                        chain_id[j] = next_id;
                        chain_stack.push(j);
                    }
                }
            }
        }
        chain_size[next_id as usize] = size;
    }
}

/// Compute one slot's scan-derived inputs (base value, local implicit fold,
/// core gates). Verbatim 1.x accumulate-loop math, reading grids from Counts.
fn slot_scan(ctx: &EvalCtx<'_>, asgn: &[SlotCard], i: usize) -> SlotInputs {
    let geom = ctx.geom;
    let cfg = ctx.cfg;
    let cores = ctx.cores;
    let der = ctx.der;
    let c = asgn[i];
    if !is_scorable(c.t) { return SKIP_INPUTS; }
    // Non-stat card (Resource/Temporal): 0 NDM itself. It was already
    // counted as a filled neighbor / n_ns / implicit-feeder above.
    if c.groups & NONSTAT_GROUPS != 0 { return SKIP_INPUTS; }

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
                    ctx.counts.row_color[r * N_COLORS + scan_color as usize] as f64
                } else {
                    ctx.counts.row_fill[r] as f64
                }
            }
            T_COL => {
                let cc = (geom.col_of[i] - geom.col_min) as usize;
                if cfg.colors_real {
                    ctx.counts.col_color[cc * N_COLORS + scan_color as usize] as f64
                } else {
                    ctx.counts.col_fill[cc] as f64
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
        let scaled = raw * der.freq_mult[c.t as usize];
        if c.t == T_DIAG { scaled.max(1.0) } else { scaled }
    } else if c.t == T_DELUXE {
        cfg.mult_deluxe_flat
    } else {
        1.0   // TYPELESS
    };

    // Core gates. Color-blind runs apply the COLOR core flat (classic
    // behavior — the parity gate depends on it); color-real runs gate on
    // the card's own color.
    let color_applies = cores.has_color_core
        && (!cfg.colors_real
            || (cores.color_core_color != COLOR_NONE
                && (c.t == T_WILD || c.color == cores.color_core_color)));
    let deluxe_applies = der.deluxe_present && c.t != T_DELUXE;

    // Additive implicit addends for THIS card (empty-slots excluded — see
    // SlotInputs::imp_local).
    let mut imp_addend = 0.0f64;
    for imp in ctx.implicits {
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
                let id = ctx.chain_id[i] as usize;
                if id != 0 {
                    imp_addend += der.chain_value * (ctx.chain_size[id] - 1) as f64;
                }
            }
            Implicit::UniqueGroups { .. } => {
                if c.groups & G_STAT != 0 {
                    imp_addend += der.unique_value * der.unique_groups_count;
                }
            }
            _ => {}
        }
    }

    SlotInputs {
        contributes: true, base, imp_local: imp_addend,
        color_applies, deluxe_applies,
    }
}

/// Combine cached/derived pieces into the slot's final NDM term — the tail
/// arithmetic of the 1.x accumulate loop, expression-for-expression.
fn finish_term(
    ctx: &EvalCtx<'_>,
    asgn: &[SlotCard],
    i: usize,
    si: &SlotInputs,
    boost_i: f64,
) -> f64 {
    let cfg = ctx.cfg;
    let der = ctx.der;
    let c = asgn[i];

    let imp_addend = si.imp_local + der.empty_addend;   // shadow applies to every scoring card

    let core_mult = if cfg.additive_cores {
        1.0 + der.baseline_sum
            + if si.color_applies { der.color_addend } else { 0.0 }
            + if si.deluxe_applies { der.deluxe_addend } else { 0.0 }
            + if der.void_present { der.void_addend } else { 0.0 }
            + der.archive_addend
            + imp_addend
    } else {
        // Vanilla path — implicits never active there; keep the pure
        // multiplicative composition identical to 1.x.
        let mut m = der.baseline_prod;
        if si.color_applies { m *= der.color_factor; }
        if si.deluxe_applies { m *= der.deluxe_factor; }
        if der.void_present { m *= der.void_factor; }
        m * der.archive_factor
    };

    // Runic (multiplicative mirror).
    let mirror_factor = if der.have_mirror {
        let pass = if ctx.geom.mirror_self[i] {
            true
        } else if let Some(mi) = ctx.geom.mirror_of[i] {
            let mc = asgn[mi];
            if mc.t == T_DEAD { false }
            else if !cfg.colors_real { true }
            else { mc.t == T_WILD || (mc.color != COLOR_NONE && mc.color == c.color) }
        } else { false };
        if pass { der.mirror_value } else { 1.0 }
    } else { 1.0 };

    let b = if cfg.greed_additive { boost_i.max(1.0) } else { boost_i };
    si.base * core_mult * b * mirror_factor
}

// Scratch buffers reused across simulate() calls (zero-alloc hot path).
struct Scratch {
    counts: Counts,
    boost: Vec<f64>,
    chain_id: Vec<u32>,      // snake component labels (0 = unlabeled)
    chain_size: Vec<u32>,    // per component id (1-indexed)
    chain_stack: Vec<usize>,
}

impl Scratch {
    fn new(geom: &Geom) -> Self {
        Scratch {
            counts: Counts::new(geom),
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

fn simulate(
    geom: &Geom,
    asgn: &[SlotCard],
    cores: &Cores,
    implicits: &[Implicit],
    cfg: &TagSimConfig,
    s: &mut Scratch,
) -> f64 {
    let n = geom.n;
    s.counts.rebuild(geom, asgn);
    let der = derive_scalars(cores, implicits, cfg, &s.counts);
    if der.have_chain {
        chain_flood_fill(
            geom, asgn, cfg,
            &mut s.chain_id, &mut s.chain_size, &mut s.chain_stack,
        );
    }
    boost_pass(geom, asgn, cfg, &mut s.boost);

    let ctx = EvalCtx {
        geom, cores, implicits, cfg,
        counts: &s.counts, der: &der,
        chain_id: &s.chain_id, chain_size: &s.chain_size,
    };
    let mut ndm = 0.0f64;
    for i in 0..n {
        let si = slot_scan(&ctx, asgn, i);
        if !si.contributes { continue; }
        ndm += finish_term(&ctx, asgn, i, &si, s.boost[i]);
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
// Delta evaluation — incremental SA re-scoring
// ─────────────────────────────────────────────────────────────────────────────
//
// The SA proposes single-slot replacements, pair swaps, and group-bit
// toggles. Re-running simulate() for each proposal costs O(n · scans);
// DeltaEval instead maintains Counts incrementally plus per-slot caches of
// SlotInputs / boost / final terms, recomputing only the slots whose inputs
// can have changed, then re-summing every contributing term in slot order.
//
// BIT-EXACT CONTRACT: after every propose() (and after rollback()) the
// tracked score is bit-for-bit what simulate() returns on the same
// assignment, so seeded SA trajectories are unchanged. This holds because
//   (a) Counts is integer state (incremental == rebuilt),
//   (b) recomputed slots run the same slot_scan()/finish_term() code the
//       full evaluator runs,
//   (c) boost_fold() visits greed sources in boost_pass()'s ascending order,
//   (d) the re-sum walks terms in slot order with the same skip structure,
//   (e) when a count-derived global scalar drifts (pure/deluxe/void/archive/
//       empty), every slot's tail arithmetic is recomputed from cached
//       inputs — same expressions, no rescans — and
//   (f) when unique_groups_count drifts (mutant), every slot is rescanned.
// Peer lists are assumed symmetric (q ∈ peers[p] ⇔ p ∈ peers[q]) — true for
// the game's row/col/surr/diag definitions every caller supplies.
//
// Decks carrying a CHAIN implicit score through component topology (globally
// non-local): sa_one_restart() keeps the full simulate() path for those.
// The delta_full_equiv stress test asserts the contract across the config
// matrix at every step, including after rollbacks.

#[derive(Clone, Copy)]
struct SlotSave {
    i: usize,
    inputs: SlotInputs,
    term: f64,
    boost: f64,
}

// Journal mode of the pending proposal (what rollback must undo).
const J_LOCAL: u8 = 0;
const J_TAIL: u8 = 1;
const J_FULL: u8 = 2;

/// Below this slot count the per-move bookkeeping costs more than the full
/// re-simulate it saves — sa_one_restart() keeps the plain path.
const DELTA_MIN_SLOTS: usize = 16;

/// The Derived-input contributions of a card: [dead, wild, deluxe, arcane,
/// greed, non-foil]. derive_scalars() reads a counter only when a matching
/// core/implicit is active, so a proposal skips the call whenever every
/// SENSITIVE net delta is zero — the output would be bit-identical. (Net
/// matters: a non-foil card swapped for another non-foil card keeps n_ns,
/// so a pure-core deck stays on the local path for that move.)
#[inline(always)]
fn derive_class(c: &SlotCard) -> [i32; 6] {
    [
        i32::from(c.t == T_DEAD),
        i32::from(c.t == T_WILD),
        i32::from(c.t == T_DELUXE),
        i32::from(c.t == T_ARCANE),
        i32::from(is_greed(c.t)),
        i32::from(c.t != T_DEAD && c.groups & G_FOIL == 0),
    ]
}

struct DeltaEval {
    counts: Counts,
    der: Derived,
    inputs: Vec<SlotInputs>,
    term: Vec<f64>,
    boost: Vec<f64>,
    score: f64,
    adj_col: bool,
    adj_surr: bool,
    colormismatch: bool,
    have_unique: bool,
    // Which counters derive_scalars() actually reads for this run.
    sens_ns: bool,
    sens_deluxe: bool,
    sens_dead: bool,
    sens_arcane: bool,
    sens_colors: bool,
    // Zero-filled stand-ins lent to EvalCtx (chain decks never use delta).
    chain_zero_id: Vec<u32>,
    chain_zero_size: Vec<u32>,
    // Dirty-set scratch (stamp-deduped) + one-proposal undo journal.
    stamp: u32,
    stamped: Vec<u32>,
    dirty: Vec<usize>,
    refresh: Vec<usize>,
    j_slots: Vec<SlotSave>,
    j_cards: Vec<(usize, SlotCard, SlotCard)>,
    j_der: Derived,
    j_score: f64,
    j_mode: u8,
}

impl DeltaEval {
    fn new(
        geom: &Geom,
        asgn: &[SlotCard],
        cores: &Cores,
        implicits: &[Implicit],
        cfg: &TagSimConfig,
    ) -> Self {
        debug_assert!(
            !implicits.iter().any(|imp| matches!(imp, Implicit::Chain { .. })),
            "DeltaEval does not support CHAIN implicits — guard at the call site",
        );
        let n = geom.n;
        let mut counts = Counts::new(geom);
        counts.rebuild(geom, asgn);
        let der = derive_scalars(cores, implicits, cfg, &counts);
        let mut boost = vec![1.0; n];
        boost_pass(geom, asgn, cfg, &mut boost);
        let chain_zero_id = vec![0u32; n];
        let chain_zero_size = vec![0u32; n + 1];
        let mut inputs = vec![SKIP_INPUTS; n];
        let mut term = vec![0.0f64; n];
        {
            let ctx = EvalCtx {
                geom, cores, implicits, cfg,
                counts: &counts, der: &der,
                chain_id: &chain_zero_id, chain_size: &chain_zero_size,
            };
            for i in 0..n {
                let si = slot_scan(&ctx, asgn, i);
                inputs[i] = si;
                term[i] = if si.contributes {
                    finish_term(&ctx, asgn, i, &si, boost[i])
                } else { 0.0 };
            }
        }
        let mut d = DeltaEval {
            counts, der, inputs, term, boost,
            score: 0.0,
            adj_col: implicits.iter().any(|imp|
                matches!(imp, Implicit::Adjacency { surrounding: false, .. })),
            adj_surr: implicits.iter().any(|imp|
                matches!(imp, Implicit::Adjacency { surrounding: true, .. })),
            colormismatch: implicits.iter().any(|imp|
                matches!(imp, Implicit::ColorMismatch { .. })),
            have_unique: implicits.iter().any(|imp|
                matches!(imp, Implicit::UniqueGroups { .. })),
            sens_ns: cores.list.iter().any(|s| s.core_type == CORE_PURE),
            sens_deluxe: cores.list.iter().any(|s| s.core_type == CORE_DELUXE),
            sens_dead: cores.list.iter().any(|s| s.core_type == CORE_VOID)
                || implicits.iter().any(|imp|
                    matches!(imp, Implicit::EmptySlots { .. })),
            sens_arcane: cores.list.iter().any(|s| s.core_type == CORE_ARCHIVE),
            sens_colors: cfg.is_shiny
                && cores.list.iter().any(|s| s.core_type == CORE_EQUILIBRIUM),
            chain_zero_id, chain_zero_size,
            stamp: 0,
            stamped: vec![0u32; n],
            dirty: Vec::with_capacity(n),
            refresh: Vec::with_capacity(16),
            j_slots: Vec::with_capacity(n),
            j_cards: Vec::with_capacity(2),
            j_der: der,
            j_score: 0.0,
            j_mode: J_LOCAL,
        };
        d.score = d.resum();
        d
    }

    /// Sum cached terms in slot order with simulate()'s skip structure —
    /// the same adds in the same order, so the same bits.
    #[inline(always)]
    fn resum(&self) -> f64 {
        let mut ndm = 0.0f64;
        for (i, si) in self.inputs.iter().enumerate() {
            if si.contributes { ndm += self.term[i]; }
        }
        ndm
    }

    #[inline(always)]
    fn mark(&mut self, i: usize) {
        if self.stamped[i] != self.stamp {
            self.stamped[i] = self.stamp;
            self.dirty.push(i);
        }
    }

    /// Score a proposal. `changed` holds (slot, PRE-move card) pairs with
    /// `asgn` already mutated to the new cards. Call commit() on accept or
    /// rollback() on reject before the next propose().
    fn propose(
        &mut self,
        geom: &Geom,
        cores: &Cores,
        implicits: &[Implicit],
        cfg: &TagSimConfig,
        asgn: &[SlotCard],
        changed: &[(usize, SlotCard)],
    ) -> f64 {
        // Journal pre-move globals, then apply the integer count deltas.
        self.j_cards.clear();
        self.j_slots.clear();
        self.j_der = self.der;
        self.j_score = self.score;
        for &(p, old) in changed {
            let new = asgn[p];
            self.counts.apply(geom, p, &old, -1);
            self.counts.apply(geom, p, &new, 1);
            self.j_cards.push((p, old, new));
        }
        // Derive-input skip: when every count derive_scalars() actually
        // reads has a zero NET delta (and, under mutant, no union mask
        // moved), its output is bit-identical — skip it and both global
        // passes.
        let mut dc = [0i32; 6];
        let mut union_moved = false;
        let mut color_moved = false;
        for k in 0..self.j_cards.len() {
            let (_, old, new) = self.j_cards[k];
            let co = derive_class(&old);
            let cn = derive_class(&new);
            for b in 0..6 { dc[b] += cn[b] - co[b]; }
            union_moved |= Counts::union_mask(&old) != Counts::union_mask(&new);
            color_moved |= old.color != new.color
                || (old.t == T_WILD) != (new.t == T_WILD)
                || (old.t == T_DEAD) != (new.t == T_DEAD);
        }
        let any_derive = (self.sens_ns && dc[5] != 0)
            || (self.sens_deluxe && dc[2] != 0)
            || (self.sens_dead && dc[0] != 0)
            || (self.sens_arcane && dc[3] != 0)
            || (self.sens_colors && color_moved)
            || (self.have_unique
                && (union_moved || dc.iter().any(|&d| d != 0)));
        let (g_full, g_tail) = if any_derive {
            let new_der = derive_scalars(cores, implicits, cfg, &self.counts);
            // unique_groups_count feeds the per-slot implicit FOLD → rescan.
            let g_full = new_der.unique_groups_count.to_bits()
                != self.j_der.unique_groups_count.to_bits();
            // Every other count-derived scalar only feeds finish_term() →
            // cheap tail recompute over cached inputs, no rescans.
            let g_tail = !g_full && new_der.tail_bits_differ(&self.j_der);
            self.der = new_der;
            (g_full, g_tail)
        } else { (false, false) };

        let n = geom.n;
        self.stamp += 1;
        self.dirty.clear();

        // Local dirty set — every slot whose scans read a changed card,
        // gated by what actually changed about it. Other slots are blind to
        // a card's positional TYPE: bases count deadness/wildness/color
        // (deadness only when color-blind), adjacency reads dead/greed/wild
        // class + groups, boosts read greed-ness, runic reads the mirror
        // partner like a base scan.
        for k in 0..self.j_cards.len() {
            let (p, old, new) = self.j_cards[k];
            self.mark(p);
            let scan_ch = if cfg.colors_real {
                (old.t == T_DEAD) != (new.t == T_DEAD)
                    || (old.t == T_WILD) != (new.t == T_WILD)
                    || old.color != new.color
            } else {
                (old.t == T_DEAD) != (new.t == T_DEAD)
            };
            let greed_ch = is_greed(old.t) || is_greed(new.t);
            let adj_ch = (self.adj_col || self.adj_surr)
                && ((old.t == T_DEAD) != (new.t == T_DEAD)
                    || (old.t == T_WILD) != (new.t == T_WILD)
                    || is_greed(old.t) != is_greed(new.t)
                    || old.groups != new.groups);
            if greed_ch || (self.colormismatch && scan_ch) {
                for d in 0..4 {
                    if let Some(q) = geom.orth[p][d] { self.mark(q); }
                }
            }
            if scan_ch {
                if let Some(m) = geom.mirror_of[p] { self.mark(m); }
                for k2 in 0..geom.row_peers[p].len() {
                    let q = geom.row_peers[p][k2];
                    if asgn[q].t == T_ROW { self.mark(q); }
                }
                for k2 in 0..geom.diag_peers[p].len() {
                    let q = geom.diag_peers[p][k2];
                    if asgn[q].t == T_DIAG { self.mark(q); }
                }
            }
            if scan_ch || (self.adj_col && adj_ch) {
                let adj = self.adj_col && adj_ch;
                for k2 in 0..geom.col_peers[p].len() {
                    let q = geom.col_peers[p][k2];
                    if adj || (scan_ch && asgn[q].t == T_COL) { self.mark(q); }
                }
            }
            if scan_ch || (self.adj_surr && adj_ch) {
                let adj = self.adj_surr && adj_ch;
                for k2 in 0..geom.surr_peers[p].len() {
                    let q = geom.surr_peers[p][k2];
                    if adj || (scan_ch && asgn[q].t == T_SURR) { self.mark(q); }
                }
            }
        }

        // Boost refresh set: changed slots always (their own boost gate reads
        // their card), plus orth targets when a greed was involved. Both are
        // marked dirty above under the same conditions, so refresh ⊆ dirty
        // and the journal below covers every refreshed slot.
        self.refresh.clear();
        for k in 0..self.j_cards.len() {
            let (p, old, new) = self.j_cards[k];
            if !self.refresh.contains(&p) { self.refresh.push(p); }
            if is_greed(old.t) || is_greed(new.t) {
                for d in 0..4 {
                    if let Some(q) = geom.orth[p][d] {
                        if !self.refresh.contains(&q) { self.refresh.push(q); }
                    }
                }
            }
        }

        // Journal the dirty slots only. Slots touched solely by a global
        // pass (tail/full) are NOT journaled — rollback() recomputes them
        // from restored state, which is the same pure function.
        self.j_mode = if g_full { J_FULL } else if g_tail { J_TAIL } else { J_LOCAL };
        for k in 0..self.dirty.len() {
            let i = self.dirty[k];
            self.j_slots.push(SlotSave {
                i, inputs: self.inputs[i],
                term: self.term[i], boost: self.boost[i],
            });
        }

        for k in 0..self.refresh.len() {
            let j = self.refresh[k];
            self.boost[j] = boost_fold(geom, asgn, cfg, j);
        }

        {
            let ctx = EvalCtx {
                geom, cores, implicits, cfg,
                counts: &self.counts, der: &self.der,
                chain_id: &self.chain_zero_id, chain_size: &self.chain_zero_size,
            };
            if g_full {
                for i in 0..n {
                    let si = slot_scan(&ctx, asgn, i);
                    self.inputs[i] = si;
                    self.term[i] = if si.contributes {
                        finish_term(&ctx, asgn, i, &si, self.boost[i])
                    } else { 0.0 };
                }
            } else {
                for k in 0..self.dirty.len() {
                    let i = self.dirty[k];
                    let si = slot_scan(&ctx, asgn, i);
                    self.inputs[i] = si;
                    self.term[i] = if si.contributes {
                        finish_term(&ctx, asgn, i, &si, self.boost[i])
                    } else { 0.0 };
                }
                if g_tail {
                    for i in 0..n {
                        if self.stamped[i] != self.stamp {
                            let si = self.inputs[i];
                            self.term[i] = if si.contributes {
                                finish_term(&ctx, asgn, i, &si, self.boost[i])
                            } else { 0.0 };
                        }
                    }
                }
            }
        }

        self.score = self.resum();
        self.score
    }

    /// Accept the pending proposal (journal discarded).
    fn commit(&mut self) {
        self.j_cards.clear();
        self.j_slots.clear();
    }

    /// Revert the pending proposal exactly: integer counts reverse-applied,
    /// journaled per-slot values and globals restored bitwise. Slots a
    /// global (tail/full) pass touched are recomputed from the restored
    /// state instead — the same pure function, so the same bits. The caller
    /// must restore `asgn` BEFORE calling this (the SA reject arms do).
    fn rollback(
        &mut self,
        geom: &Geom,
        cores: &Cores,
        implicits: &[Implicit],
        cfg: &TagSimConfig,
        asgn: &[SlotCard],
    ) {
        for k in 0..self.j_cards.len() {
            let (p, old, new) = self.j_cards[k];
            self.counts.apply(geom, p, &new, -1);
            self.counts.apply(geom, p, &old, 1);
        }
        self.der = self.j_der;
        for k in 0..self.j_slots.len() {
            let sv = self.j_slots[k];
            self.inputs[sv.i] = sv.inputs;
            self.term[sv.i] = sv.term;
            self.boost[sv.i] = sv.boost;
        }
        if self.j_mode != J_LOCAL {
            // stamped[] still holds this proposal's marks: un-journaled
            // (non-dirty) slots are exactly the un-stamped ones.
            let ctx = EvalCtx {
                geom, cores, implicits, cfg,
                counts: &self.counts, der: &self.der,
                chain_id: &self.chain_zero_id, chain_size: &self.chain_zero_size,
            };
            let full = self.j_mode == J_FULL;
            for i in 0..geom.n {
                if self.stamped[i] == self.stamp { continue; }
                if full {
                    let si = slot_scan(&ctx, asgn, i);
                    self.inputs[i] = si;
                    self.term[i] = if si.contributes {
                        finish_term(&ctx, asgn, i, &si, self.boost[i])
                    } else { 0.0 };
                } else {
                    let si = self.inputs[i];
                    self.term[i] = if si.contributes {
                        finish_term(&ctx, asgn, i, &si, self.boost[i])
                    } else { 0.0 };
                }
            }
        }
        self.score = self.j_score;
        self.j_cards.clear();
        self.j_slots.clear();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Card materialization — build the SlotCard a stack places on a slot
// ─────────────────────────────────────────────────────────────────────────────

/// Run-level foil rule (§5): which placed cards carry the Foil bit.
/// Applied on top of stack groups unless `exact_groups` (Exact mode builds
/// foil per card, with shiny⇒foil enforced in the UI builder).
/// ARCANE never takes foil (playtest ruling): a foil arcane would only ever
/// cost you (it can't gain from foil-gated implicits — it scores 0 — and
/// foil-ness could only threaten its Pure contribution). Our n_ns counts
/// arcane unconditionally, which now always matches the never-foil reality.
fn run_foil_groups(t: u8, cfg: &TagSimConfig, foil_core_active: bool, foil_banned: bool) -> u16 {
    if foil_banned { return 0; }
    let foil = if cfg.is_shiny { cfg.wv_foil_rules } else { foil_core_active };
    if foil && is_scorable(t) { G_FOIL } else { 0 }
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
        if is_scorable(spec.t) {
            groups |= run.blanket_groups & !run.assignable_groups;
        }
        groups |= run_foil_groups(spec.t, &run.cfg, foil_core_active, foil_banned);
        if is_greed(spec.t) { groups = 0; }   // greed cards carry no groups
    }
    // Arcane cards carry NO tags in any mode (playtest ruling) — no foil,
    // no categories, regardless of what a stack claims.
    if spec.t == T_ARCANE { groups = 0; }
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

    // Incremental re-scoring (bit-exact — see the DeltaEval contract).
    // CHAIN implicits score through global component topology, and tiny
    // decks pay more in bookkeeping than a full pass costs; both keep the
    // plain simulate() path.
    let use_delta = n >= DELTA_MIN_SLOTS
        && !run.implicits.iter()
            .any(|imp| matches!(imp, Implicit::Chain { .. }));
    let mut deval = if use_delta {
        let d = DeltaEval::new(geom, &asgn, cores, &run.implicits, cfg);
        debug_assert_eq!(d.score.to_bits(), score.to_bits());
        Some(d)
    } else { None };

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
            // Adding a category bit must keep the card's category set a
            // subset of some REAL card's set (removal can't break subset
            // legality). Wild never reaches here (not scorable).
            if adding && !run.legal_combos.is_empty() {
                let next_cats = (c.groups | bit) & ALL_CATEGORY_GROUPS;
                if !run.legal_combos.iter().any(|&m| next_cats & !m == 0) {
                    continue;
                }
            }
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
            let new_score = match deval.as_mut() {
                Some(d) => d.propose(geom, cores, &run.implicits, cfg, &asgn, &[(p, c)]),
                None => simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s),
            };
            let delta = new_score - score;
            if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                rules.toggle_apply(bit, adding);
                stat_placed = (stat_placed as i32 + stat_delta) as u32;
                score = new_score;
                if score > best_score { best_score = score; best_asgn = asgn.clone(); }
                if let Some(d) = deval.as_mut() { d.commit(); }
            } else {
                asgn[p].groups ^= bit;
                if let Some(d) = deval.as_mut() {
                    d.rollback(geom, cores, &run.implicits, cfg, &asgn);
                }
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
                let new_score = match deval.as_mut() {
                    Some(d) => d.propose(geom, cores, &run.implicits, cfg, &asgn, &[(p, old)]),
                    None => simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s),
                };
                let delta = new_score - score;
                if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                    score = new_score;
                    if score > best_score { best_score = score; best_asgn = asgn.clone(); }
                    if let Some(d) = deval.as_mut() { d.commit(); }
                } else {
                    rules.apply(&new, &old, &mut tmp_old);
                    if remaining[si] != u32::MAX { remaining[si] += 1; }
                    if old_si < remaining.len() && remaining[old_si] != u32::MAX { remaining[old_si] -= 1; }
                    asgn[p] = old;
                    if let Some(d) = deval.as_mut() {
                        d.rollback(geom, cores, &run.implicits, cfg, &asgn);
                    }
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

            let new_score = match deval.as_mut() {
                Some(d) => d.propose(geom, cores, &run.implicits, cfg, &asgn, &[(p, old)]),
                None => simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s),
            };
            let delta = new_score - score;
            if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                score = new_score;
                if score > best_score { best_score = score; best_asgn = asgn.clone(); }
                if let Some(d) = deval.as_mut() { d.commit(); }
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
                if let Some(d) = deval.as_mut() {
                    d.rollback(geom, cores, &run.implicits, cfg, &asgn);
                }
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

            let o1 = asgn[p1];
            let o2 = asgn[p2];
            asgn.swap(p1, p2);
            let new_score = match deval.as_mut() {
                Some(d) => d.propose(
                    geom, cores, &run.implicits, cfg, &asgn, &[(p1, o1), (p2, o2)],
                ),
                None => simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s),
            };
            let delta = new_score - score;
            if delta >= 0.0 || rng.gen::<f64>() < (delta / temperature).exp() {
                score = new_score;
                if score > best_score { best_score = score; best_asgn = asgn.clone(); }
                if let Some(d) = deval.as_mut() { d.commit(); }
            } else {
                asgn.swap(p1, p2);
                if let Some(d) = deval.as_mut() {
                    d.rollback(geom, cores, &run.implicits, cfg, &asgn);
                }
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

// ─────────────────────────────────────────────────────────────────────────────
// Tests — DeltaEval ≡ simulate() bit-exactness stress
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod delta_tests {
    use super::*;

    /// Row/col/surr/diag peer lists (self-exclusive) exactly as the callers
    /// build them from the game's definitions — symmetric by construction.
    fn build_peers(slots: &[(i32, i32)])
        -> (Vec<Vec<usize>>, Vec<Vec<usize>>, Vec<Vec<usize>>, Vec<Vec<usize>>)
    {
        let n = slots.len();
        let mut row = vec![Vec::new(); n];
        let mut col = vec![Vec::new(); n];
        let mut surr = vec![Vec::new(); n];
        let mut diag = vec![Vec::new(); n];
        for i in 0..n {
            for j in 0..n {
                if i == j { continue; }
                let (r1, c1) = slots[i];
                let (r2, c2) = slots[j];
                if r1 == r2 { row[i].push(j); }
                if c1 == c2 { col[i].push(j); }
                if (r1 - r2).abs() <= 1 && (c1 - c2).abs() <= 1 { surr[i].push(j); }
                if (r1 - r2).abs() == (c1 - c2).abs() { diag[i].push(j); }
            }
        }
        (row, col, surr, diag)
    }

    const CARD_TYPES: [u8; 13] = [
        T_ROW, T_COL, T_SURR, T_DIAG, T_DELUXE, T_TYPELESS,
        T_G_UP, T_G_DOWN, T_G_LEFT, T_G_RIGHT, T_ARCANE, T_WILD, T_DEAD,
    ];

    fn rand_card(rng: &mut SmallRng) -> SlotCard {
        let t = CARD_TYPES[rng.gen_range(0..CARD_TYPES.len())];
        if t == T_DEAD { return DEAD_CARD; }
        let color = rng.gen_range(0..N_COLORS) as u8;
        let scale = if rng.gen_bool(0.25) {
            rng.gen_range(0..N_COLORS) as u8
        } else { color };
        let mut groups = 0u16;
        for b in 0..N_GROUP_BITS {
            if rng.gen_bool(0.15) { groups |= 1u16 << b; }
        }
        SlotCard { t, color, scale, groups, stack: 0 }
    }

    fn rand_cfg(rng: &mut SmallRng) -> TagSimConfig {
        let colors_real = rng.gen_bool(0.5);
        TagSimConfig {
            mult_dir_vert: 1.0 + rng.gen_range(1..=5) as f64,
            mult_dir_horiz: 1.0 + rng.gen_range(1..=5) as f64,
            mult_pure_base: 1.0,
            mult_pure_scale: 0.03 * rng.gen_range(1..=3) as f64,
            mult_equilibrium: 1.25,
            mult_foil: 2.0,
            mult_steadfast: 1.2,
            mult_sparkling: 1.6,
            mult_color: 1.5,
            mult_deluxe_flat: 3.0,
            mult_deluxe_core_base: 1.0,
            mult_deluxe_core_scale: 0.1,
            mult_void_core_base: 1.0,
            mult_void_core_scale: 0.15,
            mult_archive_core: 1.1,
            greed_additive: rng.gen_bool(0.7),
            additive_cores: rng.gen_bool(0.7),
            is_shiny: rng.gen_bool(0.5),
            auto_place_arcane: false,
            colors_real,
            complex: colors_real && rng.gen_bool(0.4),
            wv_foil_rules: true,
            floor_counts_deluxe: false,
        }
    }

    fn rand_cores(rng: &mut SmallRng) -> Vec<CoreSpecIn> {
        const TYPES: [u8; 9] = [
            CORE_PURE, CORE_EQUILIBRIUM, CORE_STEADFAST, CORE_COLOR,
            CORE_FOIL, CORE_DELUXE, CORE_VOID, CORE_ARCHIVE, CORE_SPARKLING,
        ];
        let mut out = Vec::new();
        for &t in &TYPES {
            if rng.gen_bool(0.35) {
                out.push(CoreSpecIn {
                    core_type: t,
                    color: if t == CORE_COLOR {
                        rng.gen_range(0..N_COLORS) as u8
                    } else { COLOR_NONE },
                    override_: if rng.gen_bool(0.2) {
                        1.0 + rng.gen_range(1..=8) as f64 * 0.1
                    } else { -1.0 },
                });
            }
        }
        out
    }

    /// Every implicit kind EXCEPT Chain (chain decks bypass DeltaEval).
    fn rand_implicits(rng: &mut SmallRng) -> Vec<Implicit> {
        let mut out = Vec::new();
        if rng.gen_bool(0.4) {
            out.push(Implicit::GlobalFlat {
                value: 0.2,
                groups: if rng.gen_bool(0.5) { G_STAT } else { 0 },
                colors: if rng.gen_bool(0.4) { 1 << rng.gen_range(0..N_COLORS) } else { 0 },
            });
        }
        if rng.gen_bool(0.35) {
            out.push(Implicit::Freq { mult: 2.0, ptype: rng.gen_range(0..4) as u8 });
        }
        if rng.gen_bool(0.35) {
            out.push(Implicit::Adjacency {
                value: 0.15,
                group: 1u16 << rng.gen_range(0..N_GROUP_BITS),
                surrounding: rng.gen_bool(0.5),
            });
        }
        if rng.gen_bool(0.3) { out.push(Implicit::ColorMismatch { value: 0.1 }); }
        if rng.gen_bool(0.3) { out.push(Implicit::RowPos { value: 0.05 }); }
        if rng.gen_bool(0.3) { out.push(Implicit::EmptySlots { value: 0.12 }); }
        if rng.gen_bool(0.3) { out.push(Implicit::UniqueGroups { value: 0.08 }); }
        if rng.gen_bool(0.35) { out.push(Implicit::Mirror { value: 1.5 }); }
        out
    }

    /// Random decks × random move sequences across the config matrix: after
    /// every propose(), and after every rollback(), the delta score must be
    /// bit-for-bit identical to a fresh full simulate().
    #[test]
    fn delta_full_equiv() {
        let mut rng = SmallRng::seed_from_u64(0x00DE_CAF5);
        for case in 0..200u32 {
            let mut slots: Vec<(i32, i32)> = Vec::new();
            for r in 0..6i32 {
                for c in 0..6i32 {
                    if rng.gen_bool(0.7) { slots.push((r, c)); }
                }
            }
            while slots.len() < 4 {
                let p = (rng.gen_range(0..6), rng.gen_range(0..6));
                if !slots.contains(&p) { slots.push(p); }
            }
            let n = slots.len();
            let (row_peers, col_peers, surr_peers, diag_peers) = build_peers(&slots);
            let arcane_slot_indices: Vec<usize> =
                (0..n).filter(|_| rng.gen_bool(0.12)).collect();
            let cfg_v = rand_cfg(&mut rng);
            let implicits = rand_implicits(&mut rng);
            let core_specs = rand_cores(&mut rng);

            let run = TagRun {
                slots: &slots,
                row_peers, col_peers, surr_peers, diag_peers,
                arcane_slot_indices,
                stacks: Vec::new(),
                tag_rules: Vec::new(),
                blanket_groups: 0,
                assignable_groups: 0,
                legal_combos: Vec::new(),
                implicits: implicits.clone(),
                cores: core_specs.clone(),
                min_stat_placed: 0,
                final_pass_nonfoil_evo: false,
                exact_groups: true,
                n_iter: 0,
                restarts: 1,
                seed: None,
                cfg: cfg_v,
            };
            let geom = build_geom(&run);
            let cores = Cores::build(&core_specs);
            let cfg = &run.cfg;
            let mut sc = Scratch::new(&geom);

            let mut asgn: Vec<SlotCard> = (0..n).map(|_| rand_card(&mut rng)).collect();
            let mut d = DeltaEval::new(&geom, &asgn, &cores, &implicits, cfg);
            let full0 = simulate(&geom, &asgn, &cores, &implicits, cfg, &mut sc);
            assert_eq!(d.score.to_bits(), full0.to_bits(), "case {}: init mismatch", case);

            for step in 0..250u32 {
                let kind = rng.gen_range(0..3);
                let mut changed: Vec<(usize, SlotCard)> = Vec::new();
                match kind {
                    0 => {
                        let p = rng.gen_range(0..n);
                        let old = asgn[p];
                        asgn[p] = rand_card(&mut rng);
                        changed.push((p, old));
                    }
                    1 if n >= 2 => {
                        let p1 = rng.gen_range(0..n);
                        let mut p2 = rng.gen_range(0..n);
                        while p2 == p1 { p2 = rng.gen_range(0..n); }
                        let o1 = asgn[p1];
                        let o2 = asgn[p2];
                        asgn.swap(p1, p2);
                        changed.push((p1, o1));
                        changed.push((p2, o2));
                    }
                    _ => {
                        let p = rng.gen_range(0..n);
                        if asgn[p].t == T_DEAD { continue; }
                        let old = asgn[p];
                        asgn[p].groups ^= 1u16 << rng.gen_range(0..N_GROUP_BITS);
                        changed.push((p, old));
                    }
                }
                let sc_delta = d.propose(&geom, &cores, &implicits, cfg, &asgn, &changed);
                let sc_full = simulate(&geom, &asgn, &cores, &implicits, cfg, &mut sc);
                assert_eq!(
                    sc_delta.to_bits(), sc_full.to_bits(),
                    "case {} step {}: delta {} != full {}", case, step, sc_delta, sc_full,
                );
                if rng.gen_bool(0.4) {
                    for &(p, old) in changed.iter().rev() { asgn[p] = old; }
                    d.rollback(&geom, &cores, &implicits, cfg, &asgn);
                    let sc_back = simulate(&geom, &asgn, &cores, &implicits, cfg, &mut sc);
                    assert_eq!(
                        d.score.to_bits(), sc_back.to_bits(),
                        "case {} step {}: rollback mismatch", case, step,
                    );
                } else {
                    d.commit();
                }
            }
        }
    }

    /// Chain decks bypass DeltaEval — the SA must still run end to end.
    #[test]
    fn chain_deck_smoke() {
        let slots: Vec<(i32, i32)> = (0..5i32)
            .flat_map(|r| (0..5i32).map(move |c| (r, c)))
            .collect();
        let (row_peers, col_peers, surr_peers, diag_peers) = build_peers(&slots);
        let mut cfg = rand_cfg(&mut SmallRng::seed_from_u64(7));
        cfg.additive_cores = true;
        cfg.greed_additive = true;
        let run = TagRun {
            slots: &slots,
            row_peers, col_peers, surr_peers, diag_peers,
            arcane_slot_indices: Vec::new(),
            stacks: vec![
                CardSpec { t: T_ROW, color: RED, scale: RED, groups: 0,
                           count: u32::MAX, min_place: 0 },
                CardSpec { t: T_G_UP, color: RED, scale: RED, groups: 0,
                           count: u32::MAX, min_place: 0 },
            ],
            tag_rules: Vec::new(),
            blanket_groups: 0,
            assignable_groups: 0,
            legal_combos: Vec::new(),
            implicits: vec![Implicit::Chain { value: 0.1 }],
            cores: Vec::new(),
            min_stat_placed: 0,
            final_pass_nonfoil_evo: false,
            exact_groups: false,
            n_iter: 2000,
            restarts: 1,
            seed: Some(42),
            cfg,
        };
        let (asgn, score) = run_sa_tagged(run);
        assert!(score > 0.0);
        assert_eq!(asgn.len(), 25);
    }
}
