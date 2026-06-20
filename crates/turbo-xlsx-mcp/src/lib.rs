//! turbo-xlsx MCP server — the protocol surface.
//!
//! A hand-rolled **JSON-RPC 2.0** server (no SDK) exposing the turbo-xlsx Excel
//! utilities as MCP tools over stdio. Mirrors the turbo-surf-mcp layout: this
//! `lib` is the testable protocol core ([`handle`]); `main.rs` is the stdio pump.
//!
//! Tools (all converge on `turbo-xlsx-core`):
//!   - `write`         — a workbook object → `.xlsx` (file `out` or base64)
//!   - `write_rows`    — `{ sheetName?, columns?, rows }` fast-path → `.xlsx`
//!   - `convert_csv`   — CSV text/file → `.xlsx` (numbers inferred)
//!   - `parse`         — `.xlsx` → JSON (grid or typed) / CSV / Markdown
//!   - `inspect`       — `.xlsx` → per-sheet name + row/col dimensions
//!   - `read_range`    — `.xlsx` → values for a sheet or an `A1:C3` range
//!
//! Binary I/O is path-or-base64: every reader takes `path` OR `dataBase64`, every
//! writer takes an optional `out` path (returns `{ path, bytes }`) and otherwise
//! returns `{ base64, bytes }`. Base64 is hand-rolled to keep the dep set at
//! `serde`/`serde_json` (plus the core) — no transitive surprises.

#![forbid(unsafe_code)]

use serde_json::{json, Value};

use turbo_xlsx_core as core;

/// Per-connection state. Currently stateless — the tools are pure utilities —
/// but kept as a handle so future stateful tools (an incremental builder) slot
/// in without changing [`handle`]'s signature, exactly like turbo-surf's session.
#[derive(Default)]
pub struct Session;

impl Session {
    /// A fresh session.
    pub fn new() -> Self {
        Session
    }
}

/// Dispatch one JSON-RPC request. Returns the response value to write back, or
/// `None` for a notification (a message with no `id`, which must get no reply).
pub fn handle(session: &mut Session, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned()?;
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => Some(ok(id, initialize_result(session))),
        "tools/list" => Some(ok(id, json!({ "tools": tools() }))),
        "tools/call" => Some(tools_call(id, req.get("params"))),
        other => Some(err(id, &format!("unknown method: {other}"))),
    }
}

/// The `initialize` result: protocol version, tool capability, server identity.
fn initialize_result(_session: &mut Session) -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "turbo-xlsx-mcp", "version": env!("CARGO_PKG_VERSION") }
    })
}

// ---- tools/call dispatch ----------------------------------------------------

/// Route a `tools/call`: validate params, run the tool, wrap success or the
/// error string in the MCP content envelope (a tool error is a normal result
/// with `isError: true`, not a JSON-RPC protocol error).
fn tools_call(id: Value, params: Option<&Value>) -> Value {
    let params = match params {
        Some(p) => p,
        None => return err(id, "tools/call: missing params"),
    };
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let empty = json!({});
    let args = params.get("arguments").unwrap_or(&empty);
    match call_tool(name, args) {
        Ok(result) => ok(id, tool_content(result)),
        Err(message) => ok(id, tool_error(&message)),
    }
}

/// Dispatch by tool name. Split into write/read halves purely for readability.
fn call_tool(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "write" => tool_write(args),
        "write_rows" => tool_write_rows(args),
        "convert_csv" => tool_convert_csv(args),
        _ => read_tool(name, args),
    }
}

/// The read/inspect tools (the parse side).
fn read_tool(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "parse" => tool_parse(args),
        "inspect" => tool_inspect(args),
        "read_range" => tool_read_range(args),
        other => Err(format!("unknown tool: {other}")),
    }
}

// ---- write tools ------------------------------------------------------------

