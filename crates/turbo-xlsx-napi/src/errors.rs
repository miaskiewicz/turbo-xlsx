//! Error marshaling across the N-API boundary.
//!
//! A core [`TurboXlsxError`] carries a stable machine-readable `code` and a
//! message. N-API's own error type is only a `(Status, String)` pair, so we
//! encode the structured payload as JSON behind a sentinel prefix in the error
//! `reason`. The thin JS wrapper (`index.js`) detects the prefix and rethrows a
//! typed `TurboXlsxError` whose `.code` mirrors this payload.

use serde_json::Value;
use turbo_xlsx_core::{ErrorCode, TurboXlsxError};

/// Sentinel that marks a `reason` string as a structured turbo-xlsx error.
pub const SENTINEL: &str = "TURBO_XLSX_ERR:";

/// Encode a core error into a sentinel-prefixed N-API error.
pub fn from_core(e: TurboXlsxError) -> napi::Error {
    let body = payload(e.code, &e.message).to_string();
    napi::Error::from_reason(format!("{SENTINEL}{body}"))
}

/// A `SchemaViolation` error for input that failed to deserialize.
pub fn schema(message: impl Into<String>) -> napi::Error {
    from_core(TurboXlsxError::new(
        ErrorCode::SchemaViolation,
        message.into(),
    ))
}

/// A `ParseError`-coded fault (the parse feature has no core `ErrorCode`).
#[cfg(feature = "parse")]
pub fn parse_fault(message: String) -> napi::Error {
    let body = serde_json::json!({ "code": "ParseError", "message": message }).to_string();
    napi::Error::from_reason(format!("{SENTINEL}{body}"))
}

/// Build the JSON payload `{code, message}`.
fn payload(code: ErrorCode, message: &str) -> Value {
    serde_json::json!({ "code": code.as_str(), "message": message })
}
