//! Emit one worksheet part (`xl/worksheets/sheetN.xml`) from a [`Sheet`].
//!
//! Strings are written **inline** (`t="inlineStr"`) rather than through a shared
//! string table: it keeps per-row work O(1) (nothing global grows per row), which
//! is what the streaming writer needs, and keeps the package one part smaller.
//! Cell values are native — numbers/dates/booleans are real typed values so Excel
//! sorts, sums and filters them; only the *format* is carried via the style index.

use std::collections::HashSet;

use crate::error::{Diagnostics, ErrorCode, LintCode, Result, TurboXlsxError};
use crate::model::{Cell, Column, CurrencyFormat, DateValue, Freeze, NumberFormat, Sheet};
use crate::numfmt::{self, ResolvedFmt};
use crate::style::{self, trim_num, StyleTable};
use crate::xml::{escape, escape_into};

/// Excel's maximum column width (characters).
const MAX_COL_WIDTH: f64 = 255.0;
/// Excel's maximum outline (grouping) depth.
const MAX_OUTLINE: u32 = 7;

/// Render a whole `sheet` to its worksheet XML, interning styles into `table`
/// and collecting non-fatal lints into `diags`. The batch (`write`) path.
pub fn write_sheet(
    sheet: &Sheet,
    locale: &str,
    table: &mut StyleTable,
    diags: &mut Diagnostics,
) -> Result<String> {
    let count = sheet.rows.len() as u32;
    let max_cols = sheet.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0) as u32;
    let mut s = sheet_prefix(sheet, count, max_cols, diags);
    s.reserve(estimate_body_bytes(count, max_cols));
    let mut cache = ColCache::new();
    for (i, row) in sheet.rows.iter().enumerate() {
        row_xml(
            &mut s, sheet, row, i as u32, locale, table, diags, &mut cache,
        )?;
    }
    s.push_str(&sheet_suffix(
        &sheet.merges,
        !sheet.images.is_empty(),
        diags,
    )?);
    Ok(s)
}

/// Rough byte estimate for a sheet body (~28 bytes per emitted cell), so the
/// output buffer is reserved once instead of doubling its way up through the
/// tens of MB a large export produces.
pub fn estimate_body_bytes(rows: u32, cols: u32) -> usize {
    (rows as usize)
        .saturating_mul(cols.max(1) as usize)
        .saturating_mul(28)
        + 256
}

/// Everything up to and including `<sheetData>`: the part the streaming writer
/// emits once the final row count + width are known. `rows`/`max_cols` size the
/// `<dimension>` reference.
pub fn sheet_prefix(sheet: &Sheet, rows: u32, max_cols: u32, diags: &mut Diagnostics) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
    );
    s.push_str(&sheet_pr(sheet));
    s.push_str(&format!(
        "<dimension ref=\"{}\"/>",
        dimension(rows, max_cols)
    ));
    s.push_str(&sheet_views(sheet.freeze.as_ref(), diags));
    s.push_str("<sheetFormatPr defaultRowHeight=\"15\"/>");
    s.push_str(&cols_xml(sheet, diags));
    s.push_str("<sheetData>");
    s
}

/// The worksheet tail: close `<sheetData>`, emit any merges, reference the sheet
/// drawing (when `has_images`), then close the document. The drawing relationship
/// is always `rId1` in the worksheet's own `_rels` (its only relationship).
pub fn sheet_suffix(
    merges: &[String],
    has_images: bool,
    diags: &mut Diagnostics,
) -> Result<String> {
    let mut s = String::from("</sheetData>");
    s.push_str(&merges_xml(merges, diags)?);
    if has_images {
        s.push_str(
            "<drawing xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId1\"/>",
        );
    }
    s.push_str("</worksheet>");
    Ok(s)
}

/// The `<sheetPr>` element — present only to carry outline summary direction.
fn sheet_pr(sheet: &Sheet) -> String {
    if sheet.outline.is_none() {
        return String::new();
    }
    "<sheetPr><outlinePr summaryBelow=\"1\" summaryRight=\"1\"/></sheetPr>".to_string()
}

/// The used-range reference, e.g. `A1:D12` (just `A1` for an empty sheet).
fn dimension(rows: u32, max_cols: u32) -> String {
    if rows == 0 {
        return "A1".to_string();
    }
    let last_col = col_letters(max_cols.saturating_sub(1));
    format!("A1:{last_col}{rows}")
}

/// The `<sheetViews>` block, carrying a frozen pane when requested.
fn sheet_views(freeze: Option<&Freeze>, diags: &mut Diagnostics) -> String {
    let pane = freeze.map(|f| pane_xml(f, diags)).unwrap_or_default();
    format!("<sheetViews><sheetView workbookViewId=\"0\">{pane}</sheetView></sheetViews>")
}

/// A frozen `<pane>` plus the matching active-pane selection.
fn pane_xml(freeze: &Freeze, diags: &mut Diagnostics) -> String {
    let rows = clamp_freeze(freeze.rows.unwrap_or(0), diags);
    let cols = clamp_freeze(freeze.cols.unwrap_or(0), diags);
    if rows == 0 && cols == 0 {
        return String::new();
    }
    let top_left = format!("{}{}", col_letters(cols), rows + 1);
    let mut s = String::from("<pane");
    if cols > 0 {
        s.push_str(&format!(" xSplit=\"{cols}\""));
    }
    if rows > 0 {
        s.push_str(&format!(" ySplit=\"{rows}\""));
    }
    s.push_str(&format!(
        " topLeftCell=\"{top_left}\" activePane=\"bottomRight\" state=\"frozen\"/>"
    ));
    s.push_str(&format!(
        "<selection pane=\"bottomRight\" activeCell=\"{top_left}\" sqref=\"{top_left}\"/>"
    ));
    s
}

