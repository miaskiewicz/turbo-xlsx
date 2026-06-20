//! turbo-xlsx PyO3 binding (PyPI: `turbo-xlsx`).
//!
//! Exposes the workbook-model -> XLSX writer of `turbo-xlsx-core` to Python,
//! mirroring the Node N-API binding 1:1. Every entry mode — a declarative
//! workbook dict, a JSON workbook (string or value, validated fail-closed), the
//! rows fast-path, and the row-by-row streaming [`WorkbookWriter`] — converges on
//! the same typed model and the same emitter.
//!
//! ## Boundary contract
//! * Input is an ordinary Python value (dict/list/scalar/str), bridged to
//!   `serde_json::Value` via `pythonize`.
//! * The written `.xlsx` crosses back as Python `bytes`.
//! * Fatal faults are raised as a typed `TurboXlsxError` (see `errors`) carrying
//!   `.code`/`.message`; non-fatal lints are *returned* in the `_full` results,
//!   never raised.
//!
//! The product surface is this thin marshaling layer; all writing logic lives in
//! the core crate. This crate is a cdylib that tarpaulin cannot line-instrument,
//! so it is excluded from the coverage gate and kept deliberately minimal.

#![deny(clippy::all)]

mod convert;
mod errors;

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use serde_json::Value;

use turbo_xlsx_core as core;

use convert::{diagnostics_to_py, locale, write_options};
use errors::{from_core, schema};

/// One-shot: a complete workbook dict -> `.xlsx` `bytes`. `opts` is an optional
/// dict `{meta: {title, author, subject, company}, locale}`.
#[pyfunction]
#[pyo3(signature = (workbook, opts=None))]
pub fn write(
    py: Python<'_>,
    workbook: Bound<'_, PyAny>,
    opts: Option<Bound<'_, PyDict>>,
) -> PyResult<Py<PyBytes>> {
    let value = depythonize(&workbook)?;
    let result = run_value(value, opts.as_ref())?;
    Ok(PyBytes::new(py, &result.xlsx).unbind())
}

/// One-shot returning `(bytes, [{code, message}, ...])`: the `.xlsx` plus the
/// returned non-fatal lints. Mirrors the html2pdf `render_full`.
#[pyfunction]
#[pyo3(signature = (workbook, opts=None))]
pub fn write_full<'py>(
    py: Python<'py>,
    workbook: Bound<'py, PyAny>,
    opts: Option<Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    let value = depythonize(&workbook)?;
    let result = run_value(value, opts.as_ref())?;
    result_to_tuple(py, result)
}

/// JSON in: a JSON string OR a Python value matching the workbook schema.
/// Validated fail-closed. Returns the `.xlsx` `bytes`.
#[pyfunction]
#[pyo3(signature = (input, opts=None))]
pub fn write_from_json(
    py: Python<'_>,
    input: Bound<'_, PyAny>,
    opts: Option<Bound<'_, PyDict>>,
) -> PyResult<Py<PyBytes>> {
    let options = write_options(opts.as_ref())?;
    let result = json_result(&input, &options)?;
    Ok(PyBytes::new(py, &result.xlsx).unbind())
}

/// Convenience fast-path: one sheet from typed columns + rows. NOT a CSV
/// ingester. `input` is a dict `{sheetName?, locale?, columns?, rows}`.
#[pyfunction]
#[pyo3(signature = (input, opts=None))]
pub fn write_rows(
    py: Python<'_>,
    input: Bound<'_, PyAny>,
    opts: Option<Bound<'_, PyDict>>,
) -> PyResult<Py<PyBytes>> {
    let parsed = rows_input(&input)?;
    let options = write_options(opts.as_ref())?;
    let result = core::write_rows(
        parsed.sheet_name,
        parsed.columns,
        parsed.rows,
        parsed.locale,
        &options,
    )
    .map_err(from_core)?;
    Ok(PyBytes::new(py, &result.xlsx).unbind())
}

/// Create a streaming writer for large sheets (module-level convenience).
#[pyfunction]
#[pyo3(signature = (opts=None))]
pub fn create_writer(opts: Option<Bound<'_, PyDict>>) -> PyResult<WorkbookWriter> {
    WorkbookWriter::build(opts)
}

/// The shape of the rows fast-path input, deserialized from `serde_json::Value`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RowsInput {
    sheet_name: Option<String>,
    locale: Option<String>,
    #[serde(default)]
    columns: Vec<core::Column>,
    #[serde(default)]
    rows: Vec<core::Row>,
}

