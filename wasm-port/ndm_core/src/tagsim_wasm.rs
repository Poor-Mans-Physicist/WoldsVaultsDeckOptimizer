//! wasm-bindgen boundary for the Optimizer 2.0 tag-aware kernel.
//!
//! Entries:
//!   * `runSaTagged(input)` — full SA over a serde-deserialized run spec. With `starts` set it instead
//!     re-anneals each given layout (phase 2 spread across the caller's workers).
//!   * `scoreTagged(input)` — score-only pass on a fixed assignment (the what-if tag-edit popup and snapshot
//!     restores use this).
//!
//! Every list-y field defaults to empty so older payloads stay loadable. Unknown names (card types, colors,
//! groups, cores, rule axes, implicit kinds) come back as errors, never panics: a panic traps the WASM instance.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::tagsim::{
    group_bits_to_strs, run_sa_from, run_sa_tagged as run_core, score_tagged as score_core,
    tag_card_type_to_str, tag_color_to_str, try_group_bit_from_str, try_tag_card_type_from_str,
    try_tag_color_from_str, try_tag_core_from_str, CardSpec, CoreSpecIn, Implicit, Phase2, SaSchedule,
    SearchOpts, SlotCard, TagAxis, TagRule, TagRun, TagSimConfig, COLOR_NONE, CORE_ARSENAL,
    N_LEGAL_CLASSES, SCHEDULE_LEGACY,
};

#[derive(Deserialize, Clone)]
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

#[derive(Deserialize, Clone)]
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

#[derive(Deserialize, Clone)]
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

/// SA temperature schedule: geometric [t0, t1] or target [a0, a1, t_init, eta].
#[derive(Deserialize, Clone)]
pub struct ScheduleIn {
    pub kind: String,
    pub params: Vec<f64>,
}

#[derive(Deserialize, Clone)]
pub struct Phase2In {
    pub top_k: usize,
    pub n_iter: usize,
    pub schedule: ScheduleIn,
}

/// A placed card as `runSaTagged` returns it: [type, color, scale_color, groups[], stack].
pub type PlacedIn = (String, String, String, Vec<String>, i32);

#[derive(Deserialize, Clone)]
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
    /// Category-tag combos that exist on real cards; a card's category set
    /// must stay a subset of one. Empty = unconstrained.
    #[serde(default)]
    pub legal_combos: Vec<Vec<String>>,
    /// The same per card class: [evolution, shiny, plain stat, deluxe stat, non-stat]. Empty = use legal_combos.
    #[serde(default)]
    pub legal_by_class: Vec<Vec<Vec<String>>>,
    #[serde(default)]
    pub implicits: Vec<ImplicitIn>,
    /// Each entry: [core_type, color_or_empty, override_or_negative]. The 0.34.1 addon cores need a value.
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
    /// Missing = the legacy geometric schedule (T 100 -> 0.5).
    #[serde(default)]
    pub schedule: Option<ScheduleIn>,
    #[serde(default)]
    pub phase2: Option<Phase2In>,
    #[serde(default)]
    pub polish: bool,
    #[serde(default)]
    pub legal_only: bool,
    /// runSaTagged only: re-anneal these layouts (parallel to `slots`) instead of cold restarts.
    #[serde(default)]
    pub starts: Vec<Vec<PlacedIn>>,

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
    /// The stack this card came from (-1 = none, e.g. a dead slot).
    pub stack: i32,
}

#[derive(Serialize)]
pub struct TagRunResult {
    pub assignment: Vec<PlacedOut>,
    pub score: f64,
}

type R<T> = Result<T, String>;

fn color(s: &str) -> R<u8> { try_tag_color_from_str(s).ok_or_else(|| format!("unknown color: {s:?}")) }
fn card_type(s: &str) -> R<u8> { try_tag_card_type_from_str(s).ok_or_else(|| format!("unknown card type: {s:?}")) }

fn groups(gs: &[String]) -> R<u16> {
    let mut mask = 0u16;
    for g in gs {
        mask |= try_group_bit_from_str(g).ok_or_else(|| format!("unknown group: {g:?}"))?;
    }
    Ok(mask)
}

fn schedule(s: &ScheduleIn) -> R<SaSchedule> {
    match (s.kind.as_str(), s.params.as_slice()) {
        ("geometric", &[t0, t1]) => Ok(SaSchedule::Geometric { t0, t1 }),
        ("target", &[a0, a1, t_init, eta]) => Ok(SaSchedule::Target { a0, a1, t_init, eta }),
        (k, p) => Err(format!("bad schedule {k:?} {p:?}: want geometric [t0, t1] or target [a0, a1, t_init, eta]")),
    }
}