/// `write`: a full workbook object → `.xlsx`.
fn tool_write(args: &Value) -> Result<Value, String> {
    let workbook = args
        .get("workbook")
        .cloned()
        .ok_or("write: missing 'workbook'")?;
    let result =
        core::write_from_json_value(workbook, &core::WriteOptions::default()).map_err(stringify)?;
    emit(args, &result.xlsx)
}

/// `write_rows`: the typed columns + rows fast-path → `.xlsx`.
fn tool_write_rows(args: &Value) -> Result<Value, String> {
    let input = args.get("input").ok_or("write_rows: missing 'input'")?;
    let sheet_name = input
        .get("sheetName")
        .and_then(Value::as_str)
        .map(String::from);
    let locale = input
        .get("locale")
        .and_then(Value::as_str)
        .map(String::from);
    let columns = field_vec(input, "columns")?;
    let rows = field_vec(input, "rows")?;
    let result = core::write_rows(
        sheet_name,
        columns,
        rows,
        locale,
        &core::WriteOptions::default(),
    )
    .map_err(stringify)?;
    emit(args, &result.xlsx)
}

/// `convert_csv`: CSV text (`csv`) or a CSV file (`path`/`dataBase64`) → `.xlsx`.
/// Numbers are inferred (leading-zero strings like ZIP codes stay text).
fn tool_convert_csv(args: &Value) -> Result<Value, String> {
    let text = csv_text(args)?;
    let name = args
        .get("sheetName")
        .and_then(Value::as_str)
        .unwrap_or("Sheet1");
    let workbook = csv_to_workbook(&text, name);
    let result =
        core::write_from_json_value(workbook, &core::WriteOptions::default()).map_err(stringify)?;
    emit(args, &result.xlsx)
}

/// Deserialize an optional array field (`columns`/`rows`) into core types.
fn field_vec<T: serde::de::DeserializeOwned>(v: &Value, key: &str) -> Result<Vec<T>, String> {
    match v.get(key) {
        Some(value) => serde_json::from_value(value.clone()).map_err(stringify),
        None => Ok(Vec::new()),
    }
}

/// The CSV body: inline `csv`, else the decoded `path`/`dataBase64` bytes as UTF-8.
fn csv_text(args: &Value) -> Result<String, String> {
    if let Some(csv) = args.get("csv").and_then(Value::as_str) {
        return Ok(csv.to_string());
    }
    let bytes = input_bytes(args)?;
    String::from_utf8(bytes).map_err(|e| format!("csv is not utf-8: {e}"))
}

// ---- read tools -------------------------------------------------------------

/// `parse`: `.xlsx` → JSON grid / typed JSON / CSV / Markdown text.
fn tool_parse(args: &Value) -> Result<Value, String> {
    let bytes = input_bytes(args)?;
    let wb = core::parse::parse(&bytes).map_err(stringify)?;
    let text = render_parsed(&wb, args)?;
    Ok(json!({ "text": text }))
}

/// Serialize a parsed workbook to the requested `format` (default `json`).
fn render_parsed(wb: &core::parse::ParsedWorkbook, args: &Value) -> Result<String, String> {
    use core::parse::serialize;
    let format = args.get("format").and_then(Value::as_str).unwrap_or("json");
    let typed = args.get("typed").and_then(Value::as_bool).unwrap_or(false);
    match format {
        "json" if typed => Ok(serialize::to_json_typed(wb)),
        "json" => Ok(serialize::to_json_grid(wb)),
        "csv" => Ok(serialize::to_csv(pick_sheet(wb, args)?)),
        "md" | "markdown" => Ok(serialize::to_markdown(pick_sheet(wb, args)?)),
        other => Err(format!("unknown format: {other:?}")),
    }
}

/// `inspect`: per-sheet name + row/column dimensions.
fn tool_inspect(args: &Value) -> Result<Value, String> {
    let bytes = input_bytes(args)?;
    let wb = core::parse::parse(&bytes).map_err(stringify)?;
    let sheets: Vec<Value> = wb.sheets.iter().map(sheet_dims).collect();
    Ok(json!({ "sheets": sheets }))
}

