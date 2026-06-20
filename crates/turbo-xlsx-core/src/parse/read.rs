//! Read the OOXML parts of an `.xlsx` into a simple [`ParsedWorkbook`]: resolve
//! sheet names + relationships, the shared-string table, and which styles are
//! dates, then walk each worksheet into rows of typed [`CellValue`]s.

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::unzip::{read_zip, Entry};
use super::xml::{Event, Reader, Tag};
use super::ParseError;

/// A single parsed cell value (a date is an ISO-8601 string).
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
    Date(String),
}

/// One parsed worksheet: its name + a dense grid of rows.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSheet {
    pub name: String,
    pub rows: Vec<Vec<CellValue>>,
}

/// A parsed workbook: its sheets in tab order.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedWorkbook {
    pub sheets: Vec<ParsedSheet>,
}

/// Parse `.xlsx` bytes into a [`ParsedWorkbook`].
pub fn parse(bytes: &[u8]) -> Result<ParsedWorkbook, ParseError> {
    let entries = read_zip(bytes)?;
    let wb = part(&entries, "xl/workbook.xml").ok_or(ParseError::NotXlsx)?;
    let sheet_refs = parse_workbook(&xml_of(wb));
    let rels = parse_rels(&entries);
    let shared = parse_shared(&entries);
    let date_xf = parse_date_styles(&entries);
    let mut sheets = Vec::with_capacity(sheet_refs.len());
    for (name, rid) in sheet_refs {
        let rows = parse_one_sheet(&entries, &rels, &rid, &shared, &date_xf);
        sheets.push(ParsedSheet { name, rows });
    }
    Ok(ParsedWorkbook { sheets })
}

/// Find a part's bytes by its zip name.
fn part<'a>(entries: &'a [Entry], name: &str) -> Option<&'a [u8]> {
    entries
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.data.as_slice())
}