pub(crate) fn card(t: &str, c: &str, sc: &str, gs: &[String], stack: i32) -> R<SlotCard> {
    let col = color(c)?;
    Ok(SlotCard {
        t: card_type(t)?,
        color: col,
        scale: if sc.is_empty() { col } else { color(sc)? },
        groups: groups(gs)?,
        stack: if stack < 0 { u16::MAX } else { stack as u16 },
    })
}

pub(crate) fn build_run(inp: &TagRunInput) -> R<TagRun<'_>> {
    let mut stacks = Vec::with_capacity(inp.stacks.len());
    for s in &inp.stacks {
        let c = color(&s.color)?;
        stacks.push(CardSpec {
            t: card_type(&s.t)?,
            color: c,
            scale: if s.scale_color.is_empty() { c } else { color(&s.scale_color)? },
            groups: groups(&s.groups)?,
            count: s.count.unwrap_or(u32::MAX),
            min_place: s.min_place,
        });
    }

    let mut tag_rules = Vec::with_capacity(inp.tag_rules.len());
    for r in &inp.tag_rules {
        let (axis, key) = match r.axis.as_str() {
            "color" => (TagAxis::Color, color(&r.key)? as u16),
            "type" => (TagAxis::CardType, card_type(&r.key)? as u16),
            "group" => (TagAxis::Group, groups(std::slice::from_ref(&r.key))?),
            "greed" => (TagAxis::GreedTotal, 0u16),
            other => return Err(format!("unknown rule axis: {other:?}")),
        };
        tag_rules.push(TagRule { axis, key, min: r.min, max: r.max.unwrap_or(u32::MAX) });
    }

    let mut implicits = Vec::with_capacity(inp.implicits.len());
    for im in &inp.implicits {
        let gmask = groups(&im.groups)?;
        let mut cmask = 0u8;
        for c in &im.colors { cmask |= 1 << color(c)?; }
        implicits.push(match im.kind.as_str() {
            "global" => Implicit::GlobalFlat { value: im.value, groups: gmask, colors: cmask },
            "freq" => Implicit::Freq { mult: im.value, ptype: card_type(&im.extra)? },
            "adjacency" => Implicit::Adjacency { value: im.value, group: gmask, surrounding: im.extra == "surrounding" },
            "color_mismatch" => Implicit::ColorMismatch { value: im.value },
            "row_pos" => Implicit::RowPos { value: im.value },
            "chain" => Implicit::Chain { value: im.value },
            "empty_slots" => Implicit::EmptySlots { value: im.value },
            "unique_groups" => Implicit::UniqueGroups { value: im.value },
            "mirror" => Implicit::Mirror { value: im.value },
            "gameplay" | "mystery" => continue,
            other => return Err(format!("unknown implicit kind: {other:?}")),
        });
    }

    let mut cores = Vec::with_capacity(inp.cores.len());
    for (t, c, o) in &inp.cores {
        let core_type = try_tag_core_from_str(t).ok_or_else(|| format!("unknown core type: {t:?}"))?;
        if core_type >= CORE_ARSENAL && *o < 0.0 {
            return Err(format!("core {t:?} needs a value"));
        }
        cores.push(CoreSpecIn {
            core_type,
            color: if c.is_empty() { COLOR_NONE } else { color(c)? },
            override_: *o,
        });
    }

    let n_slots = inp.slots.len();
    for (name, peers) in [("row", &inp.row_peers), ("col", &inp.col_peers), ("surr", &inp.surr_peers),
                          ("diag", &inp.diag_peers)] {
        if peers.len() != n_slots || peers.iter().flatten().any(|&q| q >= n_slots) {
            return Err(format!("{name}_peers does not match the {n_slots} slots"));
        }
    }
    if inp.legal_by_class.len() > N_LEGAL_CLASSES {
        return Err(format!("legal_by_class has {} classes, max {N_LEGAL_CLASSES}", inp.legal_by_class.len()));
    }

    let sched = match &inp.schedule { Some(s) => schedule(s)?, None => SCHEDULE_LEGACY };
    let phase2 = match &inp.phase2 {
        Some(p) => Some(Phase2 { top_k: p.top_k, n_iter: p.n_iter, sched: schedule(&p.schedule)? }),
        None => None,
    };

    Ok(TagRun {
        slots: &inp.slots,
        row_peers: inp.row_peers.clone(),
        col_peers: inp.col_peers.clone(),
        surr_peers: inp.surr_peers.clone(),
        diag_peers: inp.diag_peers.clone(),
        arcane_slot_indices: inp.arcane_slot_indices.clone(),
        stacks, tag_rules,
        blanket_groups: groups(&inp.blanket_groups)?,
        assignable_groups: groups(&inp.assignable_groups)?,
        legal_combos: inp.legal_combos.iter().map(|c| groups(c)).collect::<R<Vec<u16>>>()?,
        legal_by_class: inp.legal_by_class.iter()
            .map(|cls| cls.iter().map(|c| groups(c)).collect::<R<Vec<u16>>>())
            .collect::<R<Vec<Vec<u16>>>>()?,
        implicits, cores,
        min_stat_placed: inp.min_stat_placed,
        final_pass_nonfoil_evo: inp.final_pass_nonfoil_evo,
        exact_groups: inp.exact_groups,
        n_iter: inp.n_iter,
        restarts: inp.restarts,
        seed: inp.seed,
        sched,
        opts: SearchOpts { polish: inp.polish, legal_only: inp.legal_only, phase2, ..SearchOpts::default() },
        value_model: None,
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
    })
}

