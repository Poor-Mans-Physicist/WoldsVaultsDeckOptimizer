//! Construction Core layout space (web structural optimizer).
//!
//! Game rules (woldsvaults `CreateSlotDeckModifier`, addon ≥ 0.33.2): each
//! added slot is a uniform pick among the cells ORTHOGONALLY adjacent to any
//! current slot (stock, arcane, or previously added) that are not slots and
//! lie in the absolute box x ∈ [0, 8], y ∈ [0, 5]. Picks are sequential, so a
//! later pick may chain off an earlier one. Every stock deck starts at x = 0,
//! y = 0, which is why nothing is ever added left of / above a deck.
//!
//! Cells are bitboard indices `row * 9 + col` (row = game y, col = game x) in
//! a u64 — the whole 9 × 6 box is 54 bits.
//!
//! This module is pure Rust (no wasm types): enumeration of every reachable
//! k-set (Redelmeier growth from the stock super-root), exact per-roll
//! probabilities, score-preserving reflection classes, and the outer
//! layout-space SA. Scoring goes through the shared tagsim kernel.

use rand::prelude::*;
use rand::rngs::SmallRng;
use std::collections::HashMap;

pub const GRID_W: i32 = 9;
pub const GRID_H: i32 = 6;
const ALL: u64 = (1u64 << 54) - 1;
const COL0: u64 = {
    let mut m = 0u64;
    let mut r = 0;
    while r < 6 { m |= 1u64 << (r * 9); r += 1; }
    m
};
const COL8: u64 = COL0 << 8;

pub fn cell_index(r: i32, c: i32) -> Option<u32> {
    if r < 0 || r >= GRID_H || c < 0 || c >= GRID_W { None } else { Some((r * GRID_W + c) as u32) }
}

pub fn cell_rc(i: u32) -> (i32, i32) { ((i / 9) as i32, (i % 9) as i32) }

fn nbr(m: u64) -> u64 {
    (((m & !COL0) >> 1) | ((m & !COL8) << 1) | (m >> 9) | (m << 9)) & ALL
}

/// Cells a construction pick may land on given the current occupancy.
pub fn frontier(occ: u64) -> u64 { nbr(occ) & !occ & ALL }

pub fn bits(m: u64) -> Vec<u32> {
    let mut v = Vec::with_capacity(m.count_ones() as usize);
    let mut x = m;
    while x != 0 { v.push(x.trailing_zeros()); x &= x - 1; }
    v
}

/// Every added cell reachable from the stock slots through added cells.
pub fn is_reachable(base: u64, set: u64) -> bool {
    if set & base != 0 || set & !ALL != 0 { return false; }
    let mut reach = base;
    loop {
        let next = reach | (nbr(reach) & set);
        if next == reach { break; }
        reach = next;
    }
    set & !reach == 0
}

/// Every size-k set the game's sequential growth can produce, each once.
pub fn enumerate(base: u64, k: usize) -> Vec<u64> {
    fn rec(mut untried: Vec<u32>, cur: u64, depth: usize, k: usize, seen: u64, out: &mut Vec<u64>) {
        if depth == k { out.push(cur); return; }
        while let Some(c) = untried.pop() {
            let mut next = untried.clone();
            let fresh = nbr(1u64 << c) & !seen & ALL;
            next.extend(bits(fresh));
            rec(next, cur | (1u64 << c), depth + 1, k, seen | fresh, out);
        }
    }
    if k == 0 { return vec![0]; }
    let f0 = frontier(base);
    let mut out = Vec::new();
    rec(bits(f0), 0, 0, k, base | f0, &mut out);
    out
}