/// Clamp a freeze split to Excel's sheet bounds, lint-warning if it was reduced.
fn clamp_freeze(n: u32, diags: &mut Diagnostics) -> u32 {
    if n > 16_384 {
        diags.push(
            LintCode::ClampedFreeze,
            format!("freeze split {n} clamped to 16384"),
        );
        return 16_384;
    }
    n
}

/// The `<cols>` block from the sheet's column definitions (omitted when none).
fn cols_xml(sheet: &Sheet, diags: &mut Diagnostics) -> String {
    if sheet.columns.is_empty() {
        return String::new();
    }
    let collapsed = sheet
        .outline
        .as_ref()
        .and_then(|o| o.columns_collapsed)
        .unwrap_or(false);
    let mut s = String::from("<cols>");
    for (i, col) in sheet.columns.iter().enumerate() {
        s.push_str(&col_xml(col, i as u32, collapsed, diags));
    }
    s.push_str("</cols>");
    s
}

/// One `<col>` element (1-based min/max), with width/outline/hidden attributes.
fn col_xml(col: &Column, idx: u32, collapsed: bool, diags: &mut Diagnostics) -> String {
    let n = idx + 1;
    let mut s = format!("<col min=\"{n}\" max=\"{n}\"");
    match col.width {
        Some(w) => s.push_str(&format!(
            " width=\"{}\" customWidth=\"1\"",
            clamp_width(w, diags)
        )),
        None => s.push_str(" width=\"8.43\""),
    }
    let level = col
        .outline_level
        .map(|l| clamp_outline(l, diags))
        .unwrap_or(0);
    if level > 0 {
        s.push_str(&format!(" outlineLevel=\"{level}\""));
    }
    if col_hidden(col, level, collapsed) {
        s.push_str(" hidden=\"1\"");
    }
    s.push_str("/>");
    s
}

/// Whether a column renders hidden: explicitly hidden, or grouped under a
/// collapsed outline.
fn col_hidden(col: &Column, level: u32, collapsed: bool) -> bool {
    col.hidden == Some(true) || (level > 0 && collapsed)
}

/// Clamp a column width into Excel's accepted range, lint-warning if reduced.
fn clamp_width(w: f64, diags: &mut Diagnostics) -> String {
    if !(0.0..=MAX_COL_WIDTH).contains(&w) {
        let clamped = w.clamp(0.0, MAX_COL_WIDTH);
        diags.push(
            LintCode::ClampedColumnWidth,
            format!("column width {w} clamped to {clamped}"),
        );
        return trim_num(clamped);
    }
    trim_num(w)
}

/// Clamp an outline level into `1..=7`, lint-warning if it was out of range.
fn clamp_outline(level: u32, diags: &mut Diagnostics) -> u32 {
    if level > MAX_OUTLINE {
        diags.push(
            LintCode::ClampedOutlineLevel,
            format!("outline level {level} clamped to 7"),
        );
        return MAX_OUTLINE;
    }
    level
}

/// Append one `<row>` and all its cells directly to `out`. Public so the
/// streaming writer can emit rows one at a time against a started sheet's
/// metadata. Writing into the caller's buffer avoids a per-row String alloc.
#[allow(clippy::too_many_arguments)]
pub fn row_xml(
    out: &mut String,
    sheet: &Sheet,
    row: &crate::model::Row,
    idx: u32,
    locale: &str,
    table: &mut StyleTable,
    diags: &mut Diagnostics,
    cache: &mut ColCache,
) -> Result<()> {
    out.push_str("<row r=\"");
    push_u32(out, idx + 1);
    out.push('"');
    if let Some(h) = row.height {
        out.push_str(" ht=\"");
        push_num(out, h);
        out.push_str("\" customHeight=\"1\"");
    }
    let level = row
        .outline_level
        .map(|l| clamp_outline(l, diags))
        .unwrap_or(0);
    if level > 0 {
        out.push_str(" outlineLevel=\"");
        push_u32(out, level);
        out.push('"');
    }
    out.push('>');
    let is_total = row.is_total.unwrap_or(false);
    let n = idx + 1;
    for (c, cell) in row.cells.iter().enumerate() {
        write_cell(
            out, sheet, cell, n, c as u32, is_total, locale, table, cache,
        )?;
    }
    out.push_str("</row>");
    Ok(())
}

/// Append one `<c>` cell to `out`: resolve its style + number format, intern it,
/// and write the native typed value — directly into the buffer, no per-cell
/// String allocation (the inner loop runs once per cell, millions of times). A
/// style-less numeric cell takes the column-cache fast path, which skips
/// rebuilding + hashing its number-format code on every cell.
#[allow(clippy::too_many_arguments)]
fn write_cell(
    out: &mut String,
    sheet: &Sheet,
    cell: &Cell,
    row: u32,
    col: u32,
    is_total: bool,
    locale: &str,
    table: &mut StyleTable,
    cache: &mut ColCache,
) -> Result<()> {
    if try_fast_numeric(out, sheet, cell, row, col, is_total, locale, table, cache) {
        return Ok(());
    }
    let col_style = sheet
        .columns
        .get(col as usize)
        .and_then(|c| c.style.as_ref());
    let style = style::resolve(col_style, is_total, cell_style(cell));
    let (fmt, type_attr, value) = resolve_value(cell, locale);
    let s_idx = table.intern(&style, &fmt)?;
    write_open_tag(out, row, col, s_idx, type_attr);
    push_value_body(out, value);
    Ok(())
}

/// The column-cache fast path: for a fully style-less numeric cell, reuse the
/// column's cached `xf` index (recomputing only on the first cell / a format
/// change) and write the value. Returns `true` when it handled the cell.
#[allow(clippy::too_many_arguments)]
fn try_fast_numeric(
    out: &mut String,
    sheet: &Sheet,
    cell: &Cell,
    row: u32,
    col: u32,
    is_total: bool,
    locale: &str,
    table: &mut StyleTable,
    cache: &mut ColCache,
) -> bool {
    let styled = is_total
        || cell_style(cell).is_some()
        || sheet
            .columns
            .get(col as usize)
            .is_some_and(|c| c.style.is_some());
    if styled {
        return false;
    }
    let Some(value) = numeric_value(cell) else {
        return false;
    };
    let s_idx = cache_numeric(cell, col, locale, table, cache);
    write_open_tag(out, row, col, s_idx, "");
    out.push_str("><v>");
    push_num(out, value);
    out.push_str("</v></c>");
    true
}

