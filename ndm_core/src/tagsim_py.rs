//! PyO3 boundary for the Optimizer 2.0 tag-aware kernel (`tagsim.rs`).
//!
//! Marshals Python kwargs into a `TagRun`, runs the SA (or a score-only
//! pass), and converts the result back to strings. All conversion happens
//! once per call — never on the hot path.

use pyo3::prelude::*;

use crate::tagsim::{
    group_bit_from_str, group_bits_to_strs, run_sa_tagged as run_core,
    score_tagged as score_core, tag_card_type_from_str, tag_card_type_to_str,
    tag_color_from_str, tag_color_to_str, tag_core_from_str, CardSpec,
    CoreSpecIn, Implicit, SlotCard, TagAxis, TagRule, TagRun, TagSimConfig,
    COLOR_NONE,
};

/// Stack tuple: (type, color, scale_color, groups, count, min_place).
/// count < 0 = unlimited.
type PyStack = (String, String, String, Vec<String>, i64, u32);
/// Rule tuple: (axis, key, min, max). max < 0 = uncapped.
type PyRule = (String, String, u32, i64);
/// Implicit tuple: (kind, value, groups, colors, extra).
/// extra = ptype for "freq" ("col"/"surr"/"diag"), range for "adjacency"
/// ("column"/"surrounding"); ignored otherwise.
type PyImplicit = (String, f64, Vec<String>, Vec<String>, String);
/// Core tuple: (core_type, color, override). override < 0 = none.
type PyCore = (String, String, f64);
/// Assignment entry: (type, color, scale_color, groups).
type PyPlaced = (String, String, String, Vec<String>);

fn parse_stacks(stacks: &[PyStack]) -> Vec<CardSpec> {
    stacks.iter().map(|(t, c, sc, groups, count, min_place)| {
        let mut mask = 0u16;
        for g in groups { mask |= group_bit_from_str(g); }
        CardSpec {
            t: tag_card_type_from_str(t),
            color: tag_color_from_str(c),
            scale: if sc.is_empty() { tag_color_from_str(c) } else { tag_color_from_str(sc) },
            groups: mask,
            count: if *count < 0 { u32::MAX } else { *count as u32 },
            min_place: *min_place,
        }
    }).collect()
}

fn parse_rules(rules: &[PyRule]) -> Vec<TagRule> {
    rules.iter().map(|(axis, key, min, max)| {
        let (axis, key) = match axis.as_str() {
            "color" => (TagAxis::Color, tag_color_from_str(key) as u16),
            "type" => (TagAxis::CardType, tag_card_type_from_str(key) as u16),
            "group" => (TagAxis::Group, group_bit_from_str(key)),
            "greed" => (TagAxis::GreedTotal, 0u16),
            other => panic!("tagsim: unknown rule axis: {}", other),
        };
        TagRule {
            axis, key,
            min: *min,
            max: if *max < 0 { u32::MAX } else { *max as u32 },
        }
    }).collect()
}

fn parse_implicits(implicits: &[PyImplicit]) -> Vec<Implicit> {
    implicits.iter().filter_map(|(kind, value, groups, colors, extra)| {
        let mut gmask = 0u16;
        for g in groups { gmask |= group_bit_from_str(g); }
        let mut cmask = 0u8;
        for c in colors { cmask |= 1 << tag_color_from_str(c); }
        match kind.as_str() {
            "global" => Some(Implicit::GlobalFlat { value: *value, groups: gmask, colors: cmask }),
            "freq" => Some(Implicit::Freq { mult: *value, ptype: tag_card_type_from_str(extra) }),
            "adjacency" => Some(Implicit::Adjacency {
                value: *value,
                group: gmask,
                surrounding: extra == "surrounding",
            }),
            "color_mismatch" => Some(Implicit::ColorMismatch { value: *value }),
            "row_pos" => Some(Implicit::RowPos { value: *value }),
            "chain" => Some(Implicit::Chain { value: *value }),
            "empty_slots" => Some(Implicit::EmptySlots { value: *value }),
            "unique_groups" => Some(Implicit::UniqueGroups { value: *value }),
            "mirror" => Some(Implicit::Mirror { value: *value }),
            // gameplay-only implicits are NDM-inert — dropped here.
            "gameplay" | "mystery" => None,
            other => panic!("tagsim: unknown implicit kind: {}", other),
        }
    }).collect()
}

