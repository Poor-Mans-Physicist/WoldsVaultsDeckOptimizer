//! wasm-bindgen boundary for the structural (Construction Core) optimizer.
//!
//!   * `constructionEnumerate` — every reachable layout for the roll's k
//!     range, grouped into score-equivalent reflection classes, with exact
//!     per-roll probabilities (plus the player's own layout's).
//!   * `constructionEval`      — score a batch of layouts with the tagged SA
//!     (cold restarts) under one fixed run template.
//!   * `constructionChain`     — one outer layout-space SA chain, whole
//!     chain inside wasm; returns the best-scored visited layouts.
//!
//! Layouts cross the boundary as lists of bitboard cell indices
//! (`row * 9 + col`); the base deck as `[row, col, arcane]` triples.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::construction::{
    bits, cell_index, cell_rc, classes, is_reachable, outer_chain, roll_prob,
};
use crate::tagsim::run_sa_tagged as run_core;
use crate::tagsim_wasm::{build_run, TagRunInput};

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

fn score_layout(tpl: &TagRunInput, base: &Base, set: u64, n_iter: usize, restarts: usize, seed: u64) -> f64 {
    let inp = layout_input(tpl, base, set, n_iter, restarts, seed);
    run_core(build_run(&inp).expect("template validated by check_template")).1
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
}

#[wasm_bindgen(js_name = constructionEval)]
pub fn construction_eval(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: EvalIn = serde_wasm_bindgen::from_value(input).map_err(err("eval input"))?;
    check_template(&inp.template)?;
    let base = parse_base(&inp.base.base)?;
    let scores: Vec<f64> = inp.sets.iter().enumerate().map(|(i, s)| {
        let seed = inp.seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        score_layout(&inp.template, &base, set_from(s), inp.n_iter, inp.restarts, seed)
    }).collect();
    serde_wasm_bindgen::to_value(&scores).map_err(err("eval output"))
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
}

fn default_keep() -> usize { 24 }

#[derive(Serialize)]
struct Visited { set: Vec<u32>, score: f64 }

#[wasm_bindgen(js_name = constructionChain)]
pub fn construction_chain(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: ChainIn = serde_wasm_bindgen::from_value(input).map_err(err("chain input"))?;
    check_template(&inp.template)?;
    let base = parse_base(&inp.base.base)?;
    let start = inp.start.as_ref().map(|s| set_from(s));
    let visited = outer_chain(base.mask, inp.k, inp.steps, inp.seed, start, |set, salt| {
        score_layout(&inp.template, &base, set, inp.n_iter, 1, salt)
    });
    let out: Vec<Visited> = visited.into_iter().take(inp.keep)
        .map(|(s, v)| Visited { set: bits(s), score: v }).collect();
    serde_wasm_bindgen::to_value(&out).map_err(err("chain output"))
}
