//! wasm-bindgen boundary for the structural (Construction Core) optimizer.
//!
//!   * `constructionEnumerate` — every reachable layout for the roll's k
//!     range, grouped into score-equivalent reflection classes, with exact
//!     per-roll probabilities (plus the player's own layout's).
//!   * `constructionEval`      — score a batch of layouts with the tagged SA
//!     under one fixed run template: cold restarts, or warm walks from given
//!     layouts; optionally returns the best layouts above a threshold.
//!   * `constructionChain`     — one outer layout-space SA chain, whole
//!     chain inside wasm (warm steps start from the parent layout's best
//!     cards); returns the best-scored visited layouts.
//!
//! Layouts cross the boundary as lists of bitboard cell indices
//! (`row * 9 + col`); the base deck as `[row, col, arcane]` triples.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use std::collections::HashMap;

use crate::construction::{
    bits, cell_index, cell_rc, classes, is_reachable, outer_chain, roll_prob,
};
use crate::tagsim::{run_sa_from, run_sa_tagged as run_core, SlotCard};
use crate::tagsim_wasm::{build_run, card, placed_out, PlacedIn, PlacedOut, ScheduleIn, TagRunInput};

#[derive(Deserialize)]
struct BaseIn {
    base: Vec<(i32, i32, bool)>,
}

struct Base {
    cells: Vec<(i32, i32, bool)>,
    mask: u64,
    arcane: u64,
}

fn parse_base(b: &[(i32, i32, bool)]) -> Result<Base, JsValue> {
    let (mut mask, mut arcane) = (0u64, 0u64);
    for &(r, c, a) in b {
        let i = cell_index(r, c).ok_or_else(|| JsValue::from_str(
            &format!("deck slot ({r},{c}) lies outside the 9×6 construction box")))?;
        mask |= 1u64 << i;
        if a { arcane |= 1u64 << i; }
    }
    Ok(Base { cells: b.to_vec(), mask, arcane })
}

fn set_from(cells: &[u32]) -> u64 { cells.iter().fold(0u64, |m, &i| m | (1u64 << i)) }

fn err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> JsValue + '_ {
    move |e| JsValue::from_str(&format!("{what}: {e}"))
}

// ── enumerate ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct EnumIn {
    #[serde(flatten)]
    base: BaseIn,
    ks: Vec<usize>,
    k_probs: Vec<f64>,
    allow_x: bool,
    allow_y: bool,
    #[serde(default)]
    user_added: Vec<(i32, i32)>,
    /// false = locate the player's layout only (skip the class list).
    #[serde(default = "default_true")]
    classes_out: bool,
}

fn default_true() -> bool { true }

#[derive(Serialize)]
struct ClassOut { k: usize, rep: Vec<u32>, members: u32, prob: f64 }

#[derive(Serialize)]
struct EnumOut {
    classes: Vec<ClassOut>,
    set_counts: Vec<usize>,
    user_legal: bool,
    /// P(k) × P(exact set | k) for the player's layout.
    user_prob: f64,
    user_class: Option<usize>,
}

#[wasm_bindgen(js_name = constructionEnumerate)]
pub fn construction_enumerate(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: EnumIn = serde_wasm_bindgen::from_value(input).map_err(err("enumerate input"))?;
    let base = parse_base(&inp.base.base)?;
    let (cls, counts) = classes(base.mask, base.arcane, &inp.ks, &inp.k_probs, inp.allow_x, inp.allow_y);

    let mut user = 0u64;
    let mut user_ok = !inp.user_added.is_empty();
    for &(r, c) in &inp.user_added {
        match cell_index(r, c) { Some(i) => user |= 1u64 << i, None => user_ok = false }
    }
    user_ok = user_ok && is_reachable(base.mask, user);
    let uk = user.count_ones() as usize;
    let kp = inp.ks.iter().position(|&k| k == uk).map(|i| inp.k_probs[i]);
    user_ok = user_ok && kp.is_some();
    let user_prob = if user_ok { kp.unwrap_or(0.0) * roll_prob(base.mask, user) } else { 0.0 };
    let user_class = if user_ok {
        let key = crate::construction::canon(base.mask, base.arcane, user, inp.allow_x, inp.allow_y);
        cls.iter().position(|c| c.k == uk
            && crate::construction::canon(base.mask, base.arcane, c.rep, inp.allow_x, inp.allow_y) == key)
    } else { None };

    let out = EnumOut {
        classes: if !inp.classes_out { Vec::new() } else { cls.iter().map(|c| ClassOut { k: c.k, rep: bits(c.rep), members: c.members, prob: c.prob }).collect() },
        set_counts: counts,
        user_legal: user_ok,
        user_prob,
        user_class,
    };
    serde_wasm_bindgen::to_value(&out).map_err(err("enumerate output"))
}

// ── scoring ──────────────────────────────────────────────────────────────────