/// Look up (or compute + cache) the `xf` index for a style-less numeric cell,
/// keyed by its column's last-seen number format.
fn cache_numeric(
    cell: &Cell,
    col: u32,
    locale: &str,
    table: &mut StyleTable,
    cache: &mut ColCache,
) -> usize {
    if let Some(idx) = cache.get(col, cell) {
        return idx;
    }
    let fmt = cell_numfmt(cell, locale);
    let idx = table.intern_format(&fmt);
    cache.put(col, cell, idx);
    idx
}

/// Open a `<c r="A1" s="N" TYPE` tag (without closing it), shared by both paths.
fn write_open_tag(out: &mut String, row: u32, col: u32, s_idx: usize, type_attr: &str) {
    out.push_str("<c r=\"");
    push_col_letters(out, col);
    push_u32(out, row);
    out.push('"');
    if s_idx != 0 {
        out.push_str(" s=\"");
        push_u32(out, s_idx as u32);
        out.push('"');
    }
    out.push_str(type_attr);
}

/// The numeric value of a `currency`/`number`/`percent` cell (`None` otherwise).
fn numeric_value(cell: &Cell) -> Option<f64> {
    match cell {
        Cell::Currency {
            value, currency, ..
        } => {
            let decimals = currency.decimals.unwrap_or(2);
            Some(*value as f64 / 10f64.powi(decimals as i32))
        }
        Cell::Number { value, .. } => Some(*value),
        Cell::Percent { value, .. } => Some(*value),
        _ => None,
    }
}

/// The resolved number format of a `currency`/`number`/`percent` cell.
fn cell_numfmt(cell: &Cell, locale: &str) -> ResolvedFmt {
    match cell {
        Cell::Currency { currency, .. } => numfmt::currency(currency, locale),
        Cell::Percent { decimals, .. } => numfmt::percent(decimals.unwrap_or(2)),
        Cell::Number { format, .. } => format
            .as_ref()
            .map(numfmt::number)
            .unwrap_or(ResolvedFmt::Builtin(0)),
        _ => ResolvedFmt::Builtin(0),
    }
}

/// A per-column cache of the last numeric cell's format → `xf` index. Real
/// reports have format-uniform columns, so this hits ~always and collapses the
/// per-cell number-format rebuild + hash of a million-cell sheet to one per
/// column. Correct for mixed columns too: a format change just recomputes.
pub struct ColCache {
    cols: Vec<Option<ColEntry>>,
}

/// One column's cached number format and its interned `xf` index.
struct ColEntry {
    key: NumFmtKey,
    s_idx: usize,
}

/// The format-determining fields of a numeric cell, compared cheaply per cell.
enum NumFmtKey {
    Currency(CurrencyFormat),
    Number(Option<NumberFormat>),
    Percent(Option<u32>),
}

impl NumFmtKey {
    /// Build a key from a numeric cell (`None` for non-numeric cells).
    fn from_cell(cell: &Cell) -> Option<NumFmtKey> {
        match cell {
            Cell::Currency { currency, .. } => Some(NumFmtKey::Currency(currency.clone())),
            Cell::Number { format, .. } => Some(NumFmtKey::Number(format.clone())),
            Cell::Percent { decimals, .. } => Some(NumFmtKey::Percent(*decimals)),
            _ => None,
        }
    }

    /// Whether this key matches `cell`'s format (no allocation).
    fn matches(&self, cell: &Cell) -> bool {
        match (self, cell) {
            (NumFmtKey::Currency(a), Cell::Currency { currency, .. }) => a == currency,
            (NumFmtKey::Number(a), Cell::Number { format, .. }) => a == format,
            (NumFmtKey::Percent(a), Cell::Percent { decimals, .. }) => a == decimals,
            _ => false,
        }
    }
}

impl ColCache {
    /// A fresh, empty cache.
    pub fn new() -> Self {
        ColCache { cols: Vec::new() }
    }

    /// The cached `xf` index for `col` when its format matches `cell`.
    fn get(&self, col: u32, cell: &Cell) -> Option<usize> {
        let entry = self.cols.get(col as usize)?.as_ref()?;
        entry.key.matches(cell).then_some(entry.s_idx)
    }

    /// Record `cell`'s format + interned index as `col`'s cache entry.
    fn put(&mut self, col: u32, cell: &Cell, s_idx: usize) {
        let Some(key) = NumFmtKey::from_cell(cell) else {
            return;
        };
        let i = col as usize;
        if self.cols.len() <= i {
            self.cols.resize_with(i + 1, || None);
        }
        self.cols[i] = Some(ColEntry { key, s_idx });
    }
}

impl Default for ColCache {
    fn default() -> Self {
        Self::new()
    }
}

/// The cell's own style override, if any.
fn cell_style(cell: &Cell) -> Option<&crate::model::CellStyle> {
    match cell {
        Cell::String { style, .. }
        | Cell::Number { style, .. }
        | Cell::Currency { style, .. }
        | Cell::Percent { style, .. }
        | Cell::Date { style, .. }
        | Cell::Boolean { style, .. }
        | Cell::Blank { style } => style.as_ref(),
    }
}

/// A cell's emit-ready value, borrowing from the model where possible.
enum CellValue<'a> {
    Blank,
    Str(&'a str),
    Bool(bool),
    Num(f64),
}

