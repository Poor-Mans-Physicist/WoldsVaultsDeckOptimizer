//! wasm-bindgen boundary for the Optimizer 2.0 tag-aware kernel.
//!
//! Two entries:
//!   * `runSaTagged(input)`   — full SA over a serde-deserialized run spec.
//!   * `scoreTagged(input)`   — score-only pass on a fixed assignment (the
//!     what-if tag-edit popup and snapshot restores use this).
//!
//! Every list-y field defaults to empty so older payloads stay loadable.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::tagsim::{
    group_bit_from_str, group_bits_to_strs, run_sa_tagged as run_core,
    score_tagged as score_core, tag_card_type_from_str, tag_card_type_to_str,
    tag_color_from_str, tag_color_to_str, tag_core_from_str, CardSpec,
    CoreSpecIn, Implicit, SlotCard, TagAxis, TagRule, TagRun, TagSimConfig,
    COLOR_NONE,
};

#[derive(Deserialize)]
pub struct StackIn {
    pub t: String,
    pub color: String,
    #[serde(default)]
    pub scale_color: String,
    #[serde(default)]
    pub groups: Vec<String>,
    /// null / missing = unlimited supply.
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub min_place: u32,
}

#[derive(Deserialize)]
pub struct RuleIn {
    pub axis: String,     // "color" | "type" | "group" | "greed"
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub min: u32,
    /// null / missing = uncapped.
    #[serde(default)]
    pub max: Option<u32>,
}

#[derive(Deserialize)]
pub struct ImplicitIn {
    pub kind: String,
    #[serde(default)]
    pub value: f64,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub colors: Vec<String>,
    /// ptype for "freq"; range ("column"/"surrounding") for "adjacency".
    #[serde(default)]
    pub extra: String,
}

#[derive(Deserialize)]
pub struct TagRunInput {
    pub slots: Vec<(i32, i32)>,
    pub row_peers: Vec<Vec<usize>>,
    pub col_peers: Vec<Vec<usize>>,
    pub surr_peers: Vec<Vec<usize>>,
    pub diag_peers: Vec<Vec<usize>>,
    #[serde(default)]
    pub arcane_slot_indices: Vec<usize>,
    #[serde(default)]
    pub stacks: Vec<StackIn>,
    #[serde(default)]
    pub tag_rules: Vec<RuleIn>,
    #[serde(default)]
    pub blanket_groups: Vec<String>,
    #[serde(default)]
    pub assignable_groups: Vec<String>,
    #[serde(default)]
    pub implicits: Vec<ImplicitIn>,
    /// Each entry: [core_type, color_or_empty, override_or_negative].
    #[serde(default)]
    pub cores: Vec<(String, String, f64)>,
    #[serde(default)]
    pub min_stat_placed: u32,
    #[serde(default)]
    pub final_pass_nonfoil_evo: bool,
    #[serde(default)]
    pub exact_groups: bool,
    pub n_iter: usize,
    pub restarts: usize,
    #[serde(default)]
    pub seed: Option<u64>,

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
    #[serde(default)]
    pub mult_void_core_base: f64,
    #[serde(default)]
    pub mult_void_core_scale: f64,
    #[serde(default = "default_one")]
    pub mult_archive_core: f64,
    pub greed_additive: bool,
    pub additive_cores: bool,
    pub is_shiny: bool,
    #[serde(default = "default_true")]
    pub auto_place_arcane: bool,
    #[serde(default)]
    pub colors_real: bool,
    #[serde(default)]
    pub complex_cards: bool,
    #[serde(default = "default_true")]
    pub wv_foil_rules: bool,
    #[serde(default = "default_true")]
    pub floor_counts_deluxe: bool,

    /// scoreTagged only: fixed assignment, parallel to `slots`.
    /// Each entry: [type, color, scale_color, groups[]].
    #[serde(default)]
    pub assignment: Vec<(String, String, String, Vec<String>)>,
}

fn default_true() -> bool { true }
fn default_one() -> f64 { 1.0 }

#[derive(Serialize)]
pub struct PlacedOut {
    pub t: String,
    pub color: String,
    pub scale_color: String,
    pub groups: Vec<String>,
}

#[derive(Serialize)]
pub struct TagRunResult {
    pub assignment: Vec<PlacedOut>,
    pub score: f64,
}

fn parse_group_list(groups: &[String]) -> u16 {
    let mut mask = 0u16;
    for g in groups { mask |= group_bit_from_str(g); }
    mask
}