fn parse_group_list(groups: &[String]) -> u16 {
    let mut mask = 0u16;
    for g in groups { mask |= group_bit_from_str(g); }
    mask
}

fn placed_to_py(asgn: &[SlotCard]) -> Vec<PyPlaced> {
    asgn.iter().map(|c| (
        tag_card_type_to_str(c.t).to_owned(),
        tag_color_to_str(c.color).to_owned(),
        tag_color_to_str(c.scale).to_owned(),
        group_bits_to_strs(c.groups).iter().map(|s| s.to_string()).collect(),
    )).collect()
}

fn placed_from_py(asgn: &[PyPlaced]) -> Vec<SlotCard> {
    asgn.iter().map(|(t, c, sc, groups)| {
        let ty = tag_card_type_from_str(t);
        let color = tag_color_from_str(c);
        SlotCard {
            t: ty,
            color,
            scale: if sc.is_empty() { color } else { tag_color_from_str(sc) },
            groups: parse_group_list(groups),
            stack: u16::MAX,
        }
    }).collect()
}

#[allow(clippy::too_many_arguments)]
fn build_run<'a>(
    slots: &'a [(i32, i32)],
    row_peers: Vec<Vec<usize>>,
    col_peers: Vec<Vec<usize>>,
    surr_peers: Vec<Vec<usize>>,
    diag_peers: Vec<Vec<usize>>,
    arcane_slot_indices: Vec<usize>,
    stacks: Vec<CardSpec>,
    tag_rules: Vec<TagRule>,
    blanket_groups: u16,
    assignable_groups: u16,
    legal_combos: Vec<u16>,
    implicits: Vec<Implicit>,
    cores: Vec<CoreSpecIn>,
    min_stat_placed: u32,
    final_pass_nonfoil_evo: bool,
    exact_groups: bool,
    n_iter: usize,
    restarts: usize,
    seed: Option<u64>,
    cfg: TagSimConfig,
) -> TagRun<'a> {
    TagRun {
        slots, row_peers, col_peers, surr_peers, diag_peers,
        arcane_slot_indices, stacks, tag_rules, blanket_groups,
        assignable_groups, legal_combos, implicits, cores, min_stat_placed,
        final_pass_nonfoil_evo, exact_groups, n_iter, restarts, seed, cfg,
    }
}

