//! ndm_core — Rust SA kernel for the Wold's Vaults deck optimizer.
//!
//! Dual-target crate:
//!   * `--features python` (default): PyO3 extension module loaded by the
//!     CLI (`uv run --extra rust optimize` / the inventory GUI's Rust path).
//!   * `--features wasm`            : wasm-bindgen module loaded by the
//!     browser SPA (only ships the inventory optimizer).
//!
//! The two feature sets are mutually exclusive at the *entry-point* level
//! (different exported symbols), but share the same pure-Rust kernel in
//! `inventory.rs`. The legacy batch optimizer (`batch.rs`) is python-only.

pub mod inventory;

#[cfg(feature = "python")]
mod batch;

#[cfg(feature = "wasm")]
pub mod wasm_api;

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
#[pymodule]
fn ndm_core(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(batch::run_sa_optimize, m)?)?;
    m.add_function(wrap_pyfunction!(inventory::run_sa_inventory, m)?)?;
    Ok(())
}
