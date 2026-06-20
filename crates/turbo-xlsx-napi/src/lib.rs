//! turbo-xlsx N-API binding.
//!
//! Exposes the workbook-model → XLSX writer of `turbo-xlsx-core` to Node/JS. All
//! entry modes converge on the same typed model and return a `Buffer` (the
//! `.xlsx`) plus non-fatal `diagnostics`. Fatal faults are thrown as a typed
//! `TurboXlsxError` (see `errors`); lints are *returned*, never thrown.
//!
//! The product surface is this thin marshaling layer — all writing logic lives in
//! the core crate, which carries the 100% coverage gate. This crate is a cdylib
//! addon tarpaulin cannot line-instrument, so it is excluded from that gate and
//! kept deliberately minimal and mechanical.

#![deny(clippy::all)]

/// Route all allocations in the addon through mimalloc — the write path is
/// dominated by many short-lived String/Vec allocations, which mimalloc services
/// faster than the system allocator. Skipped on musl (see Cargo.toml): a
/// statically-linked mimalloc segfaults when the addon is dlopen'd under musl
/// Node, so there it falls back to the system allocator.
#[cfg(not(target_env = "musl"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod errors;

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use serde::Deserialize;
use serde_json::Value;

use turbo_xlsx_core as core;

/// A non-fatal diagnostic (lint) returned in the result, never thrown.
#[napi(object)]
pub struct JsDiagnostic {
    /// Stable lint code (e.g. `"ClampedColumnWidth"`).
    pub code: String,
    /// Human-readable description.
    pub message: String,
}

/// The result of a write: the `.xlsx` bytes plus the returned lints.
#[napi(object)]
pub struct JsWriteResult {
    /// The OOXML SpreadsheetML (OPC-zipped) document.
    pub xlsx: Buffer,
    /// Non-fatal diagnostics collected during the write.
    pub diagnostics: Vec<JsDiagnostic>,
}

/// Document metadata written to the OPC core/app parts. Every field optional.
#[napi(object)]
#[derive(Default)]
pub struct JsDocMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub company: Option<String>,
}

/// Document-level metadata + global options.
#[napi(object)]
#[derive(Default)]
pub struct JsWriteOptions {
    /// Workbook metadata.
    pub meta: Option<JsDocMeta>,
    /// Default locale for the streaming writer (the batch path reads the
    /// workbook's own `locale`). BCP-47, e.g. `"es-MX"`.
    pub locale: Option<String>,
    /// AES-style password protection — accepted but deferred to v2 (no-op).
    pub password: Option<String>,
}

/// One-shot: a complete workbook object → `.xlsx` bytes.
#[napi]
pub fn write(workbook: Value, opts: Option<JsWriteOptions>) -> napi::Result<JsWriteResult> {
    let options = to_options(&opts);
    finish(core::write_from_json_value(workbook, &options))
}

/// JSON in: a file-less JSON string OR a JSON value matching the workbook schema.
/// Validated fail-closed.
#[napi]
pub fn write_from_json(input: Value, opts: Option<JsWriteOptions>) -> napi::Result<JsWriteResult> {
    let options = to_options(&opts);
    let result = match input {
        Value::String(s) => core::write_from_json_str(&s, &options),
        other => core::write_from_json_value(other, &options),
    };
    finish(result)
}