fn parse_combo_masks(combos: &Option<Vec<Vec<String>>>) -> Vec<u16> {
    combos.as_ref().map(|list| {
        list.iter().map(|combo| parse_group_list(combo)).collect()
    }).unwrap_or_default()
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    slots, row_peers, col_peers, surr_peers, diag_peers, arcane_slot_indices,
    stacks, tag_rules, blanket_groups, assignable_groups, implicits, cores,
    min_stat_placed, final_pass_nonfoil_evo, exact_groups, n_iter, restarts,
    mult_dir_vert, mult_dir_horiz, mult_pure_base, mult_pure_scale,
    mult_equilibrium, mult_foil, mult_steadfast, mult_sparkling, mult_color,
    mult_deluxe_flat, mult_deluxe_core_base, mult_deluxe_core_scale,
    mult_void_core_base, mult_void_core_scale, mult_archive_core,
    greed_additive, additive_cores, is_shiny, auto_place_arcane,
    colors_real, complex_cards, wv_foil_rules, floor_counts_deluxe,
    seed = None, legal_combos = None,
))]
pub fn run_sa_tagged(
    slots: Vec<(i32, i32)>,
    row_peers: Vec<Vec<usize>>,
    col_peers: Vec<Vec<usize>>,
    surr_peers: Vec<Vec<usize>>,
    diag_peers: Vec<Vec<usize>>,
    arcane_slot_indices: Vec<usize>,
    stacks: Vec<PyStack>,
    tag_rules: Vec<PyRule>,
    blanket_groups: Vec<String>,
    assignable_groups: Vec<String>,
    implicits: Vec<PyImplicit>,
    cores: Vec<PyCore>,
    min_stat_placed: u32,
    final_pass_nonfoil_evo: bool,
    exact_groups: bool,
    n_iter: usize,
    restarts: usize,
    mult_dir_vert: f64,
    mult_dir_horiz: f64,
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
    mult_archive_core: f64,
    greed_additive: bool,
    additive_cores: bool,
    is_shiny: bool,
    auto_place_arcane: bool,
    colors_real: bool,
    complex_cards: bool,
    wv_foil_rules: bool,
    floor_counts_deluxe: bool,
    seed: Option<u64>,
    legal_combos: Option<Vec<Vec<String>>>,
) -> PyResult<(Vec<PyPlaced>, f64)> {
    let cfg = TagSimConfig {
        mult_dir_vert, mult_dir_horiz, mult_pure_base, mult_pure_scale,
        mult_equilibrium, mult_foil, mult_steadfast, mult_sparkling,
        mult_color, mult_deluxe_flat, mult_deluxe_core_base,
        mult_deluxe_core_scale, mult_void_core_base, mult_void_core_scale,
        mult_archive_core, greed_additive, additive_cores, is_shiny,
        auto_place_arcane, colors_real, complex: complex_cards,
        wv_foil_rules, floor_counts_deluxe,
    };
    let cores_in: Vec<CoreSpecIn> = cores.iter().map(|(t, c, o)| CoreSpecIn {
        core_type: tag_core_from_str(t),
        color: if c.is_empty() { COLOR_NONE } else { tag_color_from_str(c) },
        override_: *o,
    }).collect();

    let run = build_run(
        &slots, row_peers, col_peers, surr_peers, diag_peers,
        arcane_slot_indices, parse_stacks(&stacks), parse_rules(&tag_rules),
        parse_group_list(&blanket_groups), parse_group_list(&assignable_groups),
        parse_combo_masks(&legal_combos),
        parse_implicits(&implicits), cores_in, min_stat_placed,
        final_pass_nonfoil_evo, exact_groups, n_iter, restarts, seed, cfg,
    );

    let (asgn, score) = run_core(run);
    Ok((placed_to_py(&asgn), score))
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    slots, row_peers, col_peers, surr_peers, diag_peers, arcane_slot_indices,
    assignment, implicits, cores,
    mult_dir_vert, mult_dir_horiz, mult_pure_base, mult_pure_scale,
    mult_equilibrium, mult_foil, mult_steadfast, mult_sparkling, mult_color,
    mult_deluxe_flat, mult_deluxe_core_base, mult_deluxe_core_scale,
    mult_void_core_base, mult_void_core_scale, mult_archive_core,
    greed_additive, additive_cores, is_shiny,
    colors_real, complex_cards, wv_foil_rules,
))]
pub fn score_tagged(
    slots: Vec<(i32, i32)>,
    row_peers: Vec<Vec<usize>>,
    col_peers: Vec<Vec<usize>>,
    surr_peers: Vec<Vec<usize>>,
    diag_peers: Vec<Vec<usize>>,
    arcane_slot_indices: Vec<usize>,
    assignment: Vec<PyPlaced>,
    implicits: Vec<PyImplicit>,
    cores: Vec<PyCore>,
    mult_dir_vert: f64,
    mult_dir_horiz: f64,
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
    mult_archive_core: f64,
    greed_additive: bool,
    additive_cores: bool,
    is_shiny: bool,
    colors_real: bool,
    complex_cards: bool,
    wv_foil_rules: bool,
) -> PyResult<f64> {
    let cfg = TagSimConfig {
        mult_dir_vert, mult_dir_horiz, mult_pure_base, mult_pure_scale,
        mult_equilibrium, mult_foil, mult_steadfast, mult_sparkling,
        mult_color, mult_deluxe_flat, mult_deluxe_core_base,
        mult_deluxe_core_scale, mult_void_core_base, mult_void_core_scale,
        mult_archive_core, greed_additive, additive_cores, is_shiny,
        auto_place_arcane: true, colors_real, complex: complex_cards,
        wv_foil_rules, floor_counts_deluxe: false,
    };
    let cores_in: Vec<CoreSpecIn> = cores.iter().map(|(t, c, o)| CoreSpecIn {
        core_type: tag_core_from_str(t),
        color: if c.is_empty() { COLOR_NONE } else { tag_color_from_str(c) },
        override_: *o,
    }).collect();

    let run = build_run(
        &slots, row_peers, col_peers, surr_peers, diag_peers,
        arcane_slot_indices, Vec::new(), Vec::new(), 0, 0, Vec::new(),
        parse_implicits(&implicits), cores_in, 0, false, true, 0, 1, None, cfg,
    );

    Ok(score_core(&run, &placed_from_py(&assignment)))
}