/// Bridge a Python value into a `serde_json::Value`, mapping any failure to a
/// typed `SchemaViolation`.
fn depythonize(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    pythonize::depythonize(obj).map_err(|e| schema(e.to_string()))
}

/// Lower options and write a depythonized workbook value.
fn run_value(value: Value, opts: Option<&Bound<'_, PyDict>>) -> PyResult<core::WriteResult> {
    let options = write_options(opts)?;
    core::write_from_json_value(value, &options).map_err(from_core)
}

/// Write from a JSON `str` input, else depythonize and write the value.
fn json_result(
    input: &Bound<'_, PyAny>,
    options: &core::WriteOptions,
) -> PyResult<core::WriteResult> {
    let result = match input.extract::<String>() {
        Ok(s) => core::write_from_json_str(&s, options),
        Err(_) => core::write_from_json_value(depythonize(input)?, options),
    };
    result.map_err(from_core)
}

/// Deserialize the rows fast-path input from a Python value.
fn rows_input(input: &Bound<'_, PyAny>) -> PyResult<RowsInput> {
    let value = depythonize(input)?;
    serde_json::from_value(value).map_err(|e| schema(e.to_string()))
}

/// Build the `(bytes, [diagnostic...])` tuple shared by the `_full` results.
fn result_to_tuple<'py>(py: Python<'py>, result: core::WriteResult) -> PyResult<Bound<'py, PyAny>> {
    let bytes = PyBytes::new(py, &result.xlsx);
    let diags = PyList::new(py, diagnostics_to_py(py, &result.diagnostics)?)?;
    let tuple = (bytes, diags);
    Ok(tuple.into_pyobject(py)?.into_any())
}

/// A row-by-row streaming writer. Push a sheet, stream rows, end the sheet,
/// finish the package. See the core [`core::WorkbookWriter`].
#[pyclass(module = "turbo_xlsx")]
pub struct WorkbookWriter {
    inner: Option<core::WorkbookWriter>,
}

#[pymethods]
impl WorkbookWriter {
    /// Build the writer from `locale` + an optional `opts` metadata dict.
    #[new]
    #[pyo3(signature = (locale=None, opts=None))]
    fn new(locale: Option<String>, opts: Option<Bound<'_, PyDict>>) -> PyResult<Self> {
        let options = write_options(opts.as_ref())?;
        Ok(WorkbookWriter {
            inner: Some(core::WorkbookWriter::new(locale, options)),
        })
    }

    /// Begin a new sheet from its metadata dict (its `rows` are ignored — stream
    /// them with [`Self::write_row`]).
    fn start_sheet(&mut self, sheet: Bound<'_, PyAny>) -> PyResult<()> {
        let meta: core::Sheet = deserialize(&sheet)?;
        self.writer()?.start_sheet(meta).map_err(from_core)
    }

    /// Stream one row dict into the open sheet.
    fn write_row(&mut self, row: Bound<'_, PyAny>) -> PyResult<()> {
        let row: core::Row = deserialize(&row)?;
        self.writer()?.write_row(&row).map_err(from_core)
    }

    /// Stream a chunk of rows from a JSON array string (`json.dumps(rows)`). The
    /// throughput path for large exports: the chunk is stringified in C and
    /// parsed once in Rust, skipping the per-row `pythonize` object walk.
    fn write_rows_json(&mut self, rows_json: &str) -> PyResult<()> {
        let rows: Vec<core::Row> = serde_json::from_str(rows_json).map_err(|e| {
            from_core(core::TurboXlsxError::new(
                core::ErrorCode::SchemaViolation,
                e.to_string(),
            ))
        })?;
        let writer = self.writer()?;
        for row in &rows {
            writer.write_row(row).map_err(from_core)?;
        }
        Ok(())
    }

    /// Stream a typed table: `columns` is a list of per-column type specs
    /// (`{"type": "currency", "currency": {...}}`, `{"type": "string"}`, …) and
    /// `rows` is an iterable of rows of BARE scalar values. This is the fastest
    /// Python path for large exports — no per-cell dicts and no JSON, so it
    /// sidesteps CPython's slow `json.dumps`/object building. A `None` value emits
    /// a blank cell.
    fn write_table(&mut self, columns: Bound<'_, PyAny>, rows: Bound<'_, PyAny>) -> PyResult<()> {
        let specs = parse_colspecs(&columns)?;
        push_table(self.writer()?, &specs, &rows)
    }