/// One sheet's `{ name, rows, cols }` (cols = the widest row).
fn sheet_dims(s: &core::parse::ParsedSheet) -> Value {
    let cols = s.rows.iter().map(Vec::len).max().unwrap_or(0);
    json!({ "name": s.name, "rows": s.rows.len(), "cols": cols })
}

/// `read_range`: a sheet's whole value grid, or just an `A1:C3` window.
fn tool_read_range(args: &Value) -> Result<Value, String> {
    let bytes = input_bytes(args)?;
    let wb = core::parse::parse(&bytes).map_err(stringify)?;
    let sheet = pick_sheet(&wb, args)?;
    let grid = range_grid(sheet, args)?;
    Ok(json!({ "name": sheet.name, "values": grid }))
}

/// The whole-sheet grid, or just the `range` window when one is given.
fn range_grid(sheet: &core::parse::ParsedSheet, args: &Value) -> Result<Vec<Value>, String> {
    match args.get("range").and_then(Value::as_str) {
        Some(spec) => slice_range(sheet, spec),
        None => Ok(full_grid(sheet)),
    }
}

/// Every row of the sheet as JSON value arrays.
fn full_grid(sheet: &core::parse::ParsedSheet) -> Vec<Value> {
    sheet
        .rows
        .iter()
        .map(|r| Value::Array(r.iter().map(cell_to_json).collect()))
        .collect()
}

/// The `r0..=r1 × c0..=c1` window of an `A1:C3` spec, padding gaps with null.
fn slice_range(sheet: &core::parse::ParsedSheet, spec: &str) -> Result<Vec<Value>, String> {
    let (r0, c0, r1, c1) = parse_a1_range(spec)?;
    let mut out = Vec::new();
    for r in r0..=r1 {
        out.push(slice_row(sheet, r, c0, c1));
    }
    Ok(out)
}

/// One row of the window.
fn slice_row(sheet: &core::parse::ParsedSheet, r: usize, c0: usize, c1: usize) -> Value {
    let mut cells = Vec::new();
    for c in c0..=c1 {
        cells.push(cell_at(sheet, r, c));
    }
    Value::Array(cells)
}

/// The cell at `(r, c)` as JSON, or null if past the sheet's extent.
fn cell_at(sheet: &core::parse::ParsedSheet, r: usize, c: usize) -> Value {
    sheet
        .rows
        .get(r)
        .and_then(|row| row.get(c))
        .map(cell_to_json)
        .unwrap_or(Value::Null)
}

/// A parsed cell value → JSON scalar (empty → null).
fn cell_to_json(cv: &core::parse::CellValue) -> Value {
    use core::parse::CellValue;
    match cv {
        CellValue::Empty => Value::Null,
        CellValue::Text(s) => json!(s),
        CellValue::Number(n) => json!(n),
        CellValue::Bool(b) => json!(b),
        CellValue::Date(s) => json!(s),
    }
}

/// Pick a sheet by `sheet` name, defaulting to the first.
fn pick_sheet<'a>(
    wb: &'a core::parse::ParsedWorkbook,
    args: &Value,
) -> Result<&'a core::parse::ParsedSheet, String> {
    match args.get("sheet").and_then(Value::as_str) {
        Some(name) => wb
            .sheets
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("no sheet named {name:?}")),
        None => wb
            .sheets
            .first()
            .ok_or_else(|| "workbook has no sheets".to_string()),
    }
}

// ---- CSV → workbook ---------------------------------------------------------

/// Build a one-sheet workbook value from CSV text.
fn csv_to_workbook(text: &str, name: &str) -> Value {
    let rows: Vec<Value> = parse_csv(text).into_iter().map(csv_row).collect();
    json!({ "sheets": [ { "name": name, "rows": rows } ] })
}