/// Resolve a cell into its number format, `t=` type attribute, and value. The
/// date serial is computed once here. Strings/booleans carry an explicit type so
/// Excel reads them as text / TRUE-FALSE, not numbers.
// `rustfmt::skip` keeps each arm on a single line: tarpaulin's LLVM engine
// mis-attributes the opening line of a multi-line struct-pattern arm as
// uncovered even when the arm runs, so one-line arms keep the coverage gate honest.
#[rustfmt::skip]
fn resolve_value<'a>(cell: &'a Cell, locale: &str) -> (ResolvedFmt, &'static str, CellValue<'a>) {
    match cell {
        Cell::Blank { .. } => (ResolvedFmt::Builtin(0), "", CellValue::Blank),
        Cell::String { value, .. } => (ResolvedFmt::Builtin(0), " t=\"inlineStr\"", CellValue::Str(value)),
        Cell::Boolean { value, .. } => (ResolvedFmt::Builtin(0), " t=\"b\"", CellValue::Bool(*value)),
        Cell::Number { value, format, .. } => number_value(*value, format.as_ref()),
        Cell::Percent { value, decimals, .. } => percent_value(*value, *decimals),
        Cell::Currency { value, currency, .. } => currency_value(*value, currency, locale),
        Cell::Date { value, format, .. } => resolve_date(value, format.as_ref()),
    }
}

/// Resolve a `number` cell to its format + value (general when unspecified).
fn number_value<'a>(
    value: f64,
    format: Option<&NumberFormat>,
) -> (ResolvedFmt, &'static str, CellValue<'a>) {
    let fmt = format
        .map(numfmt::number)
        .unwrap_or(ResolvedFmt::Builtin(0));
    (fmt, "", CellValue::Num(value))
}

/// Resolve a percent cell to its format + value (default 2 decimals).
fn percent_value<'a>(
    value: f64,
    decimals: Option<u32>,
) -> (ResolvedFmt, &'static str, CellValue<'a>) {
    (
        numfmt::percent(decimals.unwrap_or(2)),
        "",
        CellValue::Num(value),
    )
}

/// Resolve a currency cell: scale the integer minor units to the major amount and
/// map locale + ISO code to the OOXML number format.
fn currency_value<'a>(
    value: i64,
    currency: &CurrencyFormat,
    locale: &str,
) -> (ResolvedFmt, &'static str, CellValue<'a>) {
    let decimals = currency.decimals.unwrap_or(2);
    let major = value as f64 / 10f64.powi(decimals as i32);
    (
        numfmt::currency(currency, locale),
        "",
        CellValue::Num(major),
    )
}

/// Resolve a date cell — a real serial when the value parses, else the raw ISO
/// text as an inline string.
fn resolve_date<'a>(
    value: &'a DateValue,
    format: Option<&crate::model::DateFormat>,
) -> (ResolvedFmt, &'static str, CellValue<'a>) {
    match value {
        DateValue::Serial(n) => (numfmt::date(format), "", CellValue::Num(*n)),
        DateValue::Iso(s) => match numfmt::iso_to_serial(s) {
            Some(n) => (numfmt::date(format), "", CellValue::Num(n)),
            None => (
                ResolvedFmt::Builtin(0),
                " t=\"inlineStr\"",
                CellValue::Str(s),
            ),
        },
    }
}

/// Append a cell's closing markup for its resolved value, preserving leading and
/// trailing whitespace on inline strings so padded labels are not trimmed.
fn push_value_body(out: &mut String, value: CellValue) {
    match value {
        CellValue::Blank => out.push_str("/>"),
        CellValue::Str(s) => {
            out.push_str("><is><t xml:space=\"preserve\">");
            escape_into(out, s);
            out.push_str("</t></is></c>");
        }
        CellValue::Bool(b) => {
            out.push_str("><v>");
            out.push(if b { '1' } else { '0' });
            out.push_str("</v></c>");
        }
        CellValue::Num(n) => {
            out.push_str("><v>");
            push_num(out, n);
            out.push_str("</v></c>");
        }
    }
}

/// Append a numeric value. A non-finite value (`NaN`/`±Inf`, which have no valid
/// SpreadsheetML form and would corrupt the file) is written as `0`. An
/// integer-valued float within the exact-integer range drops its fraction (via
/// itoa); anything else uses the shortest round-trip form.
fn push_num(out: &mut String, n: f64) {
    if !n.is_finite() {
        out.push('0');
    } else if n.fract() == 0.0 && n.abs() < 9.0e15 {
        push_i64(out, n as i64);
    } else {
        use std::fmt::Write;
        let _ = write!(out, "{n}");
    }
}

/// Append a `u32` to `out` without allocating (itoa).
fn push_u32(out: &mut String, n: u32) {
    let mut buf = itoa::Buffer::new();
    out.push_str(buf.format(n));
}

/// Append an `i64` to `out` without allocating (itoa).
fn push_i64(out: &mut String, n: i64) {
    let mut buf = itoa::Buffer::new();
    out.push_str(buf.format(n));
}

/// Append a 0-based column index as letters (`0`→`A`, `26`→`AA`) without
/// allocating — the per-cell hot path. Uses a small stack buffer.
fn push_col_letters(out: &mut String, mut idx: u32) {
    let mut buf = [0u8; 8];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'A' + (idx % 26) as u8;
        if idx < 26 {
            break;
        }
        idx = idx / 26 - 1;
    }
    out.push_str(std::str::from_utf8(&buf[i..]).expect("ASCII column letters are valid UTF-8"));
}

/// The `<mergeCells>` block, dropping duplicate ranges with a lint and rejecting
/// a malformed range fatally.
fn merges_xml(merges: &[String], diags: &mut Diagnostics) -> Result<String> {
    if merges.is_empty() {
        return Ok(String::new());
    }
    let kept = dedup_merges(merges, diags)?;
    let mut s = format!("<mergeCells count=\"{}\">", kept.len());
    for range in kept {
        s.push_str(&format!("<mergeCell ref=\"{}\"/>", escape(range)));
    }
    s.push_str("</mergeCells>");
    Ok(s)
}

