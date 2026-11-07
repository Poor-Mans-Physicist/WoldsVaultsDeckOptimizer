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
/// Knack joined in 0.34.1: no Knack card carries Stat (Knack cards grant talent levels).
pub const NONSTAT_GROUPS: u16 = G_RESOURCE | G_TEMPORAL | G_KNACK;

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
// 0.34.1 addon cores (woldsvaults ModDeckCoresProvider). Their value always comes from the core spec's override.
pub const CORE_ARSENAL: u8     = 9;   // +v to Offensive cards
pub const CORE_AEGIS: u8       = 10;  // +v to Defensive cards
pub const CORE_TOOL: u8        = 11;  // +v to Utility cards
pub const CORE_NATURAL: u8     = 12;  // +v to Physical cards
pub const CORE_FAE: u8         = 13;  // +v to Magical cards
pub const CORE_BAZAAR: u8      = 14;  // +v if any of the 8 neighbours is Resource
pub const CORE_TALENT: u8      = 15;  // +v per Knack card among the 8 neighbours
pub const CORE_TEMPORAL: u8    = 16;  // +v if any star-cross cell is Temporal
pub const CORE_JUPITER: u8     = 17;  // +v to cards carrying the deck's most common group (Stat excluded)
pub const CORE_PLUTO: u8       = 18;  // +v to cards carrying the deck's least common group (Stat excluded)
pub const CORE_NITWIT: u8      = 19;  // arcane slots take regular cards at +v; arcane cards are disabled

// Extended group bits (u32): the kernel's card groups plus the groups the game derives from the card type.
// Jupiter/Pluto count every group on every placed card.
pub const XG_EVOLUTION: u32 = 1 << 11;
pub const XG_DELUXE: u32 = 1 << 12;
pub const XG_ARCANE: u32 = 1 << 13;
pub const XG_GREED: u32 = 1 << 14;
pub const XG_SHINY: u32 = 1 << 15;
pub const XG_WILD: u32 = 1 << 16;
pub const XG_STAT: u32 = G_STAT as u32;
pub const N_XG: usize = 17;
/// Game group names in alphabetical order (Dominance tie-break: the first wins).
const XG_ALPHA: [u32; N_XG] = [
    XG_ARCANE, G_DEFENSIVE as u32, XG_DELUXE, G_ESSENCE as u32, XG_EVOLUTION, G_FOIL as u32, XG_GREED,
    G_KNACK as u32, G_MAGICAL as u32, G_OFFENSIVE as u32, G_PHYSICAL as u32, G_RESOURCE as u32, XG_SHINY,
    XG_STAT, G_TEMPORAL as u32, G_UTILITY as u32, XG_WILD,
];
/// The Wild ("dummy") card's groups in the 0.34.1 card dump.
const XG_WILD_SET: u32 = XG_WILD | XG_ARCANE | XG_EVOLUTION | XG_STAT
    | (G_DEFENSIVE | G_KNACK | G_MAGICAL | G_PHYSICAL | G_RESOURCE | G_UTILITY) as u32;

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
    try_tag_core_from_str(s).unwrap_or_else(|| panic!("tagsim: unknown core type: {}", s))
}

pub fn try_tag_core_from_str(s: &str) -> Option<u8> {
    Some(match s {
        "pure" => CORE_PURE, "equilibrium" => CORE_EQUILIBRIUM,
        "steadfast" => CORE_STEADFAST, "sparkling" => CORE_SPARKLING,
        "color" => CORE_COLOR, "foil" => CORE_FOIL,
        "deluxe_core" => CORE_DELUXE, "void_core" => CORE_VOID,
        "archive_core" => CORE_ARCHIVE,
        "arsenal" => CORE_ARSENAL, "aegis" => CORE_AEGIS, "tool" => CORE_TOOL,
        "natural" => CORE_NATURAL, "fae" => CORE_FAE,
        "bazaar" => CORE_BAZAAR, "talent" => CORE_TALENT, "temporal" => CORE_TEMPORAL,
        "jupiter" => CORE_JUPITER, "pluto" => CORE_PLUTO, "nitwit" => CORE_NITWIT,
        _ => return None,
    })
}

pub fn try_tag_card_type_from_str(s: &str) -> Option<u8> {
    Some(match s {
        "row" => T_ROW, "col" => T_COL, "surr" => T_SURR, "diag" => T_DIAG,
        "deluxe" => T_DELUXE, "typeless" => T_TYPELESS,
        "dir_greed_up" => T_G_UP, "dir_greed_down" => T_G_DOWN,
        "dir_greed_left" => T_G_LEFT, "dir_greed_right" => T_G_RIGHT,
        "arcane" => T_ARCANE, "wild" => T_WILD, "dead" => T_DEAD,
        _ => return None,
    })
}

pub fn try_tag_color_from_str(s: &str) -> Option<u8> {
    Some(match s {
        "red" => RED, "green" => GREEN, "blue" => BLUE, "yellow" => YELLOW, "" => COLOR_NONE,
        _ => return None,
    })
}

pub fn try_group_bit_from_str(s: &str) -> Option<u16> {
    Some(match s {
        "Offensive" => G_OFFENSIVE, "Defensive" => G_DEFENSIVE,
        "Physical" => G_PHYSICAL, "Magical" => G_MAGICAL,
        "Utility" => G_UTILITY, "Resource" => G_RESOURCE,
        "Knack" => G_KNACK, "Temporal" => G_TEMPORAL,
        "Essence" => G_ESSENCE, "Foil" => G_FOIL, "Stat" => G_STAT,
        _ => return None,
    })
}

/// The game's group set of a placed card (extended bits), as Jupiter/Pluto and the Stat gates read it.
#[inline(always)]
fn xgroups(c: &SlotCard, is_shiny: bool) -> u32 {
    if c.t == T_DEAD { return 0; }
    if c.t == T_WILD { return XG_WILD_SET | c.groups as u32; }
    let mut m = c.groups as u32;
    if is_positional(c.t) {
        m |= if is_shiny { XG_SHINY | XG_STAT } else { XG_EVOLUTION };
    } else if c.t == T_DELUXE {
        m |= XG_DELUXE;
        if c.groups & NONSTAT_GROUPS == 0 { m |= XG_STAT; }
    } else if c.t == T_TYPELESS {
        if c.groups & NONSTAT_GROUPS == 0 { m |= XG_STAT; }
    } else if is_greed(c.t) {
        m |= XG_GREED;
    } else if c.t == T_ARCANE {
        m |= XG_ARCANE;
    }
    m
}

/// First group (alphabetical) with the highest (`most`) or lowest positive count, ignoring `exclude`. 0 = none.
fn dominant_group(counts: &[u32; N_XG], most: bool, exclude: u32) -> u32 {
    let mut best: Option<(u32, u32)> = None;
    for &bit in XG_ALPHA.iter() {
        if bit & exclude != 0 { continue; }
        let n = counts[bit.trailing_zeros() as usize];
        if n == 0 { continue; }
        let better = match best {
            None => true,
            Some((_, bn)) => if most { n > bn } else { n < bn },
        };
        if better { best = Some((bit, n)); }
    }
    best.map(|b| b.0).unwrap_or(0)
}