/// Convenience fast-path: one sheet from typed columns + rows. NOT a CSV ingester.
#[napi]
pub fn write_rows(input: Value, opts: Option<JsWriteOptions>) -> napi::Result<JsWriteResult> {
    let parsed: RowsInput =
        serde_json::from_value(input).map_err(|e| errors::schema(e.to_string()))?;
    let options = to_options(&opts);
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

/// Create a streaming writer for large sheets.
#[napi]
pub fn create_writer(opts: Option<JsWriteOptions>) -> WorkbookWriter {
    WorkbookWriter::create(opts)
}

/// Options for `parse`: `format` is `"json"` (default) / `"csv"` / `"md"`;
/// `typed` selects the round-trippable typed JSON over the values grid; `sheet`
/// picks a sheet by name for csv/md (default first). Only the `turbo-xlsx-parse`
/// build has parse.
#[cfg(feature = "parse")]
#[napi(object)]
#[derive(Default)]
pub struct JsParseOptions {
    pub format: Option<String>,
    pub sheet: Option<String>,
    pub typed: Option<bool>,
}

/// Read an `.xlsx` Buffer into JSON / CSV / Markdown (the `parse` feature).
#[cfg(feature = "parse")]
#[napi]
pub fn parse(data: Buffer, opts: Option<JsParseOptions>) -> napi::Result<String> {
    use core::parse::serialize;
    let wb = core::parse::parse(data.as_ref()).map_err(|e| errors::parse_fault(e.to_string()))?;
    let o = opts.unwrap_or_default();
    match o.format.as_deref().unwrap_or("json") {
        "json" if o.typed == Some(true) => Ok(serialize::to_json_typed(&wb)),
        "json" => Ok(serialize::to_json_grid(&wb)),
        "csv" => Ok(serialize::to_csv(pick_sheet(&wb, o.sheet.as_deref())?)),
        "md" | "markdown" => Ok(serialize::to_markdown(pick_sheet(&wb, o.sheet.as_deref())?)),
        other => Err(errors::schema(format!("unknown parse format {other:?}"))),
    }
}

/// Select a sheet by name (default the first) for the single-sheet formats.
#[cfg(feature = "parse")]
fn pick_sheet<'a>(
    wb: &'a core::parse::ParsedWorkbook,
    name: Option<&str>,
) -> napi::Result<&'a core::parse::ParsedSheet> {
    match name {
        Some(n) => wb
            .sheets
            .iter()
            .find(|s| s.name == n)
            .ok_or_else(|| errors::schema(format!("no sheet named {n:?}"))),
        None => wb
            .sheets
            .first()
            .ok_or_else(|| errors::schema("workbook has no sheets")),
    }
}

/// A row-by-row streaming writer. Push a sheet, stream rows, end the sheet,
/// finish the package. See the core [`core::WorkbookWriter`].
#[napi]
pub struct WorkbookWriter {
    inner: Option<core::WorkbookWriter>,
}

#[napi]
impl WorkbookWriter {
    /// Build the writer from its options (locale + metadata).
    fn create(opts: Option<JsWriteOptions>) -> Self {
        let locale = opts.as_ref().and_then(|o| o.locale.clone());
        let options = to_options(&opts);
        WorkbookWriter {
            inner: Some(core::WorkbookWriter::new(locale, options)),
        }
    }

    /// Begin a new sheet from its metadata (its `rows` are ignored — stream them).
    #[napi]
    pub fn start_sheet(&mut self, sheet: Value) -> napi::Result<()> {
        let meta: core::Sheet =
            serde_json::from_value(sheet).map_err(|e| errors::schema(e.to_string()))?;
        self.writer()?.start_sheet(meta).map_err(errors::from_core)
    }

    /// Stream one row into the open sheet.
    #[napi]
    pub fn write_row(&mut self, row: Value) -> napi::Result<()> {
        let row: core::Row =
            serde_json::from_value(row).map_err(|e| errors::schema(e.to_string()))?;
        self.writer()?.write_row(&row).map_err(errors::from_core)
    }

    /// Stream a whole chunk of rows from a JSON array string (`JSON.stringify` of
    /// `Row[]`). This is the throughput path for large exports: it skips the
    /// per-property N-API object walk that `writeRow` pays per cell — the caller
    /// stringifies a chunk in V8 (native) and Rust parses it in one pass. Keeps
    /// memory bounded by pushing one chunk at a time.
    #[napi]
    pub fn write_rows_json(&mut self, rows_json: String) -> napi::Result<()> {
        let rows: Vec<core::Row> =
            serde_json::from_str(&rows_json).map_err(|e| errors::schema(e.to_string()))?;
        let writer = self.writer()?;
        for row in &rows {
            writer.write_row(row).map_err(errors::from_core)?;
        }
        Ok(())
    }

    /// Stream a block of columns (the columnar fast path — the fastest ingestion
    /// shape). Numeric columns carry their values as a `Float64Array`, which
    /// crosses the N-API boundary as one buffer copy (zero per-cell FFI) and emits
    /// straight to XML with the number format interned once per column. Currency
    /// values are integer minor units; string columns carry `string[]`.
    #[napi]
    pub fn write_columns(&mut self, columns: Vec<JsColumn>) -> napi::Result<()> {
        let cols = columns
            .into_iter()
            .map(js_column_to_core)
            .collect::<napi::Result<Vec<_>>>()?;
        self.writer()?
            .write_columns(cols)
            .map_err(errors::from_core)
    }