/// One CSV record → a `{ cells: [...] }` row.
fn csv_row(fields: Vec<String>) -> Value {
    let cells: Vec<Value> = fields.into_iter().map(csv_cell).collect();
    json!({ "cells": cells })
}

/// One CSV field → a typed cell (number when it looks numeric, else string).
fn csv_cell(field: String) -> Value {
    if looks_numeric(&field) {
        json!({ "type": "number", "value": field.trim().parse::<f64>().unwrap_or(0.0) })
    } else {
        json!({ "type": "string", "value": field })
    }
}

/// Treat a field as a number only when it parses AND has no leading-zero run
/// (so `"007"`/`"01"` stay text — ZIP codes, ids — while `"0"`/`"0.5"` are kept).
fn looks_numeric(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    if t.len() > 1 && t.starts_with('0') && t.as_bytes()[1].is_ascii_digit() {
        return false;
    }
    t.parse::<f64>().is_ok()
}

/// Parse CSV (RFC-4180-ish: quoted fields, `""` escapes, embedded newlines).
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut st = CsvState::default();
    let mut i = 0;
    while i < chars.len() {
        i = csv_consume(&chars, i, &mut st);
    }
    st.finish()
}

/// The CSV scanner's running state.
#[derive(Default)]
struct CsvState {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    field: String,
    in_quotes: bool,
}

impl CsvState {
    /// Close the current field.
    fn end_field(&mut self) {
        let f = std::mem::take(&mut self.field);
        self.row.push(f);
    }

    /// Close the current row (closing its trailing field first).
    fn end_row(&mut self) {
        self.end_field();
        let r = std::mem::take(&mut self.row);
        self.rows.push(r);
    }

    /// Flush any pending field/row at end of input.
    fn finish(mut self) -> Vec<Vec<String>> {
        if !self.field.is_empty() || !self.row.is_empty() {
            self.end_row();
        }
        self.rows
    }
}

/// Consume one character, returning the next index.
fn csv_consume(chars: &[char], i: usize, st: &mut CsvState) -> usize {
    let c = chars[i];
    if st.in_quotes {
        return consume_quoted(chars, i, c, st);
    }
    consume_unquoted(i, c, st)
}

/// Inside quotes: `""` is a literal quote, a lone `"` closes the field.
fn consume_quoted(chars: &[char], i: usize, c: char, st: &mut CsvState) -> usize {
    if c == '"' {
        if chars.get(i + 1) == Some(&'"') {
            st.field.push('"');
            return i + 2;
        }
        st.in_quotes = false;
        return i + 1;
    }
    st.field.push(c);
    i + 1
}

/// Outside quotes: `"` opens, `,` ends a field, `\n` ends a row, `\r` is dropped.
fn consume_unquoted(i: usize, c: char, st: &mut CsvState) -> usize {
    match c {
        '"' => st.in_quotes = true,
        ',' => st.end_field(),
        '\n' => st.end_row(),
        '\r' => {}
        _ => st.field.push(c),
    }
    i + 1
}

// ---- A1 range parsing -------------------------------------------------------

/// `"A1:C3"` → 0-based `(r0, c0, r1, c1)`, normalized so r0≤r1 / c0≤c1.
fn parse_a1_range(spec: &str) -> Result<(usize, usize, usize, usize), String> {
    let (start, end) = spec.split_once(':').ok_or("range must look like A1:C3")?;
    let (r0, c0) = parse_a1(start)?;
    let (r1, c1) = parse_a1(end)?;
    Ok((r0.min(r1), c0.min(c1), r0.max(r1), c0.max(c1)))
}

/// `"B7"` → 0-based `(row, col)`.
fn parse_a1(cell: &str) -> Result<(usize, usize), String> {
    let split = cell
        .find(|c: char| c.is_ascii_digit())
        .ok_or("cell needs a row number")?;
    let (col, row) = cell.split_at(split);
    let col_idx = col_to_index(col)?;
    let row_num: usize = row.parse().map_err(|_| "bad row number".to_string())?;
    if row_num == 0 {
        return Err("rows are 1-based".into());
    }
    Ok((row_num - 1, col_idx))
}