fn build_run(inp: &TagRunInput) -> TagRun<'_> {
    let stacks: Vec<CardSpec> = inp.stacks.iter().map(|s| {
        let color = tag_color_from_str(&s.color);
        CardSpec {
            t: tag_card_type_from_str(&s.t),
            color,
            scale: if s.scale_color.is_empty() { color } else { tag_color_from_str(&s.scale_color) },
            groups: parse_group_list(&s.groups),
            count: s.count.unwrap_or(u32::MAX),
            min_place: s.min_place,
        }
    }).collect();

    let tag_rules: Vec<TagRule> = inp.tag_rules.iter().map(|r| {
        let (axis, key) = match r.axis.as_str() {
            "color" => (TagAxis::Color, tag_color_from_str(&r.key) as u16),
            "type" => (TagAxis::CardType, tag_card_type_from_str(&r.key) as u16),
            "group" => (TagAxis::Group, group_bit_from_str(&r.key)),
            "greed" => (TagAxis::GreedTotal, 0u16),
            other => panic!("tagsim: unknown rule axis: {}", other),
        };
        TagRule { axis, key, min: r.min, max: r.max.unwrap_or(u32::MAX) }
    }).collect();

    let implicits: Vec<Implicit> = inp.implicits.iter().filter_map(|im| {
        let gmask = parse_group_list(&im.groups);
        let mut cmask = 0u8;
        for c in &im.colors { cmask |= 1 << tag_color_from_str(c); }
        match im.kind.as_str() {
            "global" => Some(Implicit::GlobalFlat { value: im.value, groups: gmask, colors: cmask }),
            "freq" => Some(Implicit::Freq { mult: im.value, ptype: tag_card_type_from_str(&im.extra) }),
            "adjacency" => Some(Implicit::Adjacency {
                value: im.value, group: gmask,
                surrounding: im.extra == "surrounding",
            }),
            "color_mismatch" => Some(Implicit::ColorMismatch { value: im.value }),
            "row_pos" => Some(Implicit::RowPos { value: im.value }),
            "chain" => Some(Implicit::Chain { value: im.value }),
            "empty_slots" => Some(Implicit::EmptySlots { value: im.value }),
            "unique_groups" => Some(Implicit::UniqueGroups { value: im.value }),
            "mirror" => Some(Implicit::Mirror { value: im.value }),
            "gameplay" | "mystery" => None,
            other => panic!("tagsim: unknown implicit kind: {}", other),
        }
    }).collect();

    let cores: Vec<CoreSpecIn> = inp.cores.iter().map(|(t, c, o)| CoreSpecIn {
        core_type: tag_core_from_str(t),
        color: if c.is_empty() { COLOR_NONE } else { tag_color_from_str(c) },
        override_: *o,
    }).collect();

    TagRun {
        slots: &inp.slots,
        row_peers: inp.row_peers.clone(),
        col_peers: inp.col_peers.clone(),
        surr_peers: inp.surr_peers.clone(),
        diag_peers: inp.diag_peers.clone(),
        arcane_slot_indices: inp.arcane_slot_indices.clone(),
        stacks, tag_rules,
        blanket_groups: parse_group_list(&inp.blanket_groups),
        assignable_groups: parse_group_list(&inp.assignable_groups),
        implicits, cores,
        min_stat_placed: inp.min_stat_placed,
        final_pass_nonfoil_evo: inp.final_pass_nonfoil_evo,
        exact_groups: inp.exact_groups,
        n_iter: inp.n_iter,
        restarts: inp.restarts,
        seed: inp.seed,
        cfg: TagSimConfig {
            mult_dir_vert: inp.mult_dir_vert,
            mult_dir_horiz: inp.mult_dir_horiz,
            mult_pure_base: inp.mult_pure_base,
            mult_pure_scale: inp.mult_pure_scale,
            mult_equilibrium: inp.mult_equilibrium,
            mult_foil: inp.mult_foil,
            mult_steadfast: inp.mult_steadfast,
            mult_sparkling: inp.mult_sparkling,
            mult_color: inp.mult_color,
            mult_deluxe_flat: inp.mult_deluxe_flat,
            mult_deluxe_core_base: inp.mult_deluxe_core_base,
            mult_deluxe_core_scale: inp.mult_deluxe_core_scale,
            mult_void_core_base: inp.mult_void_core_base,
            mult_void_core_scale: inp.mult_void_core_scale,
            mult_archive_core: inp.mult_archive_core,
            greed_additive: inp.greed_additive,
            additive_cores: inp.additive_cores,
            is_shiny: inp.is_shiny,
            auto_place_arcane: inp.auto_place_arcane,
            colors_real: inp.colors_real,
            complex: inp.complex_cards,
            wv_foil_rules: inp.wv_foil_rules,
            floor_counts_deluxe: inp.floor_counts_deluxe,
        },
    }
}

fn placed_out(asgn: &[SlotCard]) -> Vec<PlacedOut> {
    asgn.iter().map(|c| PlacedOut {
        t: tag_card_type_to_str(c.t).to_owned(),
        color: tag_color_to_str(c.color).to_owned(),
        scale_color: tag_color_to_str(c.scale).to_owned(),
        groups: group_bits_to_strs(c.groups).iter().map(|s| s.to_string()).collect(),
    }).collect()
}

#[wasm_bindgen(js_name = runSaTagged)]
pub fn run_sa_tagged_wasm(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: TagRunInput = serde_wasm_bindgen::from_value(input)
        .map_err(|e| JsValue::from_str(&format!("tagsim input deserialize failed: {e}")))?;
    let run = build_run(&inp);
    let (asgn, score) = run_core(run);
    let out = TagRunResult { assignment: placed_out(&asgn), score };
    serde_wasm_bindgen::to_value(&out)
        .map_err(|e| JsValue::from_str(&format!("tagsim output serialize failed: {e}")))
}

#[wasm_bindgen(js_name = scoreTagged)]
pub fn score_tagged_wasm(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: TagRunInput = serde_wasm_bindgen::from_value(input)
        .map_err(|e| JsValue::from_str(&format!("tagsim input deserialize failed: {e}")))?;
    let run = build_run(&inp);
    let asgn: Vec<SlotCard> = inp.assignment.iter().map(|(t, c, sc, groups)| {
        let ty = tag_card_type_from_str(t);
        let color = tag_color_from_str(c);
        SlotCard {
            t: ty,
            color,
            scale: if sc.is_empty() { color } else { tag_color_from_str(sc) },
            groups: parse_group_list(groups),
            stack: u16::MAX,
        }
    }).collect();
    let score = score_core(&run, &asgn);
    serde_wasm_bindgen::to_value(&score)
        .map_err(|e| JsValue::from_str(&format!("tagsim output serialize failed: {e}")))
}