    /// Close the open sheet (idempotent).
    #[napi]
    pub fn end_sheet(&mut self) -> napi::Result<()> {
        self.writer()?.end_sheet().map_err(errors::from_core)
    }

    /// Finish every sheet and ZIP the package. The writer is consumed; calling
    /// any method afterwards throws.
    #[napi]
    pub fn finish(&mut self) -> napi::Result<JsWriteResult> {
        let writer = self
            .inner
            .take()
            .ok_or_else(|| errors::schema("writer already finished"))?;
        finish(writer.finish())
    }

    /// Borrow the live writer, erroring if it was already finished.
    fn writer(&mut self) -> napi::Result<&mut core::WorkbookWriter> {
        self.inner
            .as_mut()
            .ok_or_else(|| errors::schema("writer already finished"))
    }
}

/// Lower the optional JS options into core [`core::WriteOptions`]. `password` is
/// accepted but deferred (v2), so it is intentionally dropped here.
fn to_options(opts: &Option<JsWriteOptions>) -> core::WriteOptions {
    let meta = opts.as_ref().and_then(|o| o.meta.as_ref());
    core::WriteOptions {
        meta: core::DocMeta {
            title: meta.and_then(|m| m.title.clone()),
            author: meta.and_then(|m| m.author.clone()),
            subject: meta.and_then(|m| m.subject.clone()),
            company: meta.and_then(|m| m.company.clone()),
        },
    }
}

/// Convert a core write result into the JS shape, mapping a fatal error to a
/// typed N-API throw.
fn finish(result: core::Result<core::WriteResult>) -> napi::Result<JsWriteResult> {
    let r = result.map_err(errors::from_core)?;
    Ok(JsWriteResult {
        xlsx: r.xlsx.into(),
        diagnostics: r.diagnostics.lints.iter().map(to_js_diagnostic).collect(),
    })
}

/// Convert one core lint into its JS wire shape.
fn to_js_diagnostic(lint: &core::Lint) -> JsDiagnostic {
    JsDiagnostic {
        code: lint.code.as_str().to_string(),
        message: lint.message.clone(),
    }
}

/// One column for the columnar fast path. `kind` selects the type; numeric
/// columns carry `numbers` (a `Float64Array`, zero-copy), string columns carry
/// `strings`. `currency`/`format` hold the matching format object; `decimals`
/// applies to percent columns.
#[napi(object)]
pub struct JsColumn {
    /// `"string"` | `"currency"` | `"number"` | `"percent"`.
    pub kind: String,
    pub currency: Option<Value>,
    pub format: Option<Value>,
    pub decimals: Option<u32>,
    pub strings: Option<Vec<String>>,
    pub numbers: Option<napi::bindgen_prelude::Float64Array>,
}

/// Lower one JS column into a core [`core::ColumnData`].
fn js_column_to_core(col: JsColumn) -> napi::Result<core::ColumnData> {
    match col.kind.as_str() {
        "string" => Ok(core::ColumnData::Strings(col.strings.unwrap_or_default())),
        "currency" => currency_column(col),
        "number" => number_column(col),
        "percent" => Ok(core::ColumnData::Percents {
            values: numbers(col.numbers),
            decimals: col.decimals,
        }),
        other => Err(errors::schema(format!("unknown column type {other:?}"))),
    }
}

/// Build a currency column: its `currency` format + minor-unit values.
fn currency_column(col: JsColumn) -> napi::Result<core::ColumnData> {
    let value = col
        .currency
        .ok_or_else(|| errors::schema("currency column needs a 'currency' format"))?;
    let format = serde_json::from_value(value).map_err(|e| errors::schema(e.to_string()))?;
    Ok(core::ColumnData::Currency {
        values: numbers(col.numbers),
        format,
    })
}

/// Build a number column with its optional `format`.
fn number_column(col: JsColumn) -> napi::Result<core::ColumnData> {
    let format = match col.format {
        Some(v) => Some(serde_json::from_value(v).map_err(|e| errors::schema(e.to_string()))?),
        None => None,
    };
    Ok(core::ColumnData::Numbers {
        values: numbers(col.numbers),
        format,
    })
}

/// The values of a numeric column as a `Vec<f64>` (empty when absent).
fn numbers(values: Option<napi::bindgen_prelude::Float64Array>) -> Vec<f64> {
    values.map(|a| a.to_vec()).unwrap_or_default()
}
