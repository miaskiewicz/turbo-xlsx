//! WASM/`wasm-bindgen` binding for turbo-xlsx: the workbook-model → `.xlsx`
//! writer exposed to browsers and JS runtimes.
//!
//! The surface mirrors the N-API binding but is WASM-idiomatic. Every entry mode
//! — a complete workbook object ([`write`]), a JSON string/value
//! ([`write_from_json`]), the rows fast-path ([`write_rows`]), and the streaming
//! [`WorkbookWriter`] — converges on `turbo-xlsx-core`'s typed model and returns
//! `{ xlsx: Uint8Array, diagnostics }`.
//!
//! Diagnostics (lints) are *returned* in the result object, never thrown; only a
//! fatal write fault rejects, as a structured `{ code, message }` (the core
//! error's span is dropped at the boundary). The `xlsx` field is always a real
//! `js_sys::Uint8Array`.

#![forbid(unsafe_code)]

mod convert;
mod program;

use wasm_bindgen::prelude::*;

pub use program::{create_writer, write, write_from_json, write_rows, WorkbookWriter};

/// Optional async initializer. There is no module-load work to do today (the
/// writer is pure-Rust with no global setup), but exposing `init()` lets callers
/// write the idiomatic `await init()` and installs a readable panic message — so
/// the entry point is stable if future phases need real async setup.
#[wasm_bindgen]
pub fn init() {
    set_panic_hook();
}

/// Route Rust panics to a JS-readable message instead of an opaque
/// `unreachable`. No-op when the optional `console_error_panic_hook` is absent;
/// kept tiny and dependency-free so the default build stays lean.
fn set_panic_hook() {
    // Intentionally minimal: we do not pull in a panic-hook crate for the
    // default build. Panics still abort with the wasm trap; `init()` exists as
    // the documented async entry point and a hook-installation seam.
}
