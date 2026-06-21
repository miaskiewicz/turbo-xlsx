//! Boundary conversions between JS values and the core's Rust types: write
//! options, the diagnostics array returned in every result, and the structured
//! error thrown on a fatal fault.
//!
//! Everything here is plain `serde` data shuttled across `serde-wasm-bindgen`,
//! plus the `.xlsx` bytes which cross as a real `js_sys::Uint8Array`. Each helper
//! is kept small so the binding stays under the project's cyclomatic-complexity
//! gate.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use turbo_xlsx_core as core;

/// JS document metadata `{ title?, author?, subject?, company? }`. Every field
/// optional; an omitted object leaves the core defaults in place.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsDocMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub company: Option<String>,
}

/// JS write options `{ meta?, locale?, password? }`. `locale` seeds the streaming
/// writer (the batch path reads the workbook's own `locale`); `password` triggers
/// ECMA-376 Agile Encryption of the output.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JsWriteOptions {
    pub meta: Option<JsDocMeta>,
    pub locale: Option<String>,
    pub password: Option<String>,
}

impl JsWriteOptions {
    /// Lower into the core [`core::WriteOptions`].
    pub fn into_core(self) -> core::WriteOptions {
        let meta = self.meta.unwrap_or_default();
        core::WriteOptions {
            meta: core::DocMeta {
                title: meta.title,
                author: meta.author,
                subject: meta.subject,
                company: meta.company,
            },
            password: self.password,
        }
    }
}

/// Deserialize the options argument, treating `undefined`/`null` as defaults.
pub fn parse_options(opts: JsValue) -> Result<JsWriteOptions, JsValue> {
    if opts.is_undefined() || opts.is_null() {
        return Ok(JsWriteOptions::default());
    }
    serde_wasm_bindgen::from_value(opts).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// The JS shape of one diagnostic/lint: `{ code, message }`.
#[derive(Debug, Serialize)]
pub struct JsDiagnostic {
    pub code: String,
    pub message: String,
}

impl From<&core::Lint> for JsDiagnostic {
    fn from(l: &core::Lint) -> Self {
        JsDiagnostic {
            code: l.code.as_str().to_string(),
            message: l.message.clone(),
        }
    }
}

/// The JS shape of a fatal error, returned on the `Err` path: `{ code, message }`
/// (the core error's span, if any, is dropped at the boundary).
#[derive(Debug, Serialize)]
pub struct JsError {
    pub code: String,
    pub message: String,
}

impl From<core::TurboXlsxError> for JsError {
    fn from(e: core::TurboXlsxError) -> Self {
        JsError {
            code: e.code.as_str().to_string(),
            message: e.message,
        }
    }
}

impl JsError {
    /// A `SchemaViolation` error for input that failed to deserialize.
    pub fn schema(message: impl Into<String>) -> JsError {
        JsError::from(core::TurboXlsxError::new(
            core::ErrorCode::SchemaViolation,
            message.into(),
        ))
    }

    /// Serialize into a `JsValue` suitable for `Err(..)`, so the JS caller sees a
    /// structured `{ code, message }` object on the rejection path.
    pub fn into_jsvalue(self) -> JsValue {
        serde_wasm_bindgen::to_value(&self)
            .unwrap_or_else(|_| JsValue::from_str("turbo-xlsx: error serialization failed"))
    }
}

/// Build the JS result object `{ xlsx: Uint8Array, diagnostics: [{code,message}] }`
/// from a core write result. `xlsx` is a REAL `Uint8Array` (not a plain array),
/// and the diagnostics are the returned lints.
pub fn result_to_js(result: core::WriteResult) -> Result<JsValue, JsValue> {
    let obj = js_sys::Object::new();
    set_prop(&obj, "xlsx", &js_sys::Uint8Array::from(&result.xlsx[..]))?;
    set_prop(
        &obj,
        "diagnostics",
        &diagnostics_to_js(&result.diagnostics)?,
    )?;
    Ok(obj.into())
}

/// Lower collected diagnostics into the JS array returned in the result.
fn diagnostics_to_js(diags: &core::Diagnostics) -> Result<JsValue, JsValue> {
    let lints: Vec<JsDiagnostic> = diags.lints.iter().map(JsDiagnostic::from).collect();
    serde_wasm_bindgen::to_value(&lints).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Set one own-property on `obj`, mapping the (infallible-in-practice) reflect
/// failure to a structured boundary error.
fn set_prop(obj: &js_sys::Object, key: &str, value: &impl AsRef<JsValue>) -> Result<(), JsValue> {
    js_sys::Reflect::set(obj, &JsValue::from_str(key), value.as_ref())?;
    Ok(())
}

/// Map a core write `Result` into the JS result object, turning a fatal error
/// into a structured `{ code, message }` rejection.
pub fn finish(result: core::Result<core::WriteResult>) -> Result<JsValue, JsValue> {
    let r = result.map_err(|e| JsError::from(e).into_jsvalue())?;
    result_to_js(r)
}
