//! Pure conversions between core types and the Python wire shapes: write options
//! lowered from an optional `opts` dict, and diagnostics returned as a list of
//! `{code, message}` dicts. Mirrors the N-API marshaling 1:1.

use pyo3::prelude::*;
use pyo3::types::PyDict;

use turbo_xlsx_core::{Diagnostics, DocMeta, Lint, WriteOptions};

/// The `meta` string fields, paired with the setter that writes each onto the
/// [`DocMeta`] being built. Kept as data so [`meta_dict`] stays a single loop.
type MetaSetter = fn(&mut DocMeta, Option<String>);
const META_FIELDS: [(&str, MetaSetter); 4] = [
    ("title", |m, v| m.title = v),
    ("author", |m, v| m.author = v),
    ("subject", |m, v| m.subject = v),
    ("company", |m, v| m.company = v),
];

/// Lower an optional `opts` dict into core [`WriteOptions`]. Recognized keys:
/// `meta` (a dict of `title`/`author`/`subject`/`company`), `locale` (read by the
/// caller), and `password` (ECMA-376 Agile Encryption of the output).
pub fn write_options(opts: Option<&Bound<'_, PyDict>>) -> PyResult<WriteOptions> {
    let meta = match opts {
        Some(d) => meta_from_opts(d)?,
        None => DocMeta::default(),
    };
    let password = match opts {
        Some(d) => opt_str(d, "password")?,
        None => None,
    };
    Ok(WriteOptions { meta, password })
}

/// Read the optional `locale` key off an `opts` dict.
pub fn locale(opts: Option<&Bound<'_, PyDict>>) -> PyResult<Option<String>> {
    match opts {
        Some(d) => opt_str(d, "locale"),
        None => Ok(None),
    }
}

/// Pull the `meta` sub-dict off `opts` and lower it, defaulting when absent.
fn meta_from_opts(opts: &Bound<'_, PyDict>) -> PyResult<DocMeta> {
    match opts.get_item("meta")? {
        Some(v) => meta_dict(&v.downcast_into::<PyDict>()?),
        None => Ok(DocMeta::default()),
    }
}

/// Read the recognized keys off a present `meta` dict into [`DocMeta`].
fn meta_dict(m: &Bound<'_, PyDict>) -> PyResult<DocMeta> {
    let mut out = DocMeta::default();
    for (key, set) in META_FIELDS {
        set(&mut out, opt_str(m, key)?);
    }
    Ok(out)
}

/// Extract an optional `str` value for `key` from a dict.
fn opt_str(m: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<String>> {
    match m.get_item(key)? {
        Some(v) => Ok(Some(v.extract()?)),
        None => Ok(None),
    }
}

/// Convert one core lint into its Python dict `{code, message}`.
fn lint_to_py<'py>(py: Python<'py>, lint: &Lint) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("code", lint.code.as_str())?;
    d.set_item("message", &lint.message)?;
    Ok(d)
}

/// Convert the collected diagnostics into the Python list returned to callers.
pub fn diagnostics_to_py<'py>(
    py: Python<'py>,
    diags: &Diagnostics,
) -> PyResult<Vec<Bound<'py, PyDict>>> {
    diags.lints.iter().map(|l| lint_to_py(py, l)).collect()
}