/// Exact probability that one roll of |set| picks produces exactly `set`.
pub fn roll_prob(base: u64, set: u64) -> f64 {
    let cells = bits(set);
    let k = cells.len();
    let mut p = vec![0.0f64; 1 << k];
    p[0] = 1.0;
    for mask in 0..(1usize << k) {
        if p[mask] == 0.0 { continue; }
        let mut occ = base;
        for (i, &c) in cells.iter().enumerate() { if mask >> i & 1 == 1 { occ |= 1u64 << c; } }
        let fr = frontier(occ);
        let w = p[mask] / fr.count_ones() as f64;
        for (i, &c) in cells.iter().enumerate() {
            if mask >> i & 1 == 0 && fr >> c & 1 == 1 { p[mask | 1 << i] += w; }
        }
    }
    p[(1 << k) - 1]
}

/// Draw one roll of k picks exactly as the game does.
pub fn sample_roll(base: u64, k: usize, rng: &mut SmallRng) -> u64 {
    let mut set = 0u64;
    for _ in 0..k {
        let fr = bits(frontier(base | set));
        if fr.is_empty() { break; }
        set |= 1u64 << fr[rng.gen_range(0..fr.len())];
    }
    set
}

/// Canonical form of the full layout (base ∪ set, arcane flags kept) under
/// the allowed reflections, translated to the origin. Layouts with equal
/// keys score identically (reflection-invariant kernel).
pub fn canon(base: u64, arcane: u64, set: u64, allow_x: bool, allow_y: bool) -> (u64, u64) {
    let full = base | set;
    let cells = bits(full);
    let mut best: Option<(u64, u64)> = None;
    for &(fx, fy) in &[(false, false), (true, false), (false, true), (true, true)] {
        if (fx && !allow_x) || (fy && !allow_y) { continue; }
        let pts: Vec<(i32, i32, bool)> = cells.iter().map(|&i| {
            let (r, c) = cell_rc(i);
            (if fy { -r } else { r }, if fx { -c } else { c }, arcane >> i & 1 == 1)
        }).collect();
        let mr = pts.iter().map(|p| p.0).min().unwrap_or(0);
        let mc = pts.iter().map(|p| p.1).min().unwrap_or(0);
        let (mut a, mut b) = (0u64, 0u64);
        for (r, c, arc) in pts {
            let bit = 1u64 << ((r - mr) * GRID_W + (c - mc));
            a |= bit;
            if arc { b |= bit; }
        }
        if best.map_or(true, |x| (a, b) < x) { best = Some((a, b)); }
    }
    best.unwrap_or((0, 0))
}

pub struct LayoutClass {
    pub k: usize,
    pub rep: u64,
    pub members: u32,
    /// P(k) × Σ P(member | k) — chance one roll lands anywhere in the class.
    pub prob: f64,
}

/// Enumerate every k in `ks` (with roll weights `k_probs`) into classes.
pub fn classes(base: u64, arcane: u64, ks: &[usize], k_probs: &[f64], allow_x: bool, allow_y: bool)
    -> (Vec<LayoutClass>, Vec<usize>)
{
    let mut out: Vec<LayoutClass> = Vec::new();
    let mut counts = Vec::new();
    for (ki, &k) in ks.iter().enumerate() {
        let sets = enumerate(base, k);
        counts.push(sets.len());
        let mut idx: HashMap<(u64, u64), usize> = HashMap::new();
        for s in sets {
            let key = canon(base, arcane, s, allow_x, allow_y);
            let p = k_probs[ki] * roll_prob(base, s);
            match idx.get(&key) {
                Some(&ci) => { out[ci].members += 1; out[ci].prob += p; if s < out[ci].rep { out[ci].rep = s; } }
                None => {
                    idx.insert(key, out.len());
                    out.push(LayoutClass { k, rep: s, members: 1, prob: p });
                }
            }
        }
    }
    (out, counts)
}

/// A chain step's origin: the set it moved from, the added cell it vacated
/// and the frontier cell it filled (lets `eval` warm-start from the parent).
pub type Move = (u64, u32, u32);