/// Category combo classes for toggle legality: evolution, shiny, plain stat (typeless), deluxe stat, non-stat.
pub const N_LEGAL_CLASSES: usize = 5;

#[inline(always)]
fn legal_class(c: &SlotCard, is_shiny: bool) -> usize {
    if c.groups & NONSTAT_GROUPS != 0 { return 4; }
    if is_positional(c.t) { return if is_shiny { 1 } else { 0 }; }
    if c.t == T_DELUXE { 3 } else { 2 }
}

/// May a card end up with category set `cats`? Per-class combos when given, else the flat list, else free.
#[inline(always)]
fn combo_legal(run: &TagRun<'_>, c: &SlotCard, cats: u16) -> bool {
    let list = match run.legal_by_class.get(legal_class(c, run.cfg.is_shiny)) {
        Some(l) if !l.is_empty() => l,
        _ => &run.legal_combos,
    };
    list.is_empty() || list.iter().any(|&m| cats & !m == 0)
}

/// Implicit entries for the 0.34.1 addon cores in `cores` (their values are the spec overrides; a negative
/// override leaves the core inert).
fn core_implicits(cores: &[CoreSpecIn]) -> Vec<Implicit> {
    let mut out = Vec::new();
    for s in cores {
        let v = s.override_;
        if v < 0.0 { continue; }
        let flat = |g: u16| Implicit::GlobalFlat { value: v - 1.0, groups: g, colors: 0 };
        out.push(match s.core_type {
            CORE_ARSENAL => flat(G_OFFENSIVE),
            CORE_AEGIS => flat(G_DEFENSIVE),
            CORE_TOOL => flat(G_UTILITY),
            CORE_NATURAL => flat(G_PHYSICAL),
            CORE_FAE => flat(G_MAGICAL),
            CORE_BAZAAR => Implicit::AdjacencyAny { value: v - 1.0, group: G_RESOURCE, starcross: false },
            CORE_TALENT => Implicit::Adjacency { value: v, group: G_KNACK, surrounding: true },
            CORE_TEMPORAL => Implicit::AdjacencyAny { value: v - 1.0, group: G_TEMPORAL, starcross: true },
            CORE_JUPITER => Implicit::Dominance { value: v - 1.0, most: true },
            CORE_PLUTO => Implicit::Dominance { value: v - 1.0, most: false },
            CORE_NITWIT => Implicit::Nitwit { value: v - 1.0 },
            _ => continue,
        });
    }
    out
}

/// The run as the kernel scores it: addon cores appended to the implicits.
fn prepared(run: &TagRun<'_>) -> Vec<Implicit> {
    let mut imps = run.implicits.clone();
    imps.extend(core_implicits(&run.cores));
    imps
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
    /// AdjacencyBonusDeckModifier with forEachAdjacent = false (Bazaar, Temporal): +value once when any cell in
    /// range carries `group`. Range: the 8-neighbourhood, or the 12-cell star-cross when `starcross`.
    AdjacencyAny { value: f64, group: u16, starcross: bool },
    /// DominanceDeckModifier excluding Stat (Jupiter `most`, Pluto otherwise): +value to cards carrying the
    /// deck's most / least common group, alphabetical tie-break.
    Dominance { value: f64, most: bool },
    /// Nitwit core: +value to the card in each former arcane slot.
    Nitwit { value: f64 },
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

/// Build-weighted objective (archetype mode). Per stack: the card's stat value at the stage tier and the attribute
/// it feeds (NO_ATTR = none). A placed card adds slot multiplier x value to its attribute's deck total. The score is
/// sum over terms of weight x (min over the term's channels of (base + sum of channel curves) + sum of extra curves),
/// each curve piecewise linear on a uniform grid over the attribute's deck total (flat outside it, zero when empty).
/// A term is one build: damage and survival channels plus additive extras (threshold penalties). One single-channel
/// term is a plain per-attribute sum. None keeps the NDM objective.
#[derive(Clone, Debug)]
pub struct ValueModel {
    pub stack_value: Vec<f64>,
    pub stack_attr: Vec<u8>,
    pub terms: Vec<ValueTerm>,
}

#[derive(Clone, Debug)]
pub struct ValueTerm {
    pub weight: f64,
    pub channels: Vec<ValueChannel>,
    pub extra: Vec<Curve>,
}

#[derive(Clone, Debug)]
pub struct ValueChannel {
    pub base: f64,
    pub curves: Vec<Curve>,
}

#[derive(Clone, Debug)]
pub struct Curve {
    pub x0: f64,
    pub dx: f64,
    pub ys: Vec<f64>,
}

pub const NO_ATTR: u8 = u8::MAX;
pub const MAX_VALUE_ATTRS: usize = 32;

impl Curve {
    #[inline(always)]
    fn at(&self, x: f64) -> f64 {
        let n = self.ys.len();
        if n == 0 { return 0.0; }
        let t = (x - self.x0) / self.dx;
        if t <= 0.0 { return self.ys[0]; }
        let i = t as usize;
        if i >= n - 1 { return self.ys[n - 1]; }
        let f = t - i as f64;
        self.ys[i] + (self.ys[i + 1] - self.ys[i]) * f
    }
}

impl ValueModel {
    #[inline(always)]
    fn slot(&self, c: &SlotCard) -> Option<(usize, f64)> {
        let si = c.stack as usize;
        if c.t == T_DEAD || si >= self.stack_attr.len() { return None; }
        let a = self.stack_attr[si];
        if a == NO_ATTR { None } else { Some((a as usize, self.stack_value[si])) }
    }

    #[inline(always)]
    fn score(&self, x: &[f64; MAX_VALUE_ATTRS]) -> f64 {
        let mut s = 0.0f64;
        for t in &self.terms {
            let mut lo = f64::INFINITY;
            for ch in &t.channels {
                let mut v = ch.base;
                for (a, c) in ch.curves.iter().enumerate() { v += c.at(x[a]); }
                if v < lo { lo = v; }
            }
            if !lo.is_finite() { lo = 0.0; }
            for (a, c) in t.extra.iter().enumerate() { lo += c.at(x[a]); }
            s += t.weight * lo;
        }
        s
    }
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

#[derive(Clone, Copy)]
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

#[derive(Clone)]
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
    /// The same per card class (see `legal_class`): evolution, shiny, plain stat, deluxe stat, non-stat. An
    /// empty or missing class falls back to `legal_combos`.
    pub legal_by_class: Vec<Vec<u16>>,
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
    pub sched: SaSchedule,
    pub opts: SearchOpts,
    pub value_model: Option<ValueModel>,
    pub cfg: TagSimConfig,
}

/// SA temperature schedule.
/// Geometric: absolute T from t0 to t1 (the 1.x/2.0 schedule, in NDM units).
/// Target: T adapts so the share of accepted worse moves tracks a0 → a1
/// (geometric in run progress); `t_init` is relative to the start score, so
/// the schedule is independent of the deck's NDM scale.
#[derive(Clone, Copy, Debug)]
pub enum SaSchedule {
    Geometric { t0: f64, t1: f64 },
    Target { a0: f64, a1: f64, t_init: f64, eta: f64 },
}

pub const SCHEDULE_LEGACY: SaSchedule = SaSchedule::Geometric { t0: 100.0, t1: 0.5 };

/// Per-restart temperature state. The geometric arm reproduces the original
/// expression and RNG draw order exactly, so seeded runs are unchanged.
struct Temper {
    sched: SaSchedule,
    t: f64,
    log_ratio: f64,
    target: f64,
    n_iter: f64,
}

impl Temper {
    fn new(sched: SaSchedule, n_iter: usize, score0: f64) -> Self {
        let (t, log_ratio, target) = match sched {
            SaSchedule::Geometric { t0, t1 } => (t0, (t1 / t0).ln(), 0.0),
            SaSchedule::Target { a0, a1, t_init, eta: _ } =>
                (t_init * score0.abs().max(1.0), (a1 / a0).ln(), a0),
        };
        Temper { sched, t, log_ratio, target, n_iter: n_iter as f64 }
    }

    #[inline(always)]
    fn step(&mut self, it: usize) {
        match self.sched {
            SaSchedule::Geometric { t0, .. } => {
                let raw = t0 * (self.log_ratio * it as f64 / self.n_iter).exp();
                self.t = if raw < 1e-10 { 1e-10 } else { raw };
            }
            SaSchedule::Target { a0, .. } => {
                self.target = a0 * (self.log_ratio * it as f64 / self.n_iter).exp();
            }
        }
    }

    #[inline(always)]
    fn accept(&mut self, delta: f64, rng: &mut SmallRng) -> bool {
        if delta >= 0.0 { return true; }
        let ok = rng.gen::<f64>() < (delta / self.t).exp();
        if let SaSchedule::Target { eta, .. } = self.sched {
            let hit = if ok { 1.0 } else { 0.0 };
            self.t = (self.t * (eta * (self.target - hit)).exp()).clamp(1e-12, 1e12);
        }
        ok
    }
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
    /// Per slot: the 12-cell star-cross (orthogonal ±1/±2 and the diagonals), AdjacencyBonus STARCROSS.
    starcross: Vec<Vec<usize>>,
    /// Former arcane slots under a Nitwit core (they are regular slots then).
    nitwit: Vec<bool>,
}

fn build_geom(run: &TagRun<'_>, imps: &[Implicit]) -> Geom {
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
    let nitwit_on = imps.iter().any(|imp| matches!(imp, Implicit::Nitwit { .. }));
    let nitwit = if nitwit_on { std::mem::replace(&mut is_arcane_slot, vec![false; n]) } else { vec![false; n] };
    const STAR: [(i32, i32); 12] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-2, 0), (2, 0), (0, -2), (0, 2),
                                    (-1, -1), (-1, 1), (1, -1), (1, 1)];
    let starcross: Vec<Vec<usize>> = (0..n).map(|i| {
        let mut v: Vec<usize> = STAR.iter().filter_map(|&(dr, dc)| dir(i, dr, dc)).collect();
        v.sort_unstable();
        v
    }).collect();

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
        mirror_of, mirror_self, rows_from_bottom, starcross, nitwit,
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
    vm: Option<ValueModel>,
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
        Cores { list: specs.to_vec(), color_core_color, has_color_core, foil_active, vm: None }
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
    /// Placed cards per extended group (Jupiter/Pluto counts).
    xg_refs: [u32; N_XG],
    is_shiny: bool,
}