/// A part's bytes as `&str` — **borrowed** when already valid UTF-8 (the OOXML
/// case), so the whole part is not copied just to tokenize it.
fn xml_of(bytes: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

/// The sheet `(name, r:id)` list from `xl/workbook.xml`, in tab order.
fn parse_workbook(xml: &str) -> Vec<(String, String)> {
    let mut r = Reader::new(xml);
    let mut out = Vec::new();
    while let Some(ev) = r.read() {
        if let Event::Open(tag) = ev {
            push_sheet_ref(&mut out, &tag);
        }
    }
    out
}

/// Collect a `<sheet name=".." r:id="..">` reference.
fn push_sheet_ref(out: &mut Vec<(String, String)>, tag: &Tag) {
    if tag.name == "sheet" {
        if let (Some(n), Some(rid)) = (tag.attr("name"), tag.attr("r:id")) {
            out.push((n.to_string(), rid.to_string()));
        }
    }
}

/// The `rId -> Target` map from `xl/_rels/workbook.xml.rels`.
fn parse_rels(entries: &[Entry]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(bytes) = part(entries, "xl/_rels/workbook.xml.rels") else {
        return map;
    };
    let xml = xml_of(bytes);
    let mut r = Reader::new(&xml);
    while let Some(ev) = r.read() {
        if let Event::Open(tag) = ev {
            collect_rel(&mut map, &tag);
        }
    }
    map
}

/// Collect a `<Relationship Id=".." Target="..">` mapping.
fn collect_rel(map: &mut BTreeMap<String, String>, tag: &Tag) {
    if tag.name == "Relationship" {
        if let (Some(id), Some(target)) = (tag.attr("Id"), tag.attr("Target")) {
            map.insert(id.to_string(), target.to_string());
        }
    }
}

/// The shared-string table from `xl/sharedStrings.xml` (concatenating runs).
fn parse_shared(entries: &[Entry]) -> Vec<String> {
    let mut out = Vec::new();
    let Some(bytes) = part(entries, "xl/sharedStrings.xml") else {
        return out;
    };
    let xml = xml_of(bytes);
    let mut r = Reader::new(&xml);
    let (mut buf, mut in_t) = (String::new(), false);
    while let Some(ev) = r.read() {
        shared_event(&mut out, &mut buf, &mut in_t, ev);
    }
    out
}

/// Fold one sharedStrings event into the table.
fn shared_event(out: &mut Vec<String>, buf: &mut String, in_t: &mut bool, ev: Event) {
    match ev {
        Event::Open(tag) if tag.name == "si" => buf.clear(),
        Event::Open(tag) if tag.name == "t" => *in_t = true,
        Event::Text(t) if *in_t => buf.push_str(&t),
        Event::Close("t") => *in_t = false,
        Event::Close("si") => out.push(std::mem::take(buf)),
        _ => {}
    }
}

/// Per-`cellXfs`-index flag of whether that style is a date format.
fn parse_date_styles(entries: &[Entry]) -> Vec<bool> {
    let mut out = Vec::new();
    let Some(bytes) = part(entries, "xl/styles.xml") else {
        return out;
    };
    let xml = xml_of(bytes);
    let customs = collect_custom_numfmts(&xml);
    let mut r = Reader::new(&xml);
    let mut in_cellxfs = false;
    while let Some(ev) = r.read() {
        style_event(&mut out, &customs, &mut in_cellxfs, ev);
    }
    out
}

/// Fold one styles event: track `<cellXfs>` and record each `<xf>`'s date-ness.
fn style_event(
    out: &mut Vec<bool>,
    customs: &BTreeMap<u32, String>,
    in_cellxfs: &mut bool,
    ev: Event,
) {
    match ev {
        Event::Open(tag) if tag.name == "cellXfs" => *in_cellxfs = true,
        Event::Close("cellXfs") => *in_cellxfs = false,
        Event::Open(tag) if *in_cellxfs && tag.name == "xf" => out.push(xf_is_date(&tag, customs)),
        _ => {}
    }
}

/// Whether a cell `<xf>`'s number format is a date.
fn xf_is_date(tag: &Tag, customs: &BTreeMap<u32, String>) -> bool {
    let id: u32 = tag
        .attr("numFmtId")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    is_date_numfmt(id, customs)
}

/// The custom `numFmtId -> formatCode` map from a styles part.
fn collect_custom_numfmts(xml: &str) -> BTreeMap<u32, String> {
    let mut map = BTreeMap::new();
    let mut r = Reader::new(xml);
    while let Some(ev) = r.read() {
        if let Event::Open(tag) = ev {
            collect_numfmt(&mut map, &tag);
        }
    }
    map
}

/// Collect a `<numFmt numFmtId=".." formatCode="..">`.
fn collect_numfmt(map: &mut BTreeMap<u32, String>, tag: &Tag) {
    if tag.name == "numFmt" {
        if let (Some(id), Some(code)) = (tag.attr("numFmtId"), tag.attr("formatCode")) {
            if let Ok(id) = id.parse::<u32>() {
                map.insert(id, code.to_string());
            }
        }
    }
}

/// Whether a number-format id denotes a date/time (built-in range or a custom
/// code with date tokens).
fn is_date_numfmt(id: u32, customs: &BTreeMap<u32, String>) -> bool {
    if matches!(id, 14..=22 | 45..=47) {
        return true;
    }
    match customs.get(&id) {
        Some(code) => code_is_date(code),
        None => false,
    }
}

/// Heuristic: a custom format code is a date when it carries year/day tokens or
/// an `h:` time. (Built-in date ids are handled separately.)
fn code_is_date(code: &str) -> bool {
    let c = code.to_ascii_lowercase();
    c.contains('y') || c.contains('d') || c.contains("h:")
}

/// Parse a single sheet (resolved via its relationship) into rows.
fn parse_one_sheet(
    entries: &[Entry],
    rels: &BTreeMap<String, String>,
    rid: &str,
    shared: &[String],
    date_xf: &[bool],
) -> Vec<Vec<CellValue>> {
    let Some(target) = rels.get(rid) else {
        return Vec::new();
    };
    let path = sheet_path(target);
    let Some(bytes) = part(entries, &path) else {
        return Vec::new();
    };
    parse_sheet_xml(&xml_of(bytes), shared, date_xf)
}

/// Resolve a relationship target to its zip part name.
fn sheet_path(target: &str) -> String {
    match target.strip_prefix('/') {
        Some(abs) => abs.to_string(),
        None => format!("xl/{target}"),
    }
}

/// Walk a worksheet part into rows of typed values.
fn parse_sheet_xml(xml: &str, shared: &[String], date_xf: &[bool]) -> Vec<Vec<CellValue>> {
    let mut r = Reader::new(xml);
    let mut s = SheetState::default();
    while let Some(ev) = r.read() {
        sheet_event(&mut s, shared, date_xf, ev);
    }
    s.rows
}

/// A cell's `t` attribute, decoded to a copyable tag (so per-cell type tracking
/// costs no allocation).
#[derive(Clone, Copy, Default, PartialEq)]
enum CType {
    #[default]
    Num,
    Shared,
    Bool,
    Str,
}

/// Classify a cell's `t` attribute.
fn ctype_of(t: Option<&str>) -> CType {
    match t {
        Some("s") => CType::Shared,
        Some("b") => CType::Bool,
        Some("inlineStr" | "str" | "e") => CType::Str,
        _ => CType::Num,
    }
}

/// Mutable state while walking a worksheet.
#[derive(Default)]
struct SheetState {
    rows: Vec<Vec<CellValue>>,
    row: Vec<CellValue>,
    col: usize,
    ctype: CType,
    cstyle: usize,
    text: bool,
    invalue: bool,
    vbuf: String,
}

/// Fold one worksheet event into the sheet state.
fn sheet_event(s: &mut SheetState, shared: &[String], date_xf: &[bool], ev: Event) {
    match ev {
        Event::Open(tag) if tag.name == "row" => s.row = Vec::new(),
        Event::Open(tag) if tag.name == "c" => open_cell(s, &tag, shared, date_xf),
        Event::Open(tag) if tag.name == "v" => start_value(s),
        Event::Open(tag) if tag.name == "t" => s.text = true,
        Event::Text(t) if s.invalue || s.text => s.vbuf.push_str(&t),
        Event::Close("v") => s.invalue = false,
        Event::Close("t") => s.text = false,
        Event::Close("c") => end_cell(s, shared, date_xf),
        Event::Close("row") => s.rows.push(std::mem::take(&mut s.row)),
        _ => {}
    }
}

/// Begin a `<v>` value capture.
fn start_value(s: &mut SheetState) {
    s.invalue = true;
    s.vbuf.clear();
}

/// Open a `<c>` cell; a self-closing `<c …/>` (a blank cell) ends immediately.
fn open_cell(s: &mut SheetState, tag: &Tag, shared: &[String], date_xf: &[bool]) {
    begin_cell(s, tag);
    if tag.self_closing {
        end_cell(s, shared, date_xf);
    }
}

/// Begin a `<c>` cell: record its type/style/column and reset the buffer.
fn begin_cell(s: &mut SheetState, tag: &Tag) {
    s.ctype = ctype_of(tag.attr("t"));
    s.cstyle = tag.attr("s").and_then(|v| v.parse().ok()).unwrap_or(0);
    s.col = match tag.attr("r") {
        Some(rf) => col_of_ref(rf),
        None => s.row.len(),
    };
    s.vbuf.clear();
    s.text = false;
    s.invalue = false;
}

/// End a `<c>` cell: build its value, padding the row up to its column.
fn end_cell(s: &mut SheetState, shared: &[String], date_xf: &[bool]) {
    let value = build_value(s.ctype, s.cstyle, &s.vbuf, shared, date_xf);
    while s.row.len() < s.col {
        s.row.push(CellValue::Empty);
    }
    s.row.push(value);
}

/// The 0-based column index of a cell ref like `B3`.
fn col_of_ref(rf: &str) -> usize {
    let mut col = 0usize;
    for b in rf.bytes() {
        if !b.is_ascii_alphabetic() {
            break;
        }
        col = col * 26 + (b.to_ascii_uppercase() - b'A' + 1) as usize;
    }
    col.saturating_sub(1)
}

/// Build a typed value from a cell's type, style, and text buffer.
fn build_value(
    ctype: CType,
    style: usize,
    vbuf: &str,
    shared: &[String],
    date_xf: &[bool],
) -> CellValue {
    match ctype {
        CType::Shared => shared_value(vbuf, shared),
        CType::Str => text_or_empty(vbuf),
        CType::Bool => CellValue::Bool(vbuf == "1"),
        CType::Num => number_value(vbuf, style, date_xf),
    }
}

/// Resolve a shared-string cell to its text.
fn shared_value(vbuf: &str, shared: &[String]) -> CellValue {
    match vbuf.parse::<usize>().ok().and_then(|i| shared.get(i)) {
        Some(text) => CellValue::Text(text.clone()),
        None => CellValue::Empty,
    }
}

/// `Text` unless the buffer is empty (then `Empty`).
fn text_or_empty(vbuf: &str) -> CellValue {
    if vbuf.is_empty() {
        CellValue::Empty
    } else {
        CellValue::Text(vbuf.to_string())
    }
}

/// Build a numeric cell — a `Date` (ISO) when its style is a date format.
fn number_value(vbuf: &str, style: usize, date_xf: &[bool]) -> CellValue {
    if vbuf.is_empty() {
        return CellValue::Empty;
    }
    let n: f64 = match vbuf.parse() {
        Ok(n) => n,
        Err(_) => return CellValue::Text(vbuf.to_string()),
    };
    if date_xf.get(style).copied().unwrap_or(false) {
        CellValue::Date(serial_to_iso(n))
    } else {
        CellValue::Number(n)
    }
}

/// Convert an Excel serial number to an ISO-8601 date (or datetime).
fn serial_to_iso(serial: f64) -> String {
    let days = serial.floor() as i64 - 25_569;
    let (y, m, d) = civil_from_days(days);
    let frac = serial - serial.floor();
    if frac > 1e-9 {
        format!("{y:04}-{m:02}-{d:02}T{}", hms(frac))
    } else {
        format!("{y:04}-{m:02}-{d:02}")
    }
}

/// Howard Hinnant's `civil_from_days`: days-since-epoch → (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Render a day fraction as `HH:MM:SS`.
fn hms(frac: f64) -> String {
    let total = (frac * 86_400.0).round() as i64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
}
