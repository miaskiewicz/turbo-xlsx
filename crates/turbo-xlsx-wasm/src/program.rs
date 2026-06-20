//! The write entry points and the streaming [`WorkbookWriter`] handle: this is
//! where the binding wires the core pipeline together exactly as the N-API
//! binding does — every entry mode converges on `turbo-xlsx-core`'s typed model
//! and returns `{ xlsx: Uint8Array, diagnostics }`.

use serde::Deserialize;
use wasm_bindgen::prelude::*;

use turbo_xlsx_core as core;

use crate::convert::{finish, parse_options, JsError};

/// One-shot: a complete workbook object → `{ xlsx: Uint8Array, diagnostics }`.
/// `workbook` is the documented workbook shape; `opts` may be null/undefined.
#[wasm_bindgen]
pub fn write(workbook: JsValue, opts: JsValue) -> Result<JsValue, JsValue> {
    let value: serde_json::Value = serde_wasm_bindgen::from_value(workbook).map_err(schema_err)?;
    let options = parse_options(opts)?.into_core();
    finish(core::write_from_json_value(value, &options))
}

/// JSON in: a JSON string OR a JSON value matching the workbook schema. A string
/// routes through `write_from_json_str`; any other value is validated as a value.
#[wasm_bindgen(js_name = writeFromJson)]
pub fn write_from_json(input: JsValue, opts: JsValue) -> Result<JsValue, JsValue> {
    let options = parse_options(opts)?.into_core();
    if let Some(text) = input.as_string() {
        return finish(core::write_from_json_str(&text, &options));
    }
    let value: serde_json::Value = serde_wasm_bindgen::from_value(input).map_err(schema_err)?;
    finish(core::write_from_json_value(value, &options))
}

/// Convenience fast-path: one sheet from typed columns + rows. NOT a CSV ingester.
/// `input` is `{ sheetName?, locale?, columns?, rows }`.
#[wasm_bindgen(js_name = writeRows)]
pub fn write_rows(input: JsValue, opts: JsValue) -> Result<JsValue, JsValue> {
    let parsed: RowsInput = serde_wasm_bindgen::from_value(input).map_err(schema_err)?;
    let options = parse_options(opts)?.into_core();
    finish(core::write_rows(
        parsed.sheet_name,
        parsed.columns,
        parsed.rows,
        parsed.locale,
        &options,
    ))
}

/// The shape of the rows fast-path input.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RowsInput {
    sheet_name: Option<String>,
    locale: Option<String>,
    #[serde(default)]
    columns: Vec<core::Column>,
    #[serde(default)]
    rows: Vec<core::Row>,
}

/// Create a streaming writer for large sheets. `opts` may be null/undefined.
#[wasm_bindgen(js_name = createWriter)]
pub fn create_writer(opts: JsValue) -> Result<WorkbookWriter, JsValue> {
    WorkbookWriter::create(opts)
}

/// A row-by-row streaming writer. Push a sheet, stream rows, end the sheet,
/// finish the package. Wraps the core [`core::WorkbookWriter`]; `finish` consumes
/// the inner writer via `.take()`, so any call afterwards rejects.
#[wasm_bindgen]
pub struct WorkbookWriter {
    inner: Option<core::WorkbookWriter>,
}

#[wasm_bindgen]
impl WorkbookWriter {
    /// Build the writer from its options (locale + metadata).
    fn create(opts: JsValue) -> Result<WorkbookWriter, JsValue> {
        let parsed = parse_options(opts)?;
        let locale = parsed.locale.clone();
        let options = parsed.into_core();
        Ok(WorkbookWriter {
            inner: Some(core::WorkbookWriter::new(locale, options)),
        })
    }

    /// Begin a new sheet from its metadata (its `rows` are ignored — stream them).
    #[wasm_bindgen(js_name = startSheet)]
    pub fn start_sheet(&mut self, sheet: JsValue) -> Result<(), JsValue> {
        let meta: core::Sheet = serde_wasm_bindgen::from_value(sheet).map_err(schema_err)?;
        self.writer()?.start_sheet(meta).map_err(fatal_err)
    }

    /// Stream one row into the open sheet.
    #[wasm_bindgen(js_name = writeRow)]
    pub fn write_row(&mut self, row: JsValue) -> Result<(), JsValue> {
        let row: core::Row = serde_wasm_bindgen::from_value(row).map_err(schema_err)?;
        self.writer()?.write_row(&row).map_err(fatal_err)
    }

    /// Stream a chunk of rows from a JSON array string (`JSON.stringify(rows)`).
    /// The throughput path: the chunk is stringified in V8 (native) and parsed
    /// once in Rust, skipping the per-cell `serde_wasm_bindgen` object walk.
    #[wasm_bindgen(js_name = writeRowsJson)]
    pub fn write_rows_json(&mut self, rows_json: &str) -> Result<(), JsValue> {
        let rows: Vec<core::Row> = serde_json::from_str(rows_json)
            .map_err(|e| JsError::schema(e.to_string()).into_jsvalue())?;
        let writer = self.writer()?;
        for row in &rows {
            writer.write_row(row).map_err(fatal_err)?;
        }
        Ok(())
    }

    /// Close the open sheet (idempotent).
    #[wasm_bindgen(js_name = endSheet)]
    pub fn end_sheet(&mut self) -> Result<(), JsValue> {
        self.writer()?.end_sheet().map_err(fatal_err)
    }

    /// Finish every sheet and ZIP the package. The writer is consumed; calling
    /// any method afterwards rejects.
    pub fn finish(&mut self) -> Result<JsValue, JsValue> {
        let writer = self.inner.take().ok_or_else(finished_err)?;
        finish(writer.finish())
    }

    /// Borrow the live writer, erroring if it was already finished.
    fn writer(&mut self) -> Result<&mut core::WorkbookWriter, JsValue> {
        self.inner.as_mut().ok_or_else(finished_err)
    }
}

/// A `SchemaViolation` rejection for input that failed to deserialize.
fn schema_err(e: serde_wasm_bindgen::Error) -> JsValue {
    JsError::schema(e.to_string()).into_jsvalue()
}

/// A structured rejection for a fatal core write fault.
fn fatal_err(e: core::TurboXlsxError) -> JsValue {
    JsError::from(e).into_jsvalue()
}

/// A `SchemaViolation` rejection for using a writer after `finish`.
fn finished_err() -> JsValue {
    JsError::schema("writer already finished").into_jsvalue()
}