impl Counts {
    fn new(geom: &Geom, is_shiny: bool) -> Self {
        Counts {
            row_color: vec![0; geom.row_span * N_COLORS],
            col_color: vec![0; geom.col_span * N_COLORS],
            row_fill: vec![0; geom.row_span],
            col_fill: vec![0; geom.col_span],
            n_deluxe: 0, n_arcane: 0, n_greed: 0, n_dead: 0,
            n_nonfoil: 0, n_wild: 0,
            group_refs: [0; N_GROUP_BITS],
            color_cards: [0; N_COLORS],
            xg_refs: [0; N_XG],
            is_shiny,
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
        let mut xm = xgroups(c, self.is_shiny);
        while xm != 0 {
            bump(&mut self.xg_refs[xm.trailing_zeros() as usize], s);
            xm &= xm - 1;
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
        self.xg_refs = [0; N_XG];
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
    /// Equilibrium + Steadfast: cards carrying Stat only.
    stat_addend: f64,
    stat_factor: f64,
    /// Sparkling: cards carrying Shiny only (positional cards of shiny runs).
    spark_addend: f64,
    spark_factor: f64,
    /// Jupiter / Pluto: the selected extended group bit (0 = none) and the bonus.
    jup_bit: u32,
    jup_value: f64,
    plu_bit: u32,
    plu_value: f64,
}

impl Derived {
    /// Did a Jupiter/Pluto selection move? That changes per-slot folds (full rescan).
    fn dom_differs(&self, other: &Derived) -> bool {
        self.jup_bit != other.jup_bit || self.plu_bit != other.plu_bit
    }

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
            || self.stat_addend.to_bits() != other.stat_addend.to_bits()
            || self.stat_factor.to_bits() != other.stat_factor.to_bits()
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
    let mut stat_addend = 0.0f64;
    let mut stat_factor = 1.0f64;
    let mut spark_addend = 0.0f64;
    let mut spark_factor = 1.0f64;

    for spec in &cores.list {
        let ov = spec.override_ >= 0.0;
        match spec.core_type {
            CORE_PURE => {
                let scale = if ov { spec.override_ } else { cfg.mult_pure_scale };
                let v = cfg.mult_pure_base + scale * n_ns as f64;
                baseline_sum += v - 1.0;
                baseline_prod *= v;
            }
            CORE_EQUILIBRIUM => {
                // StatEfficiencyDeckModifier (audited 2026-08-01): value =
                // 1 + roll × unique deck colors, Stat cards only (shiny
                // positionals, deluxe and plain stat cards in either class).
                // `mult_equilibrium` is the PER-COLOR roll.
                let scale = if ov { spec.override_ } else { cfg.mult_equilibrium };
                let v = 1.0 + scale * n_colors as f64;
                stat_addend += v - 1.0; stat_factor *= v;
            }
            CORE_STEADFAST => {
                // GlobalDeckModifier requiredGroups Stat.
                let v = if ov { spec.override_ } else { cfg.mult_steadfast };
                stat_addend += v - 1.0; stat_factor *= v;
            }
            CORE_SPARKLING if cfg.is_shiny => {
                // GlobalDeckModifier requiredGroups Shiny: shiny positionals only.
                let v = if ov { spec.override_ } else { cfg.mult_sparkling };
                spark_addend += v - 1.0; spark_factor *= v;
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
    let (mut jup_bit, mut jup_value, mut plu_bit, mut plu_value) = (0u32, 0.0f64, 0u32, 0.0f64);

    for imp in implicits {
        match *imp {
            Implicit::Freq { mult, ptype } => {
                freq_mult[ptype as usize] *= mult.round().max(1.0);
            }
            Implicit::Chain { value } => { chain_value = value; have_chain = true; }
            Implicit::EmptySlots { value } => { empty_addend += value * n_dead as f64; }
            Implicit::UniqueGroups { value } => { unique_value = value; have_unique = true; }
            Implicit::Mirror { value } => { mirror_value = value; have_mirror = true; }
            Implicit::Dominance { value, most } => {
                let bit = dominant_group(&counts.xg_refs, most, XG_STAT);
                if most { jup_bit = bit; jup_value = value; } else { plu_bit = bit; plu_value = value; }
            }
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
        stat_addend, stat_factor, spark_addend, spark_factor,
        jup_bit, jup_value, plu_bit, plu_value,
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
    // Premium boosts Deluxe cards too: in 0.34.1 its boostGroups list is empty, so
    // GroupSynergyDeckModifier applies to every card (the "non-Deluxe" tooltip is not enforced).
    let deluxe_applies = der.deluxe_present;

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
            Implicit::AdjacencyAny { value, group, starcross } => {
                let peers: &Vec<usize> =
                    if starcross { &geom.starcross[i] } else { &geom.surr_peers[i] };
                let hit = peers.iter().any(|&q| {
                    let qc = asgn[q];
                    qc.t != T_DEAD && !is_greed(qc.t) && (qc.t == T_WILD || qc.groups & group != 0)
                });
                if hit { imp_addend += value; }
            }
            Implicit::Dominance { most, .. } => {
                let (bit, v) = if most { (der.jup_bit, der.jup_value) } else { (der.plu_bit, der.plu_value) };
                if bit != 0 && xgroups(&c, cfg.is_shiny) & bit != 0 { imp_addend += v; }
            }
            Implicit::Nitwit { value } => {
                if geom.nitwit[i] { imp_addend += value; }
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
    let xg = xgroups(&c, cfg.is_shiny);
    let stat_applies = xg & XG_STAT != 0;
    let spark_applies = xg & XG_SHINY != 0;

    let core_mult = if cfg.additive_cores {
        1.0 + der.baseline_sum
            + if stat_applies { der.stat_addend } else { 0.0 }
            + if spark_applies { der.spark_addend } else { 0.0 }
            + if si.color_applies { der.color_addend } else { 0.0 }
            + if si.deluxe_applies { der.deluxe_addend } else { 0.0 }
            + if der.void_present { der.void_addend } else { 0.0 }
            + der.archive_addend
            + imp_addend
    } else {
        // Vanilla path — implicits never active there; keep the pure
        // multiplicative composition identical to 1.x.
        let mut m = der.baseline_prod;
        if stat_applies { m *= der.stat_factor; }
        if spark_applies { m *= der.spark_factor; }
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
    fn new(geom: &Geom, is_shiny: bool) -> Self {
        Scratch {
            counts: Counts::new(geom, is_shiny),
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
    if let Some(vm) = &cores.vm {
        let mut x = [0.0f64; MAX_VALUE_ATTRS];
        for i in 0..n {
            let si = slot_scan(&ctx, asgn, i);
            if !si.contributes { continue; }
            let t = finish_term(&ctx, asgn, i, &si, s.boost[i]);
            if let Some((a, v)) = vm.slot(&asgn[i]) { x[a] += t * v; }
        }
        return vm.score(&x);
    }
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
    adj_star: bool,
    colormismatch: bool,
    have_unique: bool,
    // Which counters derive_scalars() actually reads for this run.
    sens_ns: bool,
    sens_deluxe: bool,
    sens_dead: bool,
    sens_arcane: bool,
    sens_colors: bool,
    sens_dom: bool,
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
        let mut counts = Counts::new(geom, cfg.is_shiny);
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
                matches!(imp, Implicit::Adjacency { surrounding: true, .. }
                    | Implicit::AdjacencyAny { starcross: false, .. })),
            adj_star: implicits.iter().any(|imp|
                matches!(imp, Implicit::AdjacencyAny { starcross: true, .. })),
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
            sens_colors: cores.list.iter().any(|s| s.core_type == CORE_EQUILIBRIUM),
            sens_dom: implicits.iter().any(|imp| matches!(imp, Implicit::Dominance { .. })),
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
        d.score = d.resum(asgn, cores);
        d
    }

    /// Sum cached terms in slot order with simulate()'s skip structure —
    /// the same adds in the same order, so the same bits.
    #[inline(always)]
    fn resum(&self, asgn: &[SlotCard], cores: &Cores) -> f64 {
        if let Some(vm) = &cores.vm {
            let mut x = [0.0f64; MAX_VALUE_ATTRS];
            for (i, si) in self.inputs.iter().enumerate() {
                if !si.contributes { continue; }
                if let Some((a, v)) = vm.slot(&asgn[i]) { x[a] += self.term[i] * v; }
            }
            return vm.score(&x);
        }
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
        let mut xg_moved = false;
        for k in 0..self.j_cards.len() {
            let (_, old, new) = self.j_cards[k];
            let co = derive_class(&old);
            let cn = derive_class(&new);
            for b in 0..6 { dc[b] += cn[b] - co[b]; }
            union_moved |= Counts::union_mask(&old) != Counts::union_mask(&new);
            xg_moved |= xgroups(&old, cfg.is_shiny) != xgroups(&new, cfg.is_shiny);
            color_moved |= old.color != new.color
                || (old.t == T_WILD) != (new.t == T_WILD)
                || (old.t == T_DEAD) != (new.t == T_DEAD);
        }
        let any_derive = (self.sens_ns && dc[5] != 0)
            || (self.sens_deluxe && dc[2] != 0)
            || (self.sens_dead && dc[0] != 0)
            || (self.sens_arcane && dc[3] != 0)
            || (self.sens_colors && color_moved)
            || (self.sens_dom && xg_moved)
            || (self.have_unique
                && (union_moved || dc.iter().any(|&d| d != 0)));
        let (g_full, g_tail) = if any_derive {
            let new_der = derive_scalars(cores, implicits, cfg, &self.counts);
            // unique_groups_count feeds the per-slot implicit FOLD → rescan.
            let g_full = new_der.unique_groups_count.to_bits()
                != self.j_der.unique_groups_count.to_bits()
                || new_der.dom_differs(&self.j_der);
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
            let adj_ch = (self.adj_col || self.adj_surr || self.adj_star)
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
            if self.adj_star && adj_ch {
                for k2 in 0..geom.starcross[p].len() {
                    let q = geom.starcross[p][k2];
                    self.mark(q);
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

        self.score = self.resum(asgn, cores);
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
                let mut next = c;
                next.groups |= bit;
                if !combo_legal(run, &next, next.groups & ALL_CATEGORY_GROUPS) { continue; }
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

/// Search-time options beyond the schedule (all off = the 2.0 behaviour).
/// `legal_only`: an illegal or no-op draw is redrawn instead of using up an
/// iteration (capped at MAX_REDRAWS consecutive misses).
/// `polish`: after the anneal, greedy first-improvement sweeps over every
/// single-slot replacement and pair swap of the best layout.
/// `phase2`: after all restarts, the `top_k` best layouts are each
/// re-annealed from where they ended for `n_iter` more steps on `sched`.
/// `strata`: after the free restarts, the best layout is re-annealed with a
/// deck-wide count locked to its own value ± 1..=span, once per offset and
/// axis (escapes count basins such as one deluxe card too few). The ends join
/// the phase-2 pool.
#[derive(Clone, Copy, Debug, Default)]
pub struct SearchOpts {
    pub legal_only: bool,
    pub polish: bool,
    pub phase2: Option<Phase2>,
    pub strata: Option<Strata>,
}

pub const STRATA_DEAD: u8 = 1;
pub const STRATA_DELUXE: u8 = 2;
pub const STRATA_GREED: u8 = 4;

#[derive(Clone, Copy, Debug)]
pub struct Strata {
    pub axes: u8,
    pub span: u8,
    pub n_iter: usize,
}

#[inline(always)]
fn on_axis(c: &SlotCard, axis: u8) -> i64 {
    i64::from(match axis {
        STRATA_DEAD => c.t == T_DEAD,
        STRATA_DELUXE => c.t == T_DELUXE,
        _ => is_greed(c.t),
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Phase2 {
    pub top_k: usize,
    pub n_iter: usize,
    pub sched: SaSchedule,
}

const MAX_REDRAWS: u32 = 64;
const POLISH_ROUNDS: usize = 8;
const DEAD_OPTION: usize = usize::MAX;

/// Mutable search state of one layout: the assignment plus every counter the
/// move rules read, and the incremental evaluator. Moves apply in place and
/// score; the caller then accepts or undoes them.
struct Walk<'a> {
    geom: &'a Geom,
    run: &'a TagRun<'a>,
    cores: &'a Cores,
    materialized: &'a [SlotCard],
    asgn: Vec<SlotCard>,
    remaining: Vec<u32>,
    placed: Vec<u32>,
    stat_placed: u32,
    min_stat: u32,
    rules: RuleBook,
    deval: Option<DeltaEval>,
    s: Scratch,
    score: f64,
    tmp_old: Vec<usize>,
    tmp_new: Vec<usize>,
    /// Strata lock (axis, target count): a replacement may only move the
    /// axis count toward the target, never away from it.
    lock: Option<(u8, i64)>,
    lock_count: i64,
}

impl<'a> Walk<'a> {
    fn new(
        geom: &'a Geom,
        run: &'a TagRun<'a>,
        cores: &'a Cores,
        materialized: &'a [SlotCard],
        asgn: Vec<SlotCard>,
        remaining: Vec<u32>,
        rules: RuleBook,
        min_stat: Option<u32>,
    ) -> Self {
        let cfg = &run.cfg;
        let stat_placed = asgn.iter().filter(|c| counts_toward_floor(c, cfg)).count() as u32;
        let mut placed = vec![0u32; run.stacks.len()];
        for c in &asgn {
            if c.t != T_DEAD && (c.stack as usize) < placed.len() {
                placed[c.stack as usize] += 1;
            }
        }
        let mut s = Scratch::new(geom, cfg.is_shiny);
        let score = simulate(geom, &asgn, cores, &run.implicits, cfg, &mut s);
        // Incremental re-scoring (bit-exact — see the DeltaEval contract).
        // CHAIN implicits score through global component topology, and tiny
        // decks pay more in bookkeeping than a full pass costs; both keep the
        // plain simulate() path.
        let use_delta = geom.n >= DELTA_MIN_SLOTS
            && !run.implicits.iter().any(|imp| matches!(imp, Implicit::Chain { .. }));
        let deval = if use_delta {
            let d = DeltaEval::new(geom, &asgn, cores, &run.implicits, cfg);
            debug_assert_eq!(d.score.to_bits(), score.to_bits());
            Some(d)
        } else { None };
        Walk {
            geom, run, cores, materialized, asgn, remaining, placed, stat_placed,
            min_stat: min_stat.unwrap_or_else(|| run.min_stat_placed.min(stat_placed)),
            rules, deval, s, score,
            tmp_old: Vec::new(), tmp_new: Vec::new(),
            lock: None, lock_count: 0,
        }
    }

    fn set_lock(&mut self, axis: u8, target: i64) {
        self.lock = Some((axis, target));
        self.lock_count = self.asgn.iter().map(|c| on_axis(c, axis)).sum();
    }

    #[inline(always)]
    fn lock_delta(&self, old: &SlotCard, new: &SlotCard) -> i64 {
        match self.lock {
            Some((axis, _)) => on_axis(new, axis) - on_axis(old, axis),
            None => 0,
        }
    }

    fn eval(&mut self, changed: &[(usize, SlotCard)]) -> f64 {
        let run = self.run;
        match self.deval.as_mut() {
            Some(d) => d.propose(self.geom, self.cores, &run.implicits, &run.cfg, &self.asgn, changed),
            None => simulate(self.geom, &self.asgn, self.cores, &run.implicits, &run.cfg, &mut self.s),
        }
    }

    fn accept(&mut self, new_score: f64) {
        self.score = new_score;
        if let Some(d) = self.deval.as_mut() { d.commit(); }
    }

    /// Restore the evaluator after `asgn` has been put back.
    fn rollback_eval(&mut self) {
        let run = self.run;
        if let Some(d) = self.deval.as_mut() {
            d.rollback(self.geom, self.cores, &run.implicits, &run.cfg, &self.asgn);
        }
    }

    fn floor_blocks(&self, stat_delta: i32) -> bool {
        self.min_stat > 0 && stat_delta < 0
            && (self.stat_placed as i32 + stat_delta) < self.min_stat as i32
    }

    /// Replace slot p with stack `si` (DEAD_OPTION = empty it). None = the
    /// move is illegal or a no-op (state untouched); Some = applied + scored.
    fn replace(&mut self, p: usize, si: usize) -> Option<f64> {
        let run = self.run;
        let old = self.asgn[p];
        let new = if si == DEAD_OPTION {
            DEAD_CARD
        } else {
            if self.remaining[si] == 0 { return None; }
            self.materialized[si]
        };
        if new == old { return None; }
        let old_si = old.stack as usize;
        if old.t != T_DEAD && old_si < run.stacks.len()
            && self.placed[old_si] <= run.stacks[old_si].min_place {
            return None;
        }
        let stat_delta = (counts_toward_floor(&new, &run.cfg) as i32)
            - (counts_toward_floor(&old, &run.cfg) as i32);
        if self.floor_blocks(stat_delta) { return None; }
        let lock_d = self.lock_delta(&old, &new);
        if let (Some((_, k)), true) = (self.lock, lock_d != 0) {
            let cur = self.lock_count;
            if (cur + lock_d - k).abs() > (cur - k).abs() { return None; }
        }
        if !self.rules.move_ok(&old, &new, &mut self.tmp_old, &mut self.tmp_new) { return None; }

        self.lock_count += lock_d;
        self.rules.apply(&old, &new, &mut self.tmp_old);
        if si != DEAD_OPTION && self.remaining[si] != u32::MAX { self.remaining[si] -= 1; }
        if old.t != T_DEAD && old_si < self.remaining.len() && self.remaining[old_si] != u32::MAX {
            self.remaining[old_si] += 1;
        }
        if old.t != T_DEAD && old_si < self.placed.len() { self.placed[old_si] -= 1; }
        if si != DEAD_OPTION { self.placed[si] += 1; }
        self.stat_placed = (self.stat_placed as i32 + stat_delta) as u32;
        self.asgn[p] = new;
        Some(self.eval(&[(p, old)]))
    }

    fn undo_replace(&mut self, p: usize, si: usize, old: SlotCard) {
        let run = self.run;
        let new = self.asgn[p];
        let old_si = old.stack as usize;
        let stat_delta = (counts_toward_floor(&new, &run.cfg) as i32)
            - (counts_toward_floor(&old, &run.cfg) as i32);
        self.lock_count -= self.lock_delta(&old, &new);
        self.rules.apply(&new, &old, &mut self.tmp_old);
        if si != DEAD_OPTION {
            if self.remaining[si] != u32::MAX { self.remaining[si] += 1; }
            self.placed[si] -= 1;
        }
        if old.t != T_DEAD && old_si < self.remaining.len() {
            if self.remaining[old_si] != u32::MAX { self.remaining[old_si] -= 1; }
            self.placed[old_si] += 1;
        }
        self.stat_placed = (self.stat_placed as i32 - stat_delta) as u32;
        self.asgn[p] = old;
        self.rollback_eval();
    }

    /// Auto-placed arcane slot: swap among arcane stacks only (1.x rule;
    /// per-stack placed counts are not tracked on this path).
    fn replace_locked(&mut self, p: usize, si: usize) -> Option<f64> {
        let old = self.asgn[p];
        if old.t != T_ARCANE { return None; }
        let new = self.materialized[si];
        if new == old || self.remaining[si] == 0 { return None; }
        if !self.rules.move_ok(&old, &new, &mut self.tmp_old, &mut self.tmp_new) { return None; }
        let old_si = old.stack as usize;
        self.rules.apply(&old, &new, &mut self.tmp_old);
        if self.remaining[si] != u32::MAX { self.remaining[si] -= 1; }
        if old_si < self.remaining.len() && self.remaining[old_si] != u32::MAX { self.remaining[old_si] += 1; }
        self.asgn[p] = new;
        Some(self.eval(&[(p, old)]))
    }

    fn undo_replace_locked(&mut self, p: usize, si: usize, old: SlotCard) {
        let new = self.asgn[p];
        let old_si = old.stack as usize;
        self.rules.apply(&new, &old, &mut self.tmp_old);
        if self.remaining[si] != u32::MAX { self.remaining[si] += 1; }
        if old_si < self.remaining.len() && self.remaining[old_si] != u32::MAX { self.remaining[old_si] -= 1; }
        self.asgn[p] = old;
        self.rollback_eval();
    }

    fn swap(&mut self, p1: usize, p2: usize, locked: &[bool]) -> Option<f64> {
        if self.asgn[p1] == self.asgn[p2] { return None; }
        if locked[p1] || locked[p2] { return None; }
        let legal = |arc: bool, t: u8| -> bool {
            if arc { t == T_ARCANE || t == T_DEAD } else { t != T_ARCANE }
        };
        if !legal(self.geom.is_arcane_slot[p1], self.asgn[p2].t)
            || !legal(self.geom.is_arcane_slot[p2], self.asgn[p1].t) {
            return None;
        }
        let (o1, o2) = (self.asgn[p1], self.asgn[p2]);
        self.asgn.swap(p1, p2);
        Some(self.eval(&[(p1, o1), (p2, o2)]))
    }

    fn undo_swap(&mut self, p1: usize, p2: usize) {
        self.asgn.swap(p1, p2);
        self.rollback_eval();
    }

    /// Flip an assignable group bit (Targeted). Returns (score, adding,
    /// stat_delta); rules/floor are only charged on accept (toggle_commit).
    fn toggle(&mut self, p: usize, bit: u16) -> Option<(f64, bool, i32)> {
        let run = self.run;
        let c = self.asgn[p];
        if !is_scorable(c.t) { return None; }
        let adding = c.groups & bit == 0;
        if !self.rules.toggle_ok(bit, adding) { return None; }
        // Adding a category bit must keep the card's category set a subset
        // of some REAL card's set (removal can't break subset legality).
        // Non-stat bits (Resource/Temporal/Knack) flip the card's floor status.
        let mut flipped = c;
        flipped.groups ^= bit;
        if adding && !combo_legal(run, &flipped, flipped.groups & ALL_CATEGORY_GROUPS) { return None; }
        let stat_delta = (counts_toward_floor(&flipped, &run.cfg) as i32)
            - (counts_toward_floor(&c, &run.cfg) as i32);
        if self.floor_blocks(stat_delta) { return None; }
        self.asgn[p].groups ^= bit;
        Some((self.eval(&[(p, c)]), adding, stat_delta))
    }

    fn toggle_commit(&mut self, bit: u16, adding: bool, stat_delta: i32, new_score: f64) {
        self.rules.toggle_apply(bit, adding);
        self.stat_placed = (self.stat_placed as i32 + stat_delta) as u32;
        self.accept(new_score);
    }

    fn undo_toggle(&mut self, p: usize, bit: u16) {
        self.asgn[p].groups ^= bit;
        self.rollback_eval();
    }
}

/// Supply left and rule counts implied by a layout (for re-entering a walk
/// at the best layout).
fn ledger_for(run: &TagRun<'_>, asgn: &[SlotCard]) -> (Vec<u32>, RuleBook) {
    let mut rules = RuleBook::new(&run.tag_rules);
    rules.seed(asgn);
    let mut remaining: Vec<u32> = run.stacks.iter().map(|sp| sp.count).collect();
    for c in asgn {
        let si = c.stack as usize;
        if c.t != T_DEAD && si < remaining.len() && remaining[si] != u32::MAX {
            remaining[si] = remaining[si].saturating_sub(1);
        }
    }
    (remaining, rules)
}

/// Greedy first-improvement polish: every legal single-slot replacement and
/// pair swap, repeated until a full round finds nothing (or POLISH_ROUNDS).
fn polish(w: &mut Walk<'_>, locked: &[bool], regular: &[usize], arcane: &[usize]) {
    let n = w.geom.n;
    let better = |new: f64, cur: f64| new > cur + 1e-12 * cur.abs().max(1.0);
    for _ in 0..POLISH_ROUNDS {
        let mut improved = false;
        for p in 0..n {
            if locked[p] {
                for &si in arcane {
                    let old = w.asgn[p];
                    if let Some(sc) = w.replace_locked(p, si) {
                        if better(sc, w.score) { w.accept(sc); improved = true; }
                        else { w.undo_replace_locked(p, si, old); }
                    }
                }
                continue;
            }
            let opts = if w.geom.is_arcane_slot[p] { arcane } else { regular };
            for k in 0..=opts.len() {
                let si = if k == opts.len() { DEAD_OPTION } else { opts[k] };
                let old = w.asgn[p];
                if let Some(sc) = w.replace(p, si) {
                    if better(sc, w.score) { w.accept(sc); improved = true; }
                    else { w.undo_replace(p, si, old); }
                }
            }
        }
        for p1 in 0..n {
            for p2 in (p1 + 1)..n {
                if let Some(sc) = w.swap(p1, p2, locked) {
                    if better(sc, w.score) { w.accept(sc); improved = true; }
                    else { w.undo_swap(p1, p2); }
                }
            }
        }
        if !improved { break; }
    }
}

fn sa_one_restart(
    geom: &Geom,
    run: &TagRun<'_>,
    cores: &Cores,
    materialized: &[SlotCard],
    seed: u64,
    n_iter: usize,
    sched: SaSchedule,
    start: Option<&[SlotCard]>,
    lock: Option<(u8, i64)>,
) -> (Vec<SlotCard>, f64) {
    let mut rng = SmallRng::seed_from_u64(seed);
    let n = geom.n;
    let cfg = &run.cfg;

    let (asgn, remaining, rules) = match start {
        Some(a) => {
            let (remaining, rules) = ledger_for(run, a);
            (a.to_vec(), remaining, rules)
        }
        None => {
            let mut remaining: Vec<u32> = run.stacks.iter().map(|sp| sp.count).collect();
            let mut rules = RuleBook::new(&run.tag_rules);
            let asgn = initial_fill(geom, run, materialized, &mut remaining, &mut rules);
            (asgn, remaining, rules)
        }
    };
    let mut w = Walk::new(geom, run, cores, materialized, asgn, remaining, rules, None);
    if let Some((axis, target)) = lock { w.set_lock(axis, target); }
    let effective_min_stat = w.min_stat;
    let mut best_score = w.score;
    let mut best_asgn = w.asgn.clone();

    // Proposal alphabets.
    let mut regular_options: Vec<usize> = Vec::new();
    let mut arcane_options: Vec<usize> = Vec::new();
    for (si, spec) in run.stacks.iter().enumerate() {
        if spec.t == T_ARCANE { arcane_options.push(si); }
        else { regular_options.push(si); }
    }

    // Value mode: half the replacements keep the slot's card type and redraw only its stat.
    let value_mode = cores.vm.is_some();
    let mut by_type: Vec<Vec<usize>> = vec![Vec::new(); N_TYPES];
    for &si in &regular_options { by_type[run.stacks[si].t as usize].push(si); }

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

    let mut temper = Temper::new(sched, n_iter, w.score);
    let mut it = 0usize;
    let mut misses = 0u32;

    while it < n_iter {
        temper.step(it);
        let roll: f64 = rng.gen();

        // Each arm yields whether a proposal was scored (false = illegal or
        // no-op draw, which legal_only redraws without spending the step).
        let scored: bool = 'prop: {
            if roll < toggle_prob {
                let p = rng.gen_range(0..n);
                if !is_scorable(w.asgn[p].t) { break 'prop false; }
                let bit = assignable_bits[rng.gen_range(0..assignable_bits.len())];
                let Some((sc, adding, stat_delta)) = w.toggle(p, bit) else { break 'prop false };
                if temper.accept(sc - w.score, &mut rng) {
                    w.toggle_commit(bit, adding, stat_delta, sc);
                } else {
                    w.undo_toggle(p, bit);
                }
            } else if n < 2 || roll < 0.80 + toggle_prob * 0.25 {
                let p = rng.gen_range(0..n);
                let old = w.asgn[p];
                if locked[p] {
                    // Auto-place ON: arcane slots may still swap colors among
                    // arcane stacks (mirrors 1.x behavior).
                    if arcane_options.len() < 2 { break 'prop false; }
                    let si = arcane_options[rng.gen_range(0..arcane_options.len())];
                    let Some(sc) = w.replace_locked(p, si) else { break 'prop false };
                    if temper.accept(sc - w.score, &mut rng) { w.accept(sc); }
                    else { w.undo_replace_locked(p, si, old); }
                } else {
                    // Regular stacks (or arcane stacks for arcane slots when
                    // auto-place is OFF) + DEAD.
                    let opts = if geom.is_arcane_slot[p] { &arcane_options } else { &regular_options };
                    let same = &by_type[old.t as usize];
                    let si = if value_mode && !geom.is_arcane_slot[p] && same.len() > 1 && rng.gen_bool(0.5) {
                        same[rng.gen_range(0..same.len())]
                    } else {
                        let pick = rng.gen_range(0..opts.len() + 1);
                        if pick == opts.len() { DEAD_OPTION } else { opts[pick] }
                    };
                    let Some(sc) = w.replace(p, si) else { break 'prop false };
                    if temper.accept(sc - w.score, &mut rng) { w.accept(sc); }
                    else { w.undo_replace(p, si, old); }
                }
            } else {
                let p1 = rng.gen_range(0..n);
                let mut p2 = rng.gen_range(0..n);
                while p2 == p1 { p2 = rng.gen_range(0..n); }
                let Some(sc) = w.swap(p1, p2, &locked) else { break 'prop false };
                if temper.accept(sc - w.score, &mut rng) { w.accept(sc); }
                else { w.undo_swap(p1, p2); }
            }
            if w.score > best_score {
                best_score = w.score;
                best_asgn.copy_from_slice(&w.asgn);
            }
            true
        };

        if scored || !run.opts.legal_only {
            it += 1;
            misses = 0;
        } else {
            misses += 1;
            if misses >= MAX_REDRAWS { it += 1; misses = 0; }
        }
    }

    if run.opts.polish {
        let (remaining, rules) = ledger_for(run, &best_asgn);
        let mut pw = Walk::new(geom, run, cores, materialized, best_asgn.clone(), remaining,
                               rules, Some(effective_min_stat));
        polish(&mut pw, &locked, &regular_options, &arcane_options);
        if pw.score > best_score {
            best_score = pw.score;
            best_asgn = pw.asgn;
        }
    }

    // §6 final pass — deterministic post-step on the best assignment.
    if run.final_pass_nonfoil_evo && !cfg.is_shiny && cores.foil_active {
        let (mut remaining2, mut rules2) = ledger_for(run, &best_asgn);
        let mut s = Scratch::new(geom, cfg.is_shiny);
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

/// A run ready to anneal: addon-core implicits merged, foil ban applied, per-stack template cards built.
struct Ready<'a> {
    run: TagRun<'a>,
    geom: Geom,
    cores: Cores,
    materialized: Vec<SlotCard>,
    base: u64,
}

fn ready(run: TagRun<'_>) -> Ready<'_> {
    let implicits = prepared(&run);
    let run = TagRun { implicits, ..run };
    let geom = build_geom(&run, &run.implicits);
    let mut cores = Cores::build(&run.cores);
    cores.vm = run.value_model.clone();

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
    let base = run.seed.unwrap_or_else(|| SmallRng::from_entropy().gen::<u64>());
    Ready { run, geom, cores, materialized, base }
}

/// Run the full SA (serial restarts; callers parallelize outside).
pub fn run_sa_tagged(run: TagRun<'_>) -> (Vec<SlotCard>, f64) {
    let Ready { run, geom, cores, materialized, base } = ready(run);
    let restarts = run.restarts.max(1);
    let mut ends: Vec<(Vec<SlotCard>, f64)> = (0..restarts).map(|i| {
        let seed = base ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        sa_one_restart(&geom, &run, &cores, &materialized, seed, run.n_iter, run.sched, None, None)
    }).collect();

    let mut strata_ends: Vec<(Vec<SlotCard>, f64)> = Vec::new();
    if let Some(st) = run.opts.strata {
        let best_i = (0..ends.len()).fold(0, |b, i| if ends[i].1 > ends[b].1 { i } else { b });
        let start = ends[best_i].0.clone();
        let n = geom.n as i64;
        let mut j = 0u64;
        for axis in [STRATA_DEAD, STRATA_DELUXE, STRATA_GREED] {
            if st.axes & axis == 0 { continue; }
            let c: i64 = start.iter().map(|x| on_axis(x, axis)).sum();
            for o in -(st.span as i64)..=(st.span as i64) {
                let k = c + o;
                if o == 0 || k < 0 || k > n { continue; }
                j += 1;
                let seed = base ^ 0x5A17_A5A1_7A5A_17A5 ^ j.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                let r = sa_one_restart(&geom, &run, &cores, &materialized, seed,
                                       st.n_iter, run.sched, None, Some((axis, k)));
                strata_ends.push(r);
            }
        }
    }

    // Phase 2 re-anneals the top_k FREE ends plus the single best strata end;
    // strata ends never compete for the free slots (they would crowd out
    // diverse basins with near-best layouts from the same one).
    if let Some(p2) = run.opts.phase2 {
        ends.sort_by(|a, b| b.1.total_cmp(&a.1));
        strata_ends.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut starts: Vec<Vec<SlotCard>> =
            ends.iter().take(p2.top_k).map(|e| e.0.clone()).collect();
        if let Some(e) = strata_ends.first() { starts.push(e.0.clone()); }
        for (j, start) in starts.iter().enumerate() {
            let seed = base ^ 0xD1B5_4A32_D192_ED03 ^ (j as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let r = sa_one_restart(&geom, &run, &cores, &materialized, seed,
                                   p2.n_iter, p2.sched, Some(start), None);
            ends.push(r);
        }
    }
    ends.extend(strata_ends);
    ends.into_iter().fold(None::<(Vec<SlotCard>, f64)>, |best, r| match best {
        Some(b) if b.1 >= r.1 => Some(b),
        _ => Some(r),
    }).expect("at least one restart")
}

/// Re-anneal from given layouts with `run.sched` and `run.n_iter`, one walk per start (phase 2 spread across
/// callers' workers). Each start must hold cards drawn from the run's stacks (`stack` set). Returns the best end.
pub fn run_sa_from(run: TagRun<'_>, starts: &[Vec<SlotCard>]) -> (Vec<SlotCard>, f64) {
    let Ready { run, geom, cores, materialized, base } = ready(run);
    let mut best: Option<(Vec<SlotCard>, f64)> = None;
    for (j, start) in starts.iter().enumerate() {
        let seed = base ^ 0xD1B5_4A32_D192_ED03 ^ (j as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let r = sa_one_restart(&geom, &run, &cores, &materialized, seed, run.n_iter, run.sched, Some(start), None);
        if best.as_ref().map_or(true, |b| r.1 > b.1) { best = Some(r); }
    }
    best.expect("at least one start")
}

/// Score a FIXED assignment (no SA). Used by the parity harness, the TS
/// what-if popup verification, and snapshot restores.
pub fn score_tagged(run: &TagRun<'_>, asgn: &[SlotCard]) -> f64 {
    let run = &TagRun { implicits: prepared(run), ..run.clone() };
    let geom = build_geom(run, &run.implicits);
    let mut cores = Cores::build(&run.cores);
    cores.vm = run.value_model.clone();
    let mut s = Scratch::new(&geom, run.cfg.is_shiny);
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
        const TYPES: [u8; 20] = [
            CORE_PURE, CORE_EQUILIBRIUM, CORE_STEADFAST, CORE_COLOR,
            CORE_FOIL, CORE_DELUXE, CORE_VOID, CORE_ARCHIVE, CORE_SPARKLING,
            CORE_ARSENAL, CORE_AEGIS, CORE_TOOL, CORE_NATURAL, CORE_FAE,
            CORE_BAZAAR, CORE_TALENT, CORE_TEMPORAL, CORE_JUPITER, CORE_PLUTO, CORE_NITWIT,
        ];
        let mut out = Vec::new();
        for &t in &TYPES {
            if t >= CORE_ARSENAL {
                if rng.gen_bool(0.25) {
                    out.push(CoreSpecIn {
                        core_type: t, color: COLOR_NONE,
                        override_: 1.0 + rng.gen_range(1..=20) as f64 * 0.1,
                    });
                }
                continue;
            }
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
            let mut implicits = rand_implicits(&mut rng);
            let core_specs = rand_cores(&mut rng);
            implicits.extend(core_implicits(&core_specs));

            let run = TagRun {
                slots: &slots,
                row_peers, col_peers, surr_peers, diag_peers,
                arcane_slot_indices,
                stacks: Vec::new(),
                tag_rules: Vec::new(),
                blanket_groups: 0,
                assignable_groups: 0,
                legal_combos: Vec::new(),
                legal_by_class: Vec::new(),
                implicits: implicits.clone(),
                cores: core_specs.clone(),
                min_stat_placed: 0,
                final_pass_nonfoil_evo: false,
                exact_groups: true,
                n_iter: 0,
                restarts: 1,
                seed: None,
                sched: SCHEDULE_LEGACY,
                opts: SearchOpts::default(),
                value_model: None,
                cfg: cfg_v,
            };
            let geom = build_geom(&run, &implicits);
            let mut cores = Cores::build(&core_specs);
            const K: usize = 6;
            if case % 2 == 1 {
                let n_attr = rng.gen_range(1..5usize);
                let stack_value = (0..K).map(|_| rng.gen_range(0.01..10.0)).collect();
                let stack_attr = (0..K).map(|_| if rng.gen_bool(0.2) { NO_ATTR }
                                               else { rng.gen_range(0..n_attr) as u8 }).collect();
                let terms = (0..rng.gen_range(1..4usize)).map(|_| ValueTerm {
                    weight: rng.gen_range(0.1..1.0),
                    channels: (0..rng.gen_range(1..3usize)).map(|_| ValueChannel {
                        base: rng.gen_range(-2.0..2.0),
                        curves: (0..n_attr).map(|_| Curve {
                            x0: 0.0, dx: rng.gen_range(1.0..500.0),
                            ys: (0..16).map(|_| rng.gen_range(-3.0..3.0)).collect(),
                        }).collect(),
                    }).collect(),
                    extra: (0..n_attr).map(|_| Curve {
                        x0: 0.0, dx: rng.gen_range(1.0..500.0),
                        ys: (0..rng.gen_range(0..8usize)).map(|_| rng.gen_range(-3.0..3.0)).collect(),
                    }).collect(),
                }).collect();
                cores.vm = Some(ValueModel { stack_value, stack_attr, terms });
            }
            let cfg = &run.cfg;
            let mut sc = Scratch::new(&geom, cfg.is_shiny);
            let card = |rng: &mut SmallRng| -> SlotCard {
                let mut c = rand_card(rng);
                if c.t != T_DEAD { c.stack = rng.gen_range(0..K as u16); }
                c
            };

            let mut asgn: Vec<SlotCard> = (0..n).map(|_| card(&mut rng)).collect();
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
                        asgn[p] = card(&mut rng);
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
            legal_by_class: Vec::new(),
            implicits: vec![Implicit::Chain { value: 0.1 }],
            cores: Vec::new(),
            min_stat_placed: 0,
            final_pass_nonfoil_evo: false,
            exact_groups: false,
            n_iter: 2000,
            restarts: 1,
            seed: Some(42),
            sched: SCHEDULE_LEGACY,
            opts: SearchOpts::default(),
            value_model: None,
            cfg,
        };
        let (asgn, score) = run_sa_tagged(run);
        assert!(score > 0.0);
        assert_eq!(asgn.len(), 25);
    }
}
