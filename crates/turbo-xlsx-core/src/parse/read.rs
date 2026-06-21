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

/// One parsed worksheet: its name, a dense grid of rows, and any embedded images.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSheet {
    pub name: String,
    pub rows: Vec<Vec<CellValue>>,
    pub images: Vec<ParsedImage>,
}

/// A parsed embedded image: its bytes (base64) + format + where it is anchored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImage {
    /// Base64-encoded image bytes (round-trippable straight back into the model).
    pub data: String,
    /// The image format extension (`png` / `jpeg` / `gif`).
    pub format: String,
    /// Where and how big the image sits.
    pub anchor: ParsedAnchor,
}

/// A parsed image anchor: a two-cell range or a one-cell pinned size (pixels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedAnchor {
    TwoCell {
        from_col: u32,
        from_row: u32,
        to_col: u32,
        to_row: u32,
    },
    OneCell {
        col: u32,
        row: u32,
        width: u32,
        height: u32,
    },
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
        sheets.push(parse_sheet(&entries, &rels, name, &rid, &shared, &date_xf));
    }
    Ok(ParsedWorkbook { sheets })
}

/// Resolve one sheet reference into its rows + embedded images.
fn parse_sheet(
    entries: &[Entry],
    rels: &BTreeMap<String, String>,
    name: String,
    rid: &str,
    shared: &[String],
    date_xf: &[bool],
) -> ParsedSheet {
    let (rows, images) = match rels.get(rid).map(|t| sheet_path(t)) {
        Some(path) => (
            parse_sheet_rows(entries, &path, shared, date_xf),
            parse_sheet_images(entries, &path),
        ),
        None => (Vec::new(), Vec::new()),
    };
    ParsedSheet { name, rows, images }
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

/// Parse a worksheet part (already resolved to its zip path) into rows.
fn parse_sheet_rows(
    entries: &[Entry],
    path: &str,
    shared: &[String],
    date_xf: &[bool],
) -> Vec<Vec<CellValue>> {
    let Some(bytes) = part(entries, path) else {
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

/// One `<Relationship>` (id + type + target) from a `.rels` part.
struct Rel {
    id: String,
    typ: String,
    target: String,
}

/// Every relationship in a `.rels` part (empty when the part is absent).
fn parse_rel_list(entries: &[Entry], path: &str) -> Vec<Rel> {
    let mut out = Vec::new();
    let Some(bytes) = part(entries, path) else {
        return out;
    };
    let xml = xml_of(bytes);
    let mut r = Reader::new(&xml);
    while let Some(ev) = r.read() {
        if let Event::Open(tag) = ev {
            push_rel(&mut out, &tag);
        }
    }
    out
}

/// Collect one `<Relationship Id Type Target>` row.
fn push_rel(out: &mut Vec<Rel>, tag: &Tag) {
    if tag.name == "Relationship" {
        if let (Some(id), Some(target)) = (tag.attr("Id"), tag.attr("Target")) {
            out.push(Rel {
                id: id.to_string(),
                typ: tag.attr("Type").unwrap_or("").to_string(),
                target: target.to_string(),
            });
        }
    }
}

/// The `_rels` part path for a part (`a/b/c.xml` → `a/b/_rels/c.xml.rels`).
fn rels_path_of(part_path: &str) -> String {
    match part_path.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part_path}.rels"),
    }
}

/// Resolve a (possibly `../`-relative or `/`-absolute) relationship target
/// against the directory of `base_part`, returning a normalized zip path.
fn resolve(base_part: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let dir = base_part.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut segs: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// Extract every embedded image of a worksheet: follow its `_rels` to the
/// drawing part, then that drawing's anchors + media. Empty when the sheet has
/// no drawing.
fn parse_sheet_images(entries: &[Entry], sheet_path: &str) -> Vec<ParsedImage> {
    let rels = parse_rel_list(entries, &rels_path_of(sheet_path));
    let Some(drawing) = rels.iter().find(|r| r.typ.ends_with("/drawing")) else {
        return Vec::new();
    };
    let drawing_path = resolve(sheet_path, &drawing.target);
    let Some(bytes) = part(entries, &drawing_path) else {
        return Vec::new();
    };
    let drawing_rels = parse_rel_list(entries, &rels_path_of(&drawing_path));
    parse_drawing(&xml_of(bytes), entries, &drawing_path, &drawing_rels)
}

/// Mutable state while walking a drawing part's anchors.
#[derive(Default)]
struct Anchor {
    two: bool,
    active: bool,
    in_from: bool,
    in_to: bool,
    /// 0 = none, 1 = col, 2 = row (which child of the current marker).
    field: u8,
    from: (u32, u32),
    to: (u32, u32),
    cx: u64,
    cy: u64,
    embed: String,
}

/// Walk a `drawingN.xml` part into [`ParsedImage`]s.
fn parse_drawing(
    xml: &str,
    entries: &[Entry],
    drawing_path: &str,
    rels: &[Rel],
) -> Vec<ParsedImage> {
    let mut out = Vec::new();
    let mut a = Anchor::default();
    let mut r = Reader::new(xml);
    while let Some(ev) = r.read() {
        drawing_event(&mut out, &mut a, entries, drawing_path, rels, ev);
    }
    out
}

/// Fold one drawing event into the anchor state / output.
fn drawing_event(
    out: &mut Vec<ParsedImage>,
    a: &mut Anchor,
    entries: &[Entry],
    drawing_path: &str,
    rels: &[Rel],
    ev: Event,
) {
    match ev {
        Event::Open(tag) => open_drawing(a, &tag),
        Event::Text(t) => coord_text(a, &t),
        Event::Close(name) => close_drawing(out, a, entries, drawing_path, rels, name),
    }
}

/// Handle a drawing open tag: start anchors, enter markers, read ext / blip.
fn open_drawing(a: &mut Anchor, tag: &Tag) {
    match tag.name {
        "xdr:twoCellAnchor" => start(a, true),
        "xdr:oneCellAnchor" => start(a, false),
        "xdr:from" => a.in_from = true,
        "xdr:to" => a.in_to = true,
        "xdr:col" => a.field = 1,
        "xdr:row" => a.field = 2,
        "xdr:ext" => set_ext(a, tag),
        "a:blip" => a.embed = tag.attr("r:embed").unwrap_or("").to_string(),
        _ => {}
    }
}

/// Begin a fresh anchor of the given kind.
fn start(a: &mut Anchor, two: bool) {
    *a = Anchor {
        two,
        active: true,
        ..Anchor::default()
    };
}

/// Read a one-cell anchor's `<xdr:ext cx cy>` extent (EMU).
fn set_ext(a: &mut Anchor, tag: &Tag) {
    a.cx = attr_u64(tag, "cx");
    a.cy = attr_u64(tag, "cy");
}

/// Parse a numeric attribute, defaulting to 0.
fn attr_u64(tag: &Tag, key: &str) -> u64 {
    tag.attr(key).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Record a marker coordinate (`<xdr:col>`/`<xdr:row>` text) into from/to.
fn coord_text(a: &mut Anchor, t: &str) {
    let Ok(v) = t.trim().parse::<u32>() else {
        return;
    };
    let f = a.field;
    if a.in_from {
        set_field(&mut a.from, f, v);
    } else if a.in_to {
        set_field(&mut a.to, f, v);
    }
}

/// Set the col (field 1) or row (field 2) component of a cell coordinate.
fn set_field(cell: &mut (u32, u32), field: u8, v: u32) {
    match field {
        1 => cell.0 = v,
        2 => cell.1 = v,
        _ => {}
    }
}

/// Handle a drawing close tag: leave markers, finalize an anchor.
fn close_drawing(
    out: &mut Vec<ParsedImage>,
    a: &mut Anchor,
    entries: &[Entry],
    drawing_path: &str,
    rels: &[Rel],
    name: &str,
) {
    match name {
        "xdr:from" => a.in_from = false,
        "xdr:to" => a.in_to = false,
        "xdr:col" | "xdr:row" => a.field = 0,
        "xdr:twoCellAnchor" | "xdr:oneCellAnchor" => finalize(out, a, entries, drawing_path, rels),
        _ => {}
    }
}

/// Emit the current anchor as a [`ParsedImage`] when it resolves to media.
fn finalize(
    out: &mut Vec<ParsedImage>,
    a: &mut Anchor,
    entries: &[Entry],
    drawing_path: &str,
    rels: &[Rel],
) {
    if a.active {
        if let Some(img) = build_image(a, entries, drawing_path, rels) {
            out.push(img);
        }
    }
    a.active = false;
}

/// Resolve the current anchor's blip to media bytes + build the image.
fn build_image(
    a: &Anchor,
    entries: &[Entry],
    drawing_path: &str,
    rels: &[Rel],
) -> Option<ParsedImage> {
    let rel = rels.iter().find(|r| r.id == a.embed)?;
    let media_path = resolve(drawing_path, &rel.target);
    let bytes = part(entries, &media_path)?;
    Some(ParsedImage {
        data: crate::b64::encode(bytes),
        format: ext_of(&media_path),
        anchor: anchor_of(a),
    })
}

/// Build the parsed anchor (two-cell range or one-cell pixel size).
fn anchor_of(a: &Anchor) -> ParsedAnchor {
    if a.two {
        ParsedAnchor::TwoCell {
            from_col: a.from.0,
            from_row: a.from.1,
            to_col: a.to.0,
            to_row: a.to.1,
        }
    } else {
        ParsedAnchor::OneCell {
            col: a.from.0,
            row: a.from.1,
            width: emu_px(a.cx),
            height: emu_px(a.cy),
        }
    }
}

/// EMU → pixels (96 dpi); the inverse of the writer's `px * 9525`.
fn emu_px(emu: u64) -> u32 {
    (emu / 9525) as u32
}

/// The file extension of a media path (`xl/media/image1.png` → `png`).
fn ext_of(path: &str) -> String {
    path.rsplit('.').next().unwrap_or("png").to_string()
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