/// `"AB"` → 0-based column index (base-26 bijective).
fn col_to_index(col: &str) -> Result<usize, String> {
    if col.is_empty() {
        return Err("missing column letters".into());
    }
    let mut idx = 0usize;
    for ch in col.chars() {
        idx = idx * 26 + letter_value(ch)?;
    }
    Ok(idx - 1)
}

/// A single column letter → its 1-based value (`A`→1).
fn letter_value(ch: char) -> Result<usize, String> {
    if ch.is_ascii_uppercase() {
        return Ok((ch as usize) - ('A' as usize) + 1);
    }
    if ch.is_ascii_lowercase() {
        return Ok((ch as usize) - ('a' as usize) + 1);
    }
    Err(format!("not a column letter: {ch:?}"))
}

// ---- envelopes --------------------------------------------------------------

/// Either write the bytes to `out` (→ `{ path, bytes }`) or return base64.
fn emit(args: &Value, bytes: &[u8]) -> Result<Value, String> {
    match args.get("out").and_then(Value::as_str) {
        Some(path) => {
            std::fs::write(path, bytes).map_err(|e| format!("write {path}: {e}"))?;
            Ok(json!({ "path": path, "bytes": bytes.len() }))
        }
        None => Ok(json!({ "base64": b64_encode(bytes), "bytes": bytes.len() })),
    }
}

/// Read an input's bytes: `path` (a file) or `dataBase64` (inline).
fn input_bytes(args: &Value) -> Result<Vec<u8>, String> {
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        return std::fs::read(path).map_err(|e| format!("read {path}: {e}"));
    }
    if let Some(b64) = args.get("dataBase64").and_then(Value::as_str) {
        return Ok(b64_decode(b64));
    }
    Err("provide 'path' or 'dataBase64'".into())
}

/// The MCP success envelope: the result as pretty JSON in a single text block.
fn tool_content(result: Value) -> Value {
    let text = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
    json!({ "content": [ { "type": "text", "text": text } ], "isError": false })
}

/// The MCP tool-error envelope.
fn tool_error(message: &str) -> Value {
    json!({ "content": [ { "type": "text", "text": message } ], "isError": true })
}

/// A successful JSON-RPC response.
fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC method error (used only for protocol-level faults).
fn err(id: Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } })
}

/// Render any `Display` error as a `String` (the tools' uniform error channel).
fn stringify(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---- base64 (hand-rolled, std-only) -----------------------------------------

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding.
fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(B64_ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(b64_tail(chunk.len() > 1, (n >> 6) as usize & 63));
        out.push(b64_tail(chunk.len() > 2, n as usize & 63));
    }
    out
}

/// A tail base64 char, or `=` padding when the source byte is absent.
fn b64_tail(present: bool, idx: usize) -> char {
    if present {
        B64_ALPHABET[idx] as char
    } else {
        '='
    }
}