pub(crate) fn placed_out(asgn: &[SlotCard]) -> Vec<PlacedOut> {
    asgn.iter().map(|c| PlacedOut {
        t: tag_card_type_to_str(c.t).to_owned(),
        color: tag_color_to_str(c.color).to_owned(),
        scale_color: tag_color_to_str(c.scale).to_owned(),
        groups: group_bits_to_strs(c.groups).iter().map(|s| s.to_string()).collect(),
        stack: if c.stack == u16::MAX { -1 } else { c.stack as i32 },
    }).collect()
}

fn js_err(e: String) -> JsValue { JsValue::from_str(&format!("tagsim: {e}")) }

fn run_inner(inp: &TagRunInput) -> R<TagRunResult> {
    let run = build_run(inp)?;
    let (asgn, score) = if inp.starts.is_empty() {
        run_core(run)
    } else {
        let n = inp.slots.len();
        let mut starts = Vec::with_capacity(inp.starts.len());
        for s in &inp.starts {
            if s.len() != n { return Err(format!("start layout has {} cards for {n} slots", s.len())); }
            let mut layout = Vec::with_capacity(n);
            for (t, c, sc, gs, stack) in s {
                let k = card(t, c, sc, gs, *stack)?;
                if k.stack != u16::MAX && k.stack as usize >= inp.stacks.len() {
                    return Err(format!("start card stack {} out of range", k.stack));
                }
                layout.push(k);
            }
            starts.push(layout);
        }
        run_sa_from(run, &starts)
    };
    Ok(TagRunResult { assignment: placed_out(&asgn), score })
}

#[wasm_bindgen(js_name = runSaTagged)]
pub fn run_sa_tagged_wasm(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: TagRunInput = serde_wasm_bindgen::from_value(input)
        .map_err(|e| JsValue::from_str(&format!("tagsim input deserialize failed: {e}")))?;
    let out = run_inner(&inp).map_err(js_err)?;
    serde_wasm_bindgen::to_value(&out)
        .map_err(|e| JsValue::from_str(&format!("tagsim output serialize failed: {e}")))
}

#[wasm_bindgen(js_name = scoreTagged)]
pub fn score_tagged_wasm(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: TagRunInput = serde_wasm_bindgen::from_value(input)
        .map_err(|e| JsValue::from_str(&format!("tagsim input deserialize failed: {e}")))?;
    let run = build_run(&inp).map_err(js_err)?;
    let mut asgn = Vec::with_capacity(inp.assignment.len());
    for (t, c, sc, gs) in &inp.assignment {
        asgn.push(card(t, c, sc, gs, -1).map_err(js_err)?);
    }
    if asgn.len() != inp.slots.len() {
        return Err(js_err(format!("assignment has {} cards for {} slots", asgn.len(), inp.slots.len())));
    }
    let score = score_core(&run, &asgn);
    serde_wasm_bindgen::to_value(&score)
        .map_err(|e| JsValue::from_str(&format!("tagsim output serialize failed: {e}")))
}