    /// Close the open sheet (idempotent).
    fn end_sheet(&mut self) -> PyResult<()> {
        self.writer()?.end_sheet().map_err(from_core)
    }

    /// Finish every sheet and ZIP the package, returning `(bytes, [lint...])`.
    /// The writer is consumed; calling any method afterwards raises.
    fn finish<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let writer = self
            .inner
            .take()
            .ok_or_else(|| schema("writer already finished"))?;
        let result = writer.finish().map_err(from_core)?;
        result_to_tuple(py, result)
    }
}

impl WorkbookWriter {
    /// Shared constructor body for `__init__` and [`create_writer`].
    fn build(opts: Option<Bound<'_, PyDict>>) -> PyResult<WorkbookWriter> {
        let loc = locale(opts.as_ref())?;
        WorkbookWriter::new(loc, opts)
    }

    /// Borrow the live writer, erroring if it was already finished.
    fn writer(&mut self) -> PyResult<&mut core::WorkbookWriter> {
        self.inner
            .as_mut()
            .ok_or_else(|| schema("writer already finished"))
    }
}

/// Bridge a Python value into a typed core model via `serde_json`, mapping any
/// failure to a typed `SchemaViolation`.
fn deserialize<T: serde::de::DeserializeOwned>(obj: &Bound<'_, PyAny>) -> PyResult<T> {
    let value = depythonize(obj)?;
    serde_json::from_value(value).map_err(|e| schema(e.to_string()))
}

// ---- typed-table fast path -------------------------------------------------

/// Per-column cell-construction strategy parsed once from the column specs.
enum ColKind {
    Str,
    Boolean,
    Number(Option<core::NumberFormat>),
    Currency(core::CurrencyFormat),
    Percent(Option<u32>),
    Date(Option<core::DateFormat>),
}

/// Parse every column spec into its [`ColKind`].
fn parse_colspecs(columns: &Bound<'_, PyAny>) -> PyResult<Vec<ColKind>> {
    columns.try_iter()?.map(|c| parse_colspec(&c?)).collect()
}

/// Parse one column spec dict into its [`ColKind`].
fn parse_colspec(spec: &Bound<'_, PyAny>) -> PyResult<ColKind> {
    let ty: String = spec
        .get_item("type")
        .map_err(|_| schema("column spec needs a 'type'"))?
        .extract()?;
    colkind_for(&ty, spec)
}

/// Map a column `type` string + its spec to a [`ColKind`].
fn colkind_for(ty: &str, spec: &Bound<'_, PyAny>) -> PyResult<ColKind> {
    match ty {
        "string" => Ok(ColKind::Str),
        "boolean" => Ok(ColKind::Boolean),
        "number" => Ok(ColKind::Number(opt_fmt(spec, "format"))),
        "currency" => Ok(ColKind::Currency(req_currency(spec)?)),
        "percent" => Ok(ColKind::Percent(opt_u32(spec, "decimals"))),
        "date" => Ok(ColKind::Date(opt_fmt(spec, "format"))),
        other => Err(schema(format!("unknown column type {other:?}"))),
    }
}

/// Read an optional sub-format off a column spec (missing/malformed → `None`).
fn opt_fmt<T: serde::de::DeserializeOwned>(spec: &Bound<'_, PyAny>, key: &str) -> Option<T> {
    let item = spec.get_item(key).ok()?;
    let value: Value = depythonize(&item).ok()?;
    serde_json::from_value(value).ok()
}

/// Read the required `currency` sub-dict off a currency column spec.
fn req_currency(spec: &Bound<'_, PyAny>) -> PyResult<core::CurrencyFormat> {
    let item = spec
        .get_item("currency")
        .map_err(|_| schema("currency column needs a 'currency' spec"))?;
    deserialize(&item)
}

/// Read an optional `u32` off a column spec.
fn opt_u32(spec: &Bound<'_, PyAny>, key: &str) -> Option<u32> {
    spec.get_item(key).ok()?.extract().ok()
}

/// Stream every row of a typed table into the writer.
fn push_table(
    writer: &mut core::WorkbookWriter,
    specs: &[ColKind],
    rows: &Bound<'_, PyAny>,
) -> PyResult<()> {
    for row in rows.try_iter()? {
        push_table_row(writer, specs, &row?)?;
    }
    Ok(())
}