/// Validate every merge range and drop later duplicates (Excel rejects
/// overlapping merges), linting each drop. A malformed range is fatal.
fn dedup_merges<'a>(merges: &'a [String], diags: &mut Diagnostics) -> Result<Vec<&'a String>> {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    for range in merges {
        validate_range(range)?;
        if seen.insert(range.to_ascii_uppercase()) {
            kept.push(range);
        } else {
            diags.push(
                LintCode::DroppedDuplicateMerge,
                format!("duplicate merge {range} dropped"),
            );
        }
    }
    Ok(kept)
}

/// Validate an `A1:B2` merge range — both endpoints must be parseable cell refs.
fn validate_range(range: &str) -> Result<()> {
    let bad = || {
        TurboXlsxError::new(
            ErrorCode::BadCellRef,
            format!("invalid merge range {range:?}"),
        )
    };
    let (a, b) = range.split_once(':').ok_or_else(bad)?;
    parse_cell_ref(a).ok_or_else(bad)?;
    parse_cell_ref(b).ok_or_else(bad)?;
    Ok(())
}

/// Parse an `A1`-style reference into 0-based (col, row); `None` if malformed.
fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = s.split_at(split);
    let col = letters_to_col(letters)?;
    let row: u32 = digits.parse().ok()?;
    (row > 0).then(|| (col, row - 1))
}

/// Convert column letters (`A`, `Z`, `AA`) to a 0-based column index. Rejects an
/// empty / non-uppercase / over-long (>3 letters, beyond Excel's `XFD`) run.
fn letters_to_col(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let mut col: u32 = 0;
    for b in letters.bytes() {
        col = col * 26 + (b - b'A' + 1) as u32;
    }
    Some(col - 1)
}

/// Convert a 0-based column index to its letters (`0` → `A`, `26` → `AA`).
pub fn col_letters(idx: u32) -> String {
    let mut s = String::new();
    push_col_letters(&mut s, idx);
    s
}

// ---- columnar fast path ----------------------------------------------------

/// A column of values for the columnar fast path: a fixed type + format and a
/// contiguous value vector. The fastest ingestion shape — numeric values cross a
/// binding as one typed-array copy and emit straight to XML, with no per-cell
/// structs and no per-cell number-format work (the format is interned once per
/// column). Currency/number/percent values are plain `f64`; currency values are
/// integer minor units (exact in `f64` up to 2^53).
pub enum ColumnData {
    Strings(Vec<String>),
    Currency {
        values: Vec<f64>,
        format: CurrencyFormat,
    },
    Numbers {
        values: Vec<f64>,
        format: Option<NumberFormat>,
    },
    Percents {
        values: Vec<f64>,
        decimals: Option<u32>,
    },
}

impl ColumnData {
    /// The row count this column carries.
    fn len(&self) -> usize {
        match self {
            ColumnData::Strings(v) => v.len(),
            ColumnData::Currency { values, .. } => values.len(),
            ColumnData::Numbers { values, .. } => values.len(),
            ColumnData::Percents { values, .. } => values.len(),
        }
    }
}

/// A column's emit plan: its once-interned style index + a borrow of its values.
struct ColPlan<'a> {
    s_idx: usize,
    kind: ColPlanKind<'a>,
}

/// How a planned column emits each cell.
enum ColPlanKind<'a> {
    Str(&'a [String]),
    /// `scale` = 10^decimals; the stored minor-unit value is divided by it.
    Currency {
        values: &'a [f64],
        scale: f64,
    },
    Num(&'a [f64]),
}

/// Emit rows for a set of columns directly to `out`, row-major, starting at
/// 0-based `base_row`. Returns the number of rows emitted. The format of each
/// column is interned ONCE here, not per cell.
pub fn write_columns(
    out: &mut String,
    columns: &[ColumnData],
    base_row: u32,
    table: &mut StyleTable,
    locale: &str,
) -> u32 {
    let plans: Vec<ColPlan> = columns
        .iter()
        .map(|c| plan_column(c, table, locale))
        .collect();
    let max_len = columns.iter().map(ColumnData::len).max().unwrap_or(0);
    for r in 0..max_len {
        let row = base_row + r as u32 + 1;
        out.push_str("<row r=\"");
        push_u32(out, row);
        out.push_str("\">");
        for (c, plan) in plans.iter().enumerate() {
            emit_column_cell(out, plan, r, row, c as u32);
        }
        out.push_str("</row>");
    }
    max_len as u32
}

/// Resolve a column to its once-interned style index + emit kind.
fn plan_column<'a>(col: &'a ColumnData, table: &mut StyleTable, locale: &str) -> ColPlan<'a> {
    match col {
        ColumnData::Strings(v) => ColPlan {
            s_idx: 0,
            kind: ColPlanKind::Str(v),
        },
        ColumnData::Currency { values, format } => ColPlan {
            s_idx: table.intern_format(&numfmt::currency(format, locale)),
            kind: ColPlanKind::Currency {
                values,
                scale: 10f64.powi(format.decimals.unwrap_or(2) as i32),
            },
        },
        ColumnData::Numbers { values, format } => ColPlan {
            s_idx: table.intern_format(&number_fmt(format.as_ref())),
            kind: ColPlanKind::Num(values),
        },
        ColumnData::Percents { values, decimals } => ColPlan {
            s_idx: table.intern_format(&numfmt::percent(decimals.unwrap_or(2))),
            kind: ColPlanKind::Num(values),
        },
    }
}

/// The number format of a `number` column (general when unspecified).
fn number_fmt(format: Option<&NumberFormat>) -> ResolvedFmt {
    format
        .map(numfmt::number)
        .unwrap_or(ResolvedFmt::Builtin(0))
}

/// Emit one cell of a planned column (skipping past the end of a short column).
fn emit_column_cell(out: &mut String, plan: &ColPlan, r: usize, row: u32, col: u32) {
    match &plan.kind {
        ColPlanKind::Str(v) => emit_str_cell(out, v, plan.s_idx, r, row, col),
        ColPlanKind::Currency { values, scale } => {
            emit_num_cell(out, values, *scale, plan.s_idx, r, row, col)
        }
        ColPlanKind::Num(v) => emit_num_cell(out, v, 1.0, plan.s_idx, r, row, col),
    }
}

