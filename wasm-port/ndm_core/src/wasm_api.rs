//! wasm-bindgen entry point for the inventory optimizer.
//!
//! Mirrors the PyO3 entry in `inventory.rs::run_sa_inventory` but takes its
//! arguments as a single serde-deserialized JS object so the browser side
//! doesn't have to wrestle with positional bindings.
//!
//! Every field added after the initial WASM landing is gated by
//! `#[serde(default …)]` so old JS payloads keep working — the new behaviour
//! falls back to "no arcane slots / no forced inventory / auto-place ON /
//! 0× void multipliers", which is exactly the pre-arcane behaviour.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::inventory::{
    card_type_from_str, card_type_to_str, color_from_str, color_to_str,
    core_from_str, run_sa_inventory_core, InventoryRun, SimConfig,
};

fn default_true() -> bool { true }

#[derive(Deserialize)]
pub struct WasmInventoryInput {
    pub slots:      Vec<(i32, i32)>,
    pub row_peers:  Vec<Vec<usize>>,
    pub col_peers:  Vec<Vec<usize>>,
    pub surr_peers: Vec<Vec<usize>>,
    pub diag_peers: Vec<Vec<usize>>,
    /// Indices into `slots` that correspond to arcane (`A`) slots. Empty list
    /// = no arcane slots in this deck (the common case for vanilla decks).
    #[serde(default)]
    pub arcane_slot_indices: Vec<usize>,
    /// True = arcane slots locked to ARCANE (color swaps allowed within them
    /// but no ARCANE↔DEAD swap). False = SA may swap arcane slots to DEAD
    /// for void-core trade-offs. Defaults to true to match historical behaviour.
    #[serde(default = "default_true")]
    pub auto_place_arcane: bool,
    pub is_shiny:   bool,
    /// Each entry: [type_string, color_string, count].
    pub inventory:  Vec<(String, String, u32)>,
    /// Forced inventory — per-(type, color) lower bound. Cap = inventory + forced.
    /// Defaults to empty (no forced placements).
    #[serde(default)]
    pub forced_inventory: Vec<(String, String, u32)>,
    /// Each entry: [core_type_string, color_string_or_empty, override_or_negative].
    pub cores:      Vec<(String, String, f64)>,
    pub n_iter:     usize,
    pub restarts:   usize,

    pub mult_dir_vert:          f64,
    pub mult_dir_horiz:         f64,
    pub mult_evo_greed:         f64,
    pub mult_surr_greed:        f64,
    pub mult_dir_diag_up:       f64,
    pub mult_dir_diag_down:     f64,
    pub mult_pure_base:         f64,
    pub mult_pure_scale:        f64,
    pub mult_equilibrium:       f64,
    pub mult_foil:              f64,
    pub mult_steadfast:         f64,
    pub mult_color:             f64,
    pub mult_deluxe_flat:       f64,
    pub mult_deluxe_core_base:  f64,
    pub mult_deluxe_core_scale: f64,
    /// Void core multipliers. Defaults to 0/0 so a payload that omits them
    /// disables the core entirely (matches the pre-port behaviour).
    #[serde(default)]
    pub mult_void_core_base:    f64,
    #[serde(default)]
    pub mult_void_core_scale:   f64,
    pub greed_additive:         bool,
    pub additive_cores:         bool,
}

#[derive(Serialize)]
pub struct WasmInventoryResult {
    /// Per-slot placed card, parallel to `slots`. Each entry: [type, color].
    pub assignment: Vec<(String, String)>,
    pub score:      f64,
}

/// Entry point exposed to JavaScript. Accepts a single JS object that
/// deserializes into `WasmInventoryInput`; returns `WasmInventoryResult`.
#[wasm_bindgen(js_name = runSaInventory)]
pub fn run_sa_inventory_wasm(input: JsValue) -> Result<JsValue, JsValue> {
    let inp: WasmInventoryInput = serde_wasm_bindgen::from_value(input)
        .map_err(|e| JsValue::from_str(&format!("input deserialize failed: {e}")))?;

    let inventory_u8: Vec<(u8, u8, u32)> = inp.inventory.iter()
        .map(|(t, c, n)| (card_type_from_str(t), color_from_str(c), *n))
        .collect();
    let forced_u8: Vec<(u8, u8, u32)> = inp.forced_inventory.iter()
        .map(|(t, c, n)| (card_type_from_str(t), color_from_str(c), *n))
        .collect();
    let cores_u8: Vec<(u8, u8, f64)> = inp.cores.iter()
        .map(|(t, c, o)| (core_from_str(t), color_from_str(c), *o))
        .collect();

    let cfg_mults = SimConfig {
        mult_dir_vert:          inp.mult_dir_vert,
        mult_dir_horiz:         inp.mult_dir_horiz,
        mult_evo_greed:         inp.mult_evo_greed,
        mult_surr_greed:        inp.mult_surr_greed,
        mult_dir_diag_up:       inp.mult_dir_diag_up,
        mult_dir_diag_down:     inp.mult_dir_diag_down,
        mult_pure_base:         inp.mult_pure_base,
        mult_pure_scale:        inp.mult_pure_scale,
        mult_equilibrium:       inp.mult_equilibrium,
        mult_foil:              inp.mult_foil,
        mult_steadfast:         inp.mult_steadfast,
        mult_color:             inp.mult_color,
        mult_deluxe_flat:       inp.mult_deluxe_flat,
        mult_deluxe_core_base:  inp.mult_deluxe_core_base,
        mult_deluxe_core_scale: inp.mult_deluxe_core_scale,
        mult_void_core_base:    inp.mult_void_core_base,
        mult_void_core_scale:   inp.mult_void_core_scale,
        greed_additive:         inp.greed_additive,
        additive_cores:         inp.additive_cores,
        is_shiny:               inp.is_shiny,
        auto_place_arcane:      inp.auto_place_arcane,
    };

    let run = InventoryRun {
        slots:               &inp.slots,
        row_peers:           inp.row_peers,
        col_peers:           inp.col_peers,
        surr_peers:          inp.surr_peers,
        diag_peers:          inp.diag_peers,
        arcane_slot_indices: inp.arcane_slot_indices,
        auto_place_arcane:   inp.auto_place_arcane,
        is_shiny:            inp.is_shiny,
        inventory:           inventory_u8,
        forced_inventory:    forced_u8,
        cores:               cores_u8,
        n_iter:              inp.n_iter,
        restarts:            inp.restarts,
        cfg_mults,
    };

    let (best_asgn, best_score) = run_sa_inventory_core(run);

    let assignment: Vec<(String, String)> = best_asgn.iter()
        .map(|&(t, c)| (card_type_to_str(t).to_owned(), color_to_str(c).to_owned()))
        .collect();

    let out = WasmInventoryResult { assignment, score: best_score };
    serde_wasm_bindgen::to_value(&out)
        .map_err(|e| JsValue::from_str(&format!("output serialize failed: {e}")))
}