/// Outer SA over layouts: relocate one added cell to a frontier cell, keep
/// only reachable sets, score each visited set with `eval` (best-seen per
/// set is cached). Returns visited sets with their best score.
pub fn outer_chain(
    base: u64, k: usize, steps: usize, seed: u64, start: Option<u64>,
    mut eval: impl FnMut(u64, Option<Move>, u64) -> f64,
) -> Vec<(u64, f64)> {
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut cache: HashMap<u64, f64> = HashMap::new();
    let mut salt = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut score = |s: u64, from: Option<Move>, cache: &mut HashMap<u64, f64>, salt: u64| -> f64 {
        let v = eval(s, from, salt);
        let e = cache.entry(s).or_insert(f64::MIN);
        if v > *e { *e = v; }
        *e
    };
    let mut cur = start.filter(|&s| is_reachable(base, s) && s.count_ones() as usize == k)
        .unwrap_or_else(|| sample_roll(base, k, &mut rng));
    let mut cs = score(cur, None, &mut cache, salt);
    for step in 0..steps {
        let temp = 0.03 * cs.abs().max(1e-9) * (0.01f64).powf(step as f64 / steps.max(1) as f64);
        let added = bits(cur);
        if added.is_empty() { break; }
        let fr = bits(frontier(base | cur));
        if fr.is_empty() { break; }
        let out = added[rng.gen_range(0..added.len())];
        let inn = fr[rng.gen_range(0..fr.len())];
        let next = (cur & !(1u64 << out)) | (1u64 << inn);
        if !is_reachable(base, next) { continue; }
        salt = salt.wrapping_add(0x6A09_E667_F3BC_C909);
        let ns = score(next, Some((cur, out, inn)), &mut cache, salt);
        if ns >= cs || rng.gen::<f64>() < ((ns - cs) / temp).exp() { cur = next; cs = ns; }
    }
    let mut v: Vec<(u64, f64)> = cache.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> (u64, u64) {
        let (mut b, mut a) = (0u64, 0u64);
        for (r, line) in rows.iter().enumerate() {
            for (c, ch) in line.chars().enumerate() {
                let i = cell_index(r as i32, c as i32).unwrap();
                if ch == 'O' || ch == 'A' { b |= 1u64 << i; }
                if ch == 'A' { a |= 1u64 << i; }
            }
        }
        (b, a)
    }

    const MYSTERY: [&str; 5] = ["OOOOOOO", "XOOXOOX", "OOXAXOO", "XOOXOOX", "OOOOOOO"];

    /// Same counts as the native lab + probabilities form a distribution.
    #[test]
    fn mystery_enumeration_matches_lab() {
        let (b, _) = grid(&MYSTERY);
        let expect = [18usize, 164, 1016, 4834, 18833];
        for k in 1..=5 {
            let sets = enumerate(b, k);
            assert_eq!(sets.len(), expect[k - 1], "k={k}");
            let total: f64 = sets.iter().map(|&s| roll_prob(b, s)).sum();
            assert!((total - 1.0).abs() < 1e-9, "k={k} Σp={total}");
            assert!(sets.iter().all(|&s| is_reachable(b, s)));
        }
    }

    #[test]
    fn never_left_or_above() {
        let (b, _) = grid(&MYSTERY);
        for s in enumerate(b, 5) {
            for i in bits(s) { let (r, c) = cell_rc(i); assert!(r >= 0 && c >= 0 && r <= 5 && c <= 8); }
        }
        let bench = [(0, 7), (0, 8), (4, 7), (4, 8), (5, 2)]
            .iter().fold(0u64, |m, &(r, c)| m | 1u64 << cell_index(r, c).unwrap());
        assert!((roll_prob(b, bench) - 1.065e-5).abs() < 5e-8);
    }

    #[test]
    fn class_counts_match_lab() {
        let (b, a) = grid(&MYSTERY);
        let (cx, _) = classes(b, a, &[5], &[1.0], true, true);
        assert_eq!(cx.len(), 16225);
        let (cx, _) = classes(b, a, &[5], &[1.0], true, false);
        assert_eq!(cx.len(), 17357);
        let total: f64 = cx.iter().map(|c| c.prob).sum();
        assert!((total - 1.0).abs() < 1e-9);
    }
}