/// Emit one inline-string cell from a column's values.
fn emit_str_cell(out: &mut String, values: &[String], s_idx: usize, r: usize, row: u32, col: u32) {
    let Some(value) = values.get(r) else {
        return;
    };
    write_open_tag(out, row, col, s_idx, " t=\"inlineStr\"");
    out.push_str("><is><t xml:space=\"preserve\">");
    escape_into(out, value);
    out.push_str("</t></is></c>");
}

/// Emit one numeric cell from a column's values (the minor-unit `scale` divides
/// currency values; 1.0 for plain numbers/percents).
#[allow(clippy::too_many_arguments)]
fn emit_num_cell(
    out: &mut String,
    values: &[f64],
    scale: f64,
    s_idx: usize,
    r: usize,
    row: u32,
    col: u32,
) {
    let Some(value) = values.get(r) else {
        return;
    };
    write_open_tag(out, row, col, s_idx, "");
    out.push_str("><v>");
    push_num(out, value / scale);
    out.push_str("</v></c>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ErrorCode, LintCode};
    use crate::model::{
        Cell, CellStyle, Column, CurrencyFormat, DateFormat, DateKind, DateValue, Font,
        NumberFormat, Outline, Row,
    };

    fn render(sheet: &Sheet) -> (String, Diagnostics) {
        let mut table = StyleTable::new();
        let mut diags = Diagnostics::default();
        let xml = write_sheet(sheet, "en-US", &mut table, &mut diags).unwrap();
        assert_well_formed(&xml);
        (xml, diags)
    }

    /// Catch malformed markup (e.g. a `<c …` tag missing its closing `>`): in our
    /// output a literal `<` only ever opens a tag and `>` only closes one — all
    /// content `<`/`>` are escaped — so a `<` seen while already inside a tag is a
    /// structural bug. This is what the earlier `contains()` assertions missed.
    fn assert_well_formed(xml: &str) {
        let mut in_tag = false;
        for ch in xml.chars() {
            if ch == '<' {
                assert!(!in_tag, "'<' inside an unclosed tag — malformed XML");
                in_tag = true;
            } else if ch == '>' {
                assert!(in_tag, "'>' outside a tag — malformed XML");
                in_tag = false;
            }
        }
        assert!(!in_tag, "unclosed tag at end of XML");
    }

    fn cur(code: &str) -> CurrencyFormat {
        CurrencyFormat {
            code: code.to_string(),
            locale: None,
            decimals: None,
            negative: None,
            symbol: None,
        }
    }

    #[test]
    fn renders_all_cell_types_and_layout() {
        let sheet = Sheet {
            name: "S".into(),
            columns: vec![
                Column {
                    width: Some(24.0),
                    ..Default::default()
                },
                Column {
                    key: None,
                    width: None,
                    hidden: Some(true),
                    outline_level: Some(8),
                    style: Some(CellStyle {
                        font: Some(Font {
                            bold: Some(true),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                },
            ],
            outline: Some(Outline {
                columns_collapsed: Some(true),
            }),
            freeze: Some(Freeze {
                rows: Some(1),
                cols: Some(2),
            }),
            merges: vec!["A1:B1".into(), "A1:B1".into()],
            rows: vec![
                Row {
                    cells: vec![
                        Cell::String {
                            value: " padded ".into(),
                            style: None,
                        },
                        Cell::Number {
                            value: 1000.0,
                            format: Some(NumberFormat {
                                decimals: Some(2),
                                ..Default::default()
                            }),
                            style: None,
                        },
                    ],
                    height: Some(20.0),
                    outline_level: Some(1),
                    is_total: None,
                },
                Row {
                    cells: vec![
                        Cell::Currency {
                            value: 123456,
                            currency: cur("USD"),
                            style: None,
                        },
                        Cell::Percent {
                            value: 0.15,
                            decimals: Some(1),
                            style: None,
                        },
                        Cell::Date {
                            value: DateValue::Iso("2024-01-01".into()),
                            format: Some(DateFormat {
                                kind: Some(DateKind::Date),
                                raw: None,
                            }),
                            style: None,
                        },
                        Cell::Date {
                            value: DateValue::Serial(45292.0),
                            format: None,
                            style: None,
                        },
                        Cell::Date {
                            value: DateValue::Iso("bad".into()),
                            format: None,
                            style: None,
                        },
                        Cell::Boolean {
                            value: true,
                            style: None,
                        },
                        Cell::Boolean {
                            value: false,
                            style: None,
                        },
                        Cell::Blank {
                            style: Some(CellStyle {
                                fill: Some("#eeeeee".into()),
                                ..Default::default()
                            }),
                        },
                    ],
                    is_total: Some(true),
                    ..Default::default()
                },
            ],
            images: vec![],
        };
        let (xml, diags) = render(&sheet);
        assert!(xml.contains("<sheetPr>"));
        assert!(xml.contains("xml:space=\"preserve\""));
        assert!(xml.contains("t=\"inlineStr\"") || xml.contains("<is>"));
        assert!(xml.contains("<v>1000</v>"));
        assert!(xml.contains("<v>1234.56</v>")); // currency minor units / 100
        assert!(xml.contains("state=\"frozen\""));
        assert!(xml.contains("outlineLevel=\"7\"")); // clamped from 8
        assert!(xml.contains("customHeight=\"1\""));
        assert!(xml.contains("<mergeCells count=\"1\">"));
        // duplicate merge + clamped outline both linted
        let codes: Vec<LintCode> = diags.lints.iter().map(|l| l.code).collect();
        assert!(codes.contains(&LintCode::DroppedDuplicateMerge));
        assert!(codes.contains(&LintCode::ClampedOutlineLevel));
    }

    #[test]
    fn empty_sheet_has_a1_dimension() {
        let (xml, _) = render(&Sheet {
            name: "S".into(),
            ..Default::default()
        });
        assert!(xml.contains("ref=\"A1\""));
    }

    #[test]
    fn freeze_rows_only_and_clamp() {
        let (xml, diags) = render(&Sheet {
            name: "S".into(),
            freeze: Some(Freeze {
                rows: Some(20000),
                cols: None,
            }),
            rows: vec![Row {
                cells: vec![Cell::Blank { style: None }],
                ..Default::default()
            }],
            ..Default::default()
        });
        assert!(xml.contains("ySplit=\"16384\""));
        assert!(diags
            .lints
            .iter()
            .any(|l| l.code == LintCode::ClampedFreeze));
    }

    #[test]
    fn freeze_zero_is_no_pane() {
        let (xml, _) = render(&Sheet {
            name: "S".into(),
            freeze: Some(Freeze {
                rows: Some(0),
                cols: Some(0),
            }),
            ..Default::default()
        });
        assert!(!xml.contains("<pane"));
    }

    #[test]
    fn clamps_negative_and_oversized_widths() {
        let (xml, diags) = render(&Sheet {
            name: "S".into(),
            columns: vec![
                Column {
                    width: Some(-5.0),
                    ..Default::default()
                },
                Column {
                    width: Some(9000.0),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        assert!(xml.contains("width=\"0\""));
        assert!(xml.contains("width=\"255\""));
        assert_eq!(
            diags
                .lints
                .iter()
                .filter(|l| l.code == LintCode::ClampedColumnWidth)
                .count(),
            2
        );
    }

    #[test]
    fn bad_merge_range_is_fatal() {
        let mut table = StyleTable::new();
        let mut diags = Diagnostics::default();
        let sheet = Sheet {
            name: "S".into(),
            merges: vec!["A1".into()],
            ..Default::default()
        };
        assert_eq!(
            write_sheet(&sheet, "en-US", &mut table, &mut diags)
                .unwrap_err()
                .code,
            ErrorCode::BadCellRef
        );
        let sheet2 = Sheet {
            name: "S".into(),
            merges: vec!["ZZZZZZZZ1:A1".into()],
            ..Default::default()
        };
        assert_eq!(
            write_sheet(&sheet2, "en-US", &mut table, &mut diags)
                .unwrap_err()
                .code,
            ErrorCode::BadCellRef
        );
        let sheet3 = Sheet {
            name: "S".into(),
            merges: vec!["a1:b2".into()],
            ..Default::default()
        };
        assert_eq!(
            write_sheet(&sheet3, "en-US", &mut table, &mut diags)
                .unwrap_err()
                .code,
            ErrorCode::BadCellRef
        );
        let sheet4 = Sheet {
            name: "S".into(),
            merges: vec!["A0:B2".into()],
            ..Default::default()
        };
        assert_eq!(
            write_sheet(&sheet4, "en-US", &mut table, &mut diags)
                .unwrap_err()
                .code,
            ErrorCode::BadCellRef
        );
        let sheet5 = Sheet {
            name: "S".into(),
            merges: vec!["1:B2".into()],
            ..Default::default()
        };
        assert_eq!(
            write_sheet(&sheet5, "en-US", &mut table, &mut diags)
                .unwrap_err()
                .code,
            ErrorCode::BadCellRef
        );
    }

    #[test]
    fn column_letters_cover_carry() {
        assert_eq!(col_letters(0), "A");
        assert_eq!(col_letters(25), "Z");
        assert_eq!(col_letters(26), "AA");
        assert_eq!(col_letters(701), "ZZ");
        assert_eq!(col_letters(702), "AAA");
    }

    fn cur_full(code: &str, locale: &str) -> CurrencyFormat {
        CurrencyFormat {
            code: code.to_string(),
            locale: Some(locale.to_string()),
            decimals: None,
            negative: None,
            symbol: None,
        }
    }

    fn num_cell(value: f64) -> Cell {
        Cell::Currency {
            value: value as i64,
            currency: cur_full("MXN", "es-MX"),
            style: None,
        }
    }

    /// Exercise the per-column number-format cache: a miss (first cell), hits
    /// (later cells, same format), a format change (different ISO code), and the
    /// `number`/`percent` numeric kinds — asserting EXACT, well-formed cell markup
    /// so a missing `>` (a class of bug `contains()` checks miss) cannot pass.
    fn column_cache_sheet() -> Sheet {
        Sheet {
            name: "S".into(),
            rows: vec![
                Row {
                    cells: vec![
                        Cell::String {
                            value: "a".into(),
                            style: None,
                        },
                        num_cell(111111.0),
                    ],
                    ..Default::default()
                },
                Row {
                    cells: vec![
                        Cell::String {
                            value: "b".into(),
                            style: None,
                        },
                        num_cell(222222.0),
                    ],
                    ..Default::default()
                },
                Row {
                    cells: vec![
                        Cell::Number {
                            value: 7.5,
                            format: Some(NumberFormat {
                                decimals: Some(2),
                                ..Default::default()
                            }),
                            style: None,
                        },
                        Cell::Currency {
                            value: 444444,
                            currency: cur_full("USD", "en-US"),
                            style: None,
                        },
                    ],
                    ..Default::default()
                },
                Row {
                    cells: vec![
                        Cell::Percent {
                            value: 0.16,
                            decimals: Some(1),
                            style: None,
                        },
                        Cell::Blank { style: None },
                    ],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn column_cache_emits_well_formed_exact_cells() {
        let (xml, _) = render(&column_cache_sheet());
        // Style-less currency cells in column B share one xf via the cache; the
        // markup must close the `<c>` tag before the value.
        assert!(xml.contains("<c r=\"B1\" s=\"1\"><v>1111.11</v></c>"));
        assert!(xml.contains("<c r=\"B2\" s=\"1\"><v>2222.22</v></c>"));
        // A format change (USD) interns a fresh xf, not the cached MXN one.
        assert!(xml.contains("<v>4444.44</v></c>"));
        assert!(!xml.contains("\"<v>"), "missing '>' before a value");
        // number + percent numeric kinds round-trip through the fast path too.
        assert!(xml.contains("<c r=\"A3\" s=\"2\"><v>7.5</v></c>"));
        assert!(xml.contains("<v>0.16</v></c>"));
    }

    #[test]
    fn nonfinite_and_huge_numbers_stay_well_formed() {
        let sheet = Sheet {
            name: "S".into(),
            rows: vec![Row {
                cells: vec![
                    Cell::Number {
                        value: f64::NAN,
                        format: None,
                        style: None,
                    },
                    Cell::Number {
                        value: f64::INFINITY,
                        format: None,
                        style: None,
                    },
                    Cell::Number {
                        value: f64::NEG_INFINITY,
                        format: None,
                        style: None,
                    },
                    Cell::Number {
                        value: 1e30,
                        format: None,
                        style: None,
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let (xml, _) = render(&sheet); // render() asserts well-formedness
                                       // NaN / ±Inf collapse to 0 (no invalid `<v>NaN</v>`).
        assert_eq!(xml.matches("<v>0</v>").count(), 3);
        // A huge integer-valued float uses full digits, NOT a truncated i64.
        assert!(xml.contains("<v>1000000000000000000000000000000</v>"));
        assert!(!xml.contains("9223372036854775807"));
    }

    #[test]
    fn columnar_emits_well_formed_exact_cells() {
        let mut table = StyleTable::new();
        let cols = vec![
            ColumnData::Strings(vec!["a".into(), "b".into()]),
            ColumnData::Currency {
                values: vec![111111.0, 222222.0],
                format: cur_full("MXN", "es-MX"),
            },
            ColumnData::Numbers {
                values: vec![7.5],
                format: Some(NumberFormat {
                    decimals: Some(2),
                    ..Default::default()
                }),
            },
            ColumnData::Percents {
                values: vec![0.16, 0.5],
                decimals: Some(1),
            },
            ColumnData::Strings(vec!["solo".into()]), // short string column (row 2 skipped)
        ];
        let mut out = String::new();
        let emitted = write_columns(&mut out, &cols, 0, &mut table, "es-MX");
        assert_eq!(emitted, 2);
        assert!(out.contains(">solo</t>") && !out.contains("<c r=\"E2\"")); // short col skip
        assert_well_formed(&format!("<sheetData>{out}</sheetData>"));
        assert!(out
            .contains("<c r=\"A1\" t=\"inlineStr\"><is><t xml:space=\"preserve\">a</t></is></c>"));
        assert!(out.contains("<c r=\"B1\" s=\"1\"><v>1111.11</v></c>"));
        assert!(out.contains("<c r=\"B2\" s=\"1\"><v>2222.22</v></c>"));
        assert!(out.contains("<v>7.5</v>")); // number column, row 1
        assert!(!out.contains("<c r=\"C2\"")); // short number column: no row-2 cell
        assert!(out.contains("<v>0.16</v>") && out.contains("<v>0.5</v>")); // percent column
                                                                            // every column's format interned once → exactly 3 custom xfs (cur/num/pct).
        assert!(out.contains("s=\"1\"") && out.contains("s=\"2\"") && out.contains("s=\"3\""));
    }

    #[test]
    fn columnar_empty_is_zero_rows() {
        let mut table = StyleTable::new();
        let mut out = String::new();
        assert_eq!(write_columns(&mut out, &[], 0, &mut table, "en-US"), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn cache_internals_cover_all_variants() {
        let s = Cell::String {
            value: "x".into(),
            style: None,
        };
        let num = Cell::Number {
            value: 1.0,
            format: None,
            style: None,
        };
        let pct = Cell::Percent {
            value: 0.5,
            decimals: None,
            style: None,
        };
        let curr = num_cell(100.0);
        // cell_numfmt's non-numeric arm falls back to General.
        assert_eq!(cell_numfmt(&s, "en-US"), ResolvedFmt::Builtin(0));
        // from_cell: numeric kinds yield a key, non-numeric yields None.
        assert!(NumFmtKey::from_cell(&s).is_none());
        let (kn, kp, kc) = (
            NumFmtKey::from_cell(&num).unwrap(),
            NumFmtKey::from_cell(&pct).unwrap(),
            NumFmtKey::from_cell(&curr).unwrap(),
        );
        // matches: same kind true, every cross-kind + non-numeric false.
        assert!(kn.matches(&num) && kp.matches(&pct) && kc.matches(&curr));
        assert!(!kc.matches(&num) && !kn.matches(&pct) && !kp.matches(&curr) && !kn.matches(&s));
        // ColCache: empty get, put + get, the resize path, and a non-numeric no-op.
        let mut c = ColCache::default();
        assert!(c.get(0, &num).is_none());
        c.put(0, &num, 5);
        c.put(3, &pct, 7);
        c.put(0, &Cell::Blank { style: None }, 9);
        assert_eq!(c.get(0, &num), Some(5));
        assert_eq!(c.get(3, &pct), Some(7));
    }

    #[test]
    fn column_cache_matches_styled_and_string_paths() {
        // A totals row (styled) and a per-cell style must bypass the numeric fast
        // path; assert they still produce well-formed bold/styled cells.
        let sheet = Sheet {
            name: "S".into(),
            rows: vec![
                Row {
                    cells: vec![num_cell(100.0)],
                    is_total: Some(true),
                    ..Default::default()
                },
                Row {
                    cells: vec![Cell::Currency {
                        value: 200,
                        currency: cur_full("MXN", "es-MX"),
                        style: Some(CellStyle {
                            font: Some(Font {
                                italic: Some(true),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let (xml, _) = render(&sheet);
        // Both rows: a styled currency cell with a non-zero, value-closed tag.
        assert!(xml.matches("</v></c>").count() >= 2);
        assert!(xml.contains("<v>1</v></c>") || xml.contains("<v>2</v></c>"));
    }
}
