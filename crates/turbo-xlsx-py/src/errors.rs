//! Error marshaling across the PyO3 boundary.
//!
//! A core fatal [`TurboXlsxError`] carries a stable machine-readable
//! [`ErrorCode`] and a message. It surfaces to Python as a typed
//! `TurboXlsxError` exception whose `.code` (the stable variant string) and
//! `.message` mirror the core payload. There is no source span — the writer
//! validates a structured model, not parsed text. Non-fatal lints are
//! *returned* in the result, never raised.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

use turbo_xlsx_core::ErrorCode;

create_exception!(
    turbo_xlsx,
    TurboXlsxError,
    PyException,
    "Fatal validate/write fault. Carries `.code` (stable string) and `.message`."
);

/// A `SchemaViolation` error for input that failed to deserialize.
pub fn schema(message: impl Into<String>) -> PyErr {
    from_core(turbo_xlsx_core::TurboXlsxError::new(
        ErrorCode::SchemaViolation,
        message.into(),
    ))
}

/// Map a core fatal error to a typed `TurboXlsxError` with `.code`/`.message`.
pub fn from_core(e: turbo_xlsx_core::TurboXlsxError) -> PyErr {
    Python::with_gil(|py| build(py, e.code, &e.message))
}

/// Construct a `TurboXlsxError` instance, falling back to the bare exception if
/// attribute population ever fails (e.g. during interpreter shutdown).
fn build(py: Python<'_>, code: ErrorCode, message: &str) -> PyErr {
    match build_inner(py, code, message) {
        Ok(err) => err,
        Err(e) => e,
    }
}

/// The fallible body of [`build`]: instantiate the exception and attach fields.
fn build_inner(py: Python<'_>, code: ErrorCode, message: &str) -> PyResult<PyErr> {
    let err = TurboXlsxError::new_err(message.to_string());
    let value = err.value(py);
    value.setattr("code", code.as_str())?;
    value.setattr("message", message)?;
    Ok(err)
}