/// Full-layout run input: base slots first (given order), added cells after
/// in ascending index order; peers rebuilt with the kernel's definitions.
fn layout_input(tpl: &TagRunInput, base: &Base, set: u64, n_iter: usize, restarts: usize, seed: u64) -> TagRunInput {
    let mut cells: Vec<(i32, i32, bool)> = base.cells.clone();
    for i in bits(set) { let (r, c) = cell_rc(i); cells.push((r, c, false)); }
    let n = cells.len();
    let (mut row, mut col, mut surr, mut diag) = (vec![vec![]; n], vec![vec![]; n], vec![vec![]; n], vec![vec![]; n]);
    for i in 0..n {
        let (r, c, _) = cells[i];
        for j in 0..n {
            if i == j { continue; }
            let (qr, qc, _) = cells[j];
            if qr == r { row[i].push(j); }
            if qc == c { col[i].push(j); }
            if (qr - r).abs() <= 1 && (qc - c).abs() <= 1 { surr[i].push(j); }
            if qr - qc == r - c || qr + qc == r + c { diag[i].push(j); }
        }
    }
    let mut inp = tpl.clone();
    inp.slots = cells.iter().map(|&(r, c, _)| (r, c)).collect();
    inp.row_peers = row;
    inp.col_peers = col;
    inp.surr_peers = surr;
    inp.diag_peers = diag;
    inp.arcane_slot_indices = cells.iter().enumerate().filter(|x| x.1 .2).map(|x| x.0).collect();
    inp.n_iter = n_iter;
    inp.restarts = restarts;
    inp.seed = Some(seed);
    inp
}

/// Names in the template are validated once (check_template); each layout only changes the geometry.
fn check_template(tpl: &TagRunInput) -> Result<(), JsValue> {
    build_run(tpl).map(|_| ()).map_err(|e| JsValue::from_str(&format!("tagsim: {e}")))
}

/// Re-anneal budget for walks that start from a known layout (`schedule` replaces the template's, no phase 2).
#[derive(Deserialize, Clone)]
struct WarmIn {
    n_iter: usize,
    schedule: ScheduleIn,
}

/// Score one layout: cold restarts under the template, or, given `start`, `restarts` warm walks from it.
fn score_layout(tpl: &TagRunInput, base: &Base, set: u64, n_iter: usize, restarts: usize, seed: u64,
                start: Option<(&[SlotCard], &WarmIn)>) -> (Vec<SlotCard>, f64) {
    match start {
        None => {
            let inp = layout_input(tpl, base, set, n_iter, restarts, seed);
            run_core(build_run(&inp).expect("template validated by check_template"))
        }
        Some((s, w)) => {
            let mut inp = layout_input(tpl, base, set, w.n_iter, 1, seed);
            inp.schedule = Some(w.schedule.clone());
            inp.phase2 = None;
            let starts: Vec<Vec<SlotCard>> = (0..restarts.max(1)).map(|_| s.to_vec()).collect();
            run_sa_from(build_run(&inp).expect("template validated by check_template"), &starts)
        }
    }
}

fn parse_layout(cards: &[PlacedIn], n: usize, n_stacks: usize) -> Result<Vec<SlotCard>, JsValue> {
    if cards.len() != n {
        return Err(JsValue::from_str(&format!("start layout has {} cards for {n} slots", cards.len())));
    }
    cards.iter().map(|(t, c, sc, gs, stack)| {
        let k = card(t, c, sc, gs, *stack).map_err(|e| JsValue::from_str(&format!("start card: {e}")))?;
        if k.stack != u16::MAX && k.stack as usize >= n_stacks {
            return Err(JsValue::from_str(&format!("start card stack {} out of range", k.stack)));
        }
        Ok(k)
    }).collect()
}

#[derive(Deserialize)]
struct EvalIn {
    template: TagRunInput,
    #[serde(flatten)]
    base: BaseIn,
    sets: Vec<Vec<u32>>,
    n_iter: usize,
    restarts: usize,
    seed: u64,
    /// Per set (parallel to `sets`, may be shorter): a layout to warm-start from, re-annealed with `warm`.
    #[serde(default)]
    starts: Vec<Option<Vec<PlacedIn>>>,
    #[serde(default)]
    warm: Option<WarmIn>,
    /// Return the best layout of every set scoring at least this much (None = scores only).
    #[serde(default)]
    keep_above: Option<f64>,
    /// Sets without a given start re-anneal (with `warm`) from the previous set's best layout remapped onto
    /// their cells; the job's first set is scored cold. Callers order `sets` so neighbours share most cells.
    #[serde(default)]
    chain_warm: bool,
}

#[derive(Serialize)]
struct EvalOut {
    scores: Vec<f64>,
    layouts: Vec<Option<Vec<PlacedOut>>>,
}