/// Build + stream one table row from its bare scalar values.
fn push_table_row(
    writer: &mut core::WorkbookWriter,
    specs: &[ColKind],
    row: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let cells = build_row_cells(specs, row)?;
    writer
        .write_row(&core::Row {
            cells,
            ..Default::default()
        })
        .map_err(from_core)
}

/// Build the cells of one row, one per column spec.
fn build_row_cells(specs: &[ColKind], row: &Bound<'_, PyAny>) -> PyResult<Vec<core::Cell>> {
    let mut cells = Vec::with_capacity(specs.len());
    for (i, kind) in specs.iter().enumerate() {
        cells.push(build_cell(kind, &row.get_item(i)?)?);
    }
    Ok(cells)
}

/// Build one typed cell from a column kind and a bare scalar value (`None` →
/// blank).
fn build_cell(kind: &ColKind, value: &Bound<'_, PyAny>) -> PyResult<core::Cell> {
    if value.is_none() {
        return Ok(core::Cell::Blank { style: None });
    }
    match kind {
        ColKind::Str => str_cell(value),
        ColKind::Boolean => bool_cell(value),
        ColKind::Number(f) => number_cell(value, f),
        ColKind::Currency(c) => currency_cell(value, c),
        ColKind::Percent(d) => percent_cell(value, *d),
        ColKind::Date(f) => date_cell(value, f),
    }
}

/// A `string` cell from a scalar.
fn str_cell(value: &Bound<'_, PyAny>) -> PyResult<core::Cell> {
    Ok(core::Cell::String {
        value: value.extract()?,
        style: None,
    })
}

/// A `boolean` cell from a scalar.
fn bool_cell(value: &Bound<'_, PyAny>) -> PyResult<core::Cell> {
    Ok(core::Cell::Boolean {
        value: value.extract()?,
        style: None,
    })
}

/// A `number` cell from a scalar.
fn number_cell(
    value: &Bound<'_, PyAny>,
    format: &Option<core::NumberFormat>,
) -> PyResult<core::Cell> {
    Ok(core::Cell::Number {
        value: value.extract()?,
        format: format.clone(),
        style: None,
    })
}

/// A `currency` cell (integer minor units) from a scalar.
fn currency_cell(
    value: &Bound<'_, PyAny>,
    currency: &core::CurrencyFormat,
) -> PyResult<core::Cell> {
    Ok(core::Cell::Currency {
        value: value.extract()?,
        currency: currency.clone(),
        style: None,
    })
}

/// A `percent` cell from a scalar.
fn percent_cell(value: &Bound<'_, PyAny>, decimals: Option<u32>) -> PyResult<core::Cell> {
    Ok(core::Cell::Percent {
        value: value.extract()?,
        decimals,
        style: None,
    })
}

/// A `date` cell from a scalar (ISO string or Excel serial number).
fn date_cell(value: &Bound<'_, PyAny>, format: &Option<core::DateFormat>) -> PyResult<core::Cell> {
    let date = match value.extract::<String>() {
        Ok(s) => core::DateValue::Iso(s),
        Err(_) => core::DateValue::Serial(value.extract()?),
    };
    Ok(core::Cell::Date {
        value: date,
        format: format.clone(),
        style: None,
    })
}

/// Register the one-shot write entry points (`write`, `write_full`).
fn register_write_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(write, m)?)?;
    m.add_function(wrap_pyfunction!(write_full, m)?)
}

/// Register the JSON / rows entry points.
fn register_input_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(write_from_json, m)?)?;
    m.add_function(wrap_pyfunction!(write_rows, m)?)
}

/// Register the streaming entry point (`create_writer`).
fn register_stream_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(create_writer, m)?)
}

/// Register all module-level functions.
fn register_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_write_functions(m)?;
    register_input_functions(m)?;
    register_stream_functions(m)
}

/// The Python extension module `turbo_xlsx._turbo_xlsx`. Re-exported by the
/// pure-Python `turbo_xlsx/__init__.py` shim.
#[pymodule]
fn _turbo_xlsx(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<WorkbookWriter>()?;
    register_functions(m)?;
    m.add(
        "TurboXlsxError",
        m.py().get_type::<errors::TurboXlsxError>(),
    )
}
