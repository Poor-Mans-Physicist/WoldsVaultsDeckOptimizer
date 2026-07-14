//! ndm_core — Rust SA kernel for the WASM web app.
//!
//! Built only for `wasm-bindgen` (browser SPA). The PyO3 / native CLI path
//! lives in the OUTER `ndm_core/` crate; this one is web-only since the
//! refactor that nuked the in-tree CLI duplicate.
//!
//! Pure-Rust kernel lives in `inventory.rs`; `wasm_api.rs` is the
//! wasm-bindgen entry layer that the SPA calls.

pub mod inventory;
pub mod wasm_api;
// Optimizer 2.0 tag-aware kernel — canonical source lives in the OUTER
// crate (`ndm_core/src/tagsim.rs`); included here verbatim so the scoring
// math can never drift between the CLI and the web app.
#[path = "../../../ndm_core/src/tagsim.rs"]
pub mod tagsim;
pub mod tagsim_wasm;