#[wasm_bindgen(js_name = constructionEval)]
pub fn construction_eval(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: EvalIn = serde_wasm_bindgen::from_value(input).map_err(err("eval input"))?;
    check_template(&inp.template)?;
    if (inp.chain_warm || inp.starts.iter().any(|s| s.is_some())) && inp.warm.is_none() {
        return Err(JsValue::from_str("eval: warm starts given without a warm budget"));
    }
    let nb_cells = parse_base(&inp.base.base)?.cells.len();
    let mut prev: Option<(u64, Vec<SlotCard>)> = None;
    let base = parse_base(&inp.base.base)?;
    let n_stacks = inp.template.stacks.len();
    let mut scores = Vec::with_capacity(inp.sets.len());
    let mut layouts = Vec::with_capacity(inp.sets.len());
    for (i, s) in inp.sets.iter().enumerate() {
        let seed = inp.seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let set = set_from(s);
        let start = match inp.starts.get(i).and_then(|x| x.as_ref()) {
            Some(cards) => Some(parse_layout(cards, base.cells.len() + s.len(), n_stacks)?),
            None if inp.chain_warm => prev.as_ref().and_then(|(ps, pl)| remap(nb_cells, *ps, pl, set)),
            None => None,
        };
        let warm = inp.warm.as_ref();
        let (asgn, v) = score_layout(&inp.template, &base, set, inp.n_iter, inp.restarts, seed,
                                     start.as_deref().zip(warm));
        if inp.chain_warm { prev = Some((set, asgn.clone())); }
        scores.push(v);
        layouts.push(match inp.keep_above { Some(t) if v >= t => Some(placed_out(&asgn)), _ => None });
    }
    serde_wasm_bindgen::to_value(&EvalOut { scores, layouts }).map_err(err("eval output"))
}

// ── outer chain ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ChainIn {
    template: TagRunInput,
    #[serde(flatten)]
    base: BaseIn,
    k: usize,
    steps: usize,
    n_iter: usize,
    seed: u64,
    #[serde(default)]
    start: Option<Vec<u32>>,
    #[serde(default = "default_keep")]
    keep: usize,
    /// Warm steps: a moved-to layout re-anneals from its parent's best layout (the vacated cell's card moves to
    /// the new cell) with this budget; the chain's first layout is scored cold with `n_iter`.
    #[serde(default)]
    warm: Option<WarmIn>,
}

fn default_keep() -> usize { 24 }

#[derive(Serialize)]
struct Visited { set: Vec<u32>, score: f64 }

/// A layout of set `from` carried onto set `to` (same size): base slots unchanged, shared added cells keep
/// their cards, and the cards of vacated cells fill the new cells in ascending order (stack counts stay the
/// same). None when the sizes differ.
fn remap(nb: usize, from: u64, layout: &[SlotCard], to: u64) -> Option<Vec<SlotCard>> {
    if from.count_ones() != to.count_ones() || layout.len() != nb + from.count_ones() as usize { return None; }
    let fcells = bits(from);
    let at = |cell: u32| layout[nb + fcells.iter().position(|&c| c == cell).expect("cell in from")];
    let mut spare = bits(from & !to).into_iter().map(at);
    let mut v: Vec<SlotCard> = layout[..nb].to_vec();
    for c in bits(to) {
        v.push(if from >> c & 1 == 1 { at(c) } else { spare.next().expect("equal sizes") });
    }
    Some(v)
}

#[wasm_bindgen(js_name = constructionChain)]
pub fn construction_chain(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: ChainIn = serde_wasm_bindgen::from_value(input).map_err(err("chain input"))?;
    check_template(&inp.template)?;
    let base = parse_base(&inp.base.base)?;
    let nb = base.cells.len();
    let start = inp.start.as_ref().map(|s| set_from(s));
    let mut best: HashMap<u64, (f64, Vec<SlotCard>)> = HashMap::new();
    let visited = outer_chain(base.mask, inp.k, inp.steps, inp.seed, start, |set, from, salt| {
        let warm_start = match (&inp.warm, from) {
            (Some(w), Some((parent, _, _))) => best.get(&parent)
                .and_then(|(_, l)| remap(nb, parent, l, set)).map(|l| (l, w)),
            _ => None,
        };
        let (asgn, v) = match &warm_start {
            Some((l, w)) => score_layout(&inp.template, &base, set, 0, 1, salt, Some((l.as_slice(), *w))),
            None => score_layout(&inp.template, &base, set, inp.n_iter, 1, salt, None),
        };
        if best.get(&set).map_or(true, |b| v > b.0) { best.insert(set, (v, asgn)); }
        v
    });
    let out: Vec<Visited> = visited.into_iter().take(inp.keep)
        .map(|(s, v)| Visited { set: bits(s), score: v }).collect();
    serde_wasm_bindgen::to_value(&out).map_err(err("chain output"))
}
