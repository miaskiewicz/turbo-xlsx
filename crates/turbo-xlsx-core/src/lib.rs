//! turbo-xlsx core engine.
//!
//! A native writer that turns a structured **workbook model** into a formatted
//! `.xlsx` (OOXML SpreadsheetML, OPC-zipped). Every entry mode — a declarative
//! [`Workbook`], a JSON workbook (string or value, validated fail-closed), the
//! rows fast-path, and the row-by-row streaming [`WorkbookWriter`] — converges on
//! the same typed model and the same emitter. It is **write-only** and
//! **country-agnostic**: locale and ISO-4217 currency code are inputs, never
//! hardcoded. There are no formulas and no cross-sheet references — pre-computed
//! typed values in, a spreadsheet out.
//!
//! See `docs`/the spec for the full surface; the N-API binding mirrors these
//! functions one-to-one.

#![forbid(unsafe_code)]

pub mod error;
pub mod model;
mod numfmt;
pub mod package;
mod style;
pub mod validate;
mod worksheet;
pub mod writer;
mod xml;
mod zip;

pub use error::{Diagnostics, ErrorCode, Lint, LintCode, Result, TurboXlsxError};
pub use model::{
    Align, Border, BorderEdge, BorderStyle, Cell, CellStyle, Column, CurrencyFormat, DateFormat,
    DateKind, DateValue, DocMeta, Font, Freeze, HAlign, Negative, NumberFormat, Outline, Row,
    Sheet, VAlign, Workbook, WriteOptions,
};
pub use package::{WriteResult, DEFAULT_LOCALE};
pub use worksheet::ColumnData;
pub use writer::WorkbookWriter;

/// Internal phase entry points exposed ONLY under the `bench-internals` feature
/// so the hotspot harness (`benches/hotspot.rs`) can time each phase in
/// isolation. Off by default — never compiled into the shipped library or the
/// coverage build.
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod internals {
    pub use crate::package::package;
    pub use crate::style::StyleTable;
    pub use crate::worksheet::write_sheet;
    pub use crate::zip::{build as zip_build, crc32, Part};
}

use serde_json::Value;

/// One-shot: validate a complete [`Workbook`] and write it to `.xlsx` bytes.
pub fn write(workbook: &Workbook, opts: &WriteOptions) -> Result<WriteResult> {
    validate::validate(workbook)?;
    let mut diagnostics = Diagnostics::default();
    let xlsx = package::package(workbook, opts, &mut diagnostics)?;
    Ok(WriteResult { xlsx, diagnostics })
}

/// JSON in (string form): parse + schema-validate the documented workbook schema,
/// then write. A syntax error is `InvalidJson`; a wrong shape is `SchemaViolation`.
pub fn write_from_json_str(input: &str, opts: &WriteOptions) -> Result<WriteResult> {
    let workbook = validate::from_json_str(input)?;
    write(&workbook, opts)
}

/// JSON in (already-parsed value): schema-validate the shape, then write.
pub fn write_from_json_value(input: Value, opts: &WriteOptions) -> Result<WriteResult> {
    let workbook = validate::from_json_value(input)?;
    write(&workbook, opts)
}

/// Convenience fast-path: a single sheet from typed columns + rows. The rows are
/// already-typed cells (this is NOT a CSV ingester).
pub fn write_rows(
    sheet_name: Option<String>,
    columns: Vec<Column>,
    rows: Vec<Row>,
    locale: Option<String>,
    opts: &WriteOptions,
) -> Result<WriteResult> {
    let workbook = Workbook {
        schema_version: None,
        locale,
        sheets: vec![Sheet {
            name: sheet_name.unwrap_or_else(|| "Sheet1".to_string()),
            columns,
            rows,
            ..Sheet::default()
        }],
    };
    write(&workbook, opts)
}