/// Lenient base64 decode: skips padding/whitespace and any non-alphabet byte.
fn b64_decode(s: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut nbits = 0u32;
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for &c in s.as_bytes() {
        let Some(v) = b64_val(c) else { continue };
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    out
}

/// A base64 alphabet byte → its 6-bit value, or `None` for non-alphabet bytes.
fn b64_val(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

// ---- tool catalog -----------------------------------------------------------

/// The advertised tool list (name + description + JSON-Schema input shape).
fn tools() -> Vec<Value> {
    vec![
        tool_schema(
            "write",
            "Write a full workbook object to .xlsx. Provide 'workbook' (the turbo-xlsx \
             workbook shape). Returns base64, or { path, bytes } when 'out' is set.",
            json!({
                "type": "object",
                "properties": {
                    "workbook": { "type": "object", "description": "Workbook: { sheets: [...] }" },
                    "out": { "type": "string", "description": "Optional output file path" }
                },
                "required": ["workbook"]
            }),
        ),
        tool_schema(
            "write_rows",
            "Fast-path: one sheet from typed columns + rows. 'input' is \
             { sheetName?, locale?, columns?, rows }. Returns base64 or { path, bytes }.",
            json!({
                "type": "object",
                "properties": {
                    "input": { "type": "object" },
                    "out": { "type": "string" }
                },
                "required": ["input"]
            }),
        ),
        tool_schema(
            "convert_csv",
            "Convert CSV to .xlsx. Provide 'csv' (text) or 'path'/'dataBase64' (a CSV file). \
             Numbers are inferred; leading-zero strings stay text. 'sheetName' optional.",
            json!({
                "type": "object",
                "properties": {
                    "csv": { "type": "string" },
                    "path": { "type": "string" },
                    "dataBase64": { "type": "string" },
                    "sheetName": { "type": "string" },
                    "out": { "type": "string" }
                }
            }),
        ),
        tool_schema(
            "parse",
            "Read an .xlsx into JSON (grid or typed), CSV, or Markdown. Provide 'path' or \
             'dataBase64'. 'format' = json|csv|md (default json); 'typed' for round-trippable \
             JSON; 'sheet' selects a sheet for csv/md.",
            read_input_schema(json!({
                "format": { "type": "string", "enum": ["json", "csv", "md", "markdown"] },
                "typed": { "type": "boolean" },
                "sheet": { "type": "string" }
            })),
        ),
        tool_schema(
            "inspect",
            "Report each sheet's name and row/column dimensions for an .xlsx. \
             Provide 'path' or 'dataBase64'.",
            read_input_schema(json!({})),
        ),
        tool_schema(
            "read_range",
            "Read a sheet's values, or just an A1:C3 'range'. Provide 'path' or 'dataBase64'; \
             'sheet' selects a sheet (default the first). Returns a value grid (null for gaps).",
            read_input_schema(json!({
                "sheet": { "type": "string" },
                "range": { "type": "string", "description": "e.g. \"A1:C10\"" }
            })),
        ),
    ]
}

/// One tool descriptor.
fn tool_schema(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

/// An input schema for a reader: `path`/`dataBase64` plus the tool's own props.
fn read_input_schema(extra: Value) -> Value {
    let mut props = json!({
        "path": { "type": "string", "description": "Path to an .xlsx file" },
        "dataBase64": { "type": "string", "description": "Inline .xlsx bytes, base64" }
    });
    merge_objects(props.as_object_mut(), extra);
    json!({ "type": "object", "properties": props })
}

/// Merge the `extra` object's keys into `target` (best-effort; non-objects ignored).
fn merge_objects(target: Option<&mut serde_json::Map<String, Value>>, extra: Value) {
    if let (Some(map), Value::Object(items)) = (target, extra) {
        for (k, v) in items {
            map.insert(k, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Value) -> Value {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": args } });
        let mut s = Session::new();
        handle(&mut s, &req).expect("a response")
    }

    fn tool_text(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn xlsx_b64() -> String {
        let wb = json!({ "sheets": [ { "name": "S", "rows": [
            { "cells": [ { "type": "string", "value": "a" }, { "type": "number", "value": 3.5 } ] },
            { "cells": [ { "type": "boolean", "value": true } ] }
        ] } ] });
        let resp = call("write", json!({ "workbook": wb }));
        let text = tool_text(&resp);
        serde_json::from_str::<Value>(&text).unwrap()["base64"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn initialize_advertises_tools() {
        let mut s = Session::new();
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let resp = handle(&mut s, &req).unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "turbo-xlsx-mcp");
        let list = handle(
            &mut s,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"write") && names.contains(&"parse") && names.contains(&"read_range")
        );
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut s = Session::new();
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle(&mut s, &note).is_none());
    }

    #[test]
    fn unknown_method_and_tool_are_reported() {
        let mut s = Session::new();
        let bad = handle(
            &mut s,
            &json!({ "jsonrpc": "2.0", "id": 9, "method": "frobnicate" }),
        )
        .unwrap();
        assert!(bad["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown method"));
        let resp = call("frobnicate", json!({}));
        assert_eq!(resp["result"]["isError"], true);
    }

    #[test]
    fn write_then_parse_round_trips() {
        let b64 = xlsx_b64();
        let resp = call("parse", json!({ "dataBase64": b64, "format": "csv" }));
        let inner: Value = serde_json::from_str(tool_text(&resp).as_str()).unwrap();
        assert!(inner["text"].as_str().unwrap().starts_with("a,3.5"));
    }

    #[test]
    fn parse_typed_and_grid_shapes() {
        let b64 = xlsx_b64();
        let grid = serde_json::from_str::<Value>(&tool_text(&call(
            "parse",
            json!({ "dataBase64": &b64 }),
        )))
        .unwrap();
        let inner: Value = serde_json::from_str(grid["text"].as_str().unwrap()).unwrap();
        assert_eq!(inner["sheets"][0]["rows"][0][0], "a");
        let typed = call("parse", json!({ "dataBase64": &b64, "typed": true }));
        assert!(tool_text(&typed).contains("schemaVersion"));
    }

    #[test]
    fn inspect_reports_dimensions() {
        let resp = call("inspect", json!({ "dataBase64": xlsx_b64() }));
        let v: Value = serde_json::from_str(tool_text(&resp).as_str()).unwrap();
        assert_eq!(v["sheets"][0]["name"], "S");
        assert_eq!(v["sheets"][0]["cols"], 2);
    }

    #[test]
    fn read_range_windows_and_pads() {
        let b64 = xlsx_b64();
        let resp = call(
            "read_range",
            json!({ "dataBase64": &b64, "range": "A1:B2" }),
        );
        let v: Value = serde_json::from_str(tool_text(&resp).as_str()).unwrap();
        assert_eq!(v["values"][0][0], "a");
        assert_eq!(v["values"][0][1], 3.5);
        assert_eq!(v["values"][1][1], Value::Null); // row 2 has one cell -> B2 is null
    }

    #[test]
    fn convert_csv_infers_numbers_and_keeps_zip_codes() {
        let resp = call(
            "convert_csv",
            json!({ "csv": "name,zip,amount\n\"Smith, J\",007,1200\n" }),
        );
        let b64 = serde_json::from_str::<Value>(tool_text(&resp).as_str()).unwrap()["base64"]
            .as_str()
            .unwrap()
            .to_string();
        let parsed = call("parse", json!({ "dataBase64": b64 }));
        let grid: Value = serde_json::from_str(
            serde_json::from_str::<Value>(tool_text(&parsed).as_str()).unwrap()["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let row = &grid["sheets"][0]["rows"][1];
        assert_eq!(row[0], "Smith, J"); // quoted comma preserved
        assert_eq!(row[1], "007"); // leading zero kept as text
        assert_eq!(row[2], 1200.0); // plain integer inferred numeric
    }

    #[test]
    fn base64_round_trips_arbitrary_bytes() {
        let data: Vec<u8> = (0u8..=255).collect();
        assert_eq!(b64_decode(&b64_encode(&data)), data);
        assert_eq!(b64_decode(&b64_encode(b"M")), b"M"); // 1-byte (double pad)
        assert_eq!(b64_decode(&b64_encode(b"Ma")), b"Ma"); // 2-byte (single pad)
    }

    #[test]
    fn missing_inputs_error_cleanly() {
        assert_eq!(
            call("write", json!({})).pointer("/result/isError"),
            Some(&json!(true))
        );
        assert_eq!(
            call("parse", json!({})).pointer("/result/isError"),
            Some(&json!(true))
        );
        let bad_range = call(
            "read_range",
            json!({ "dataBase64": xlsx_b64(), "range": "oops" }),
        );
        assert_eq!(bad_range["result"]["isError"], true);
    }
}
