//! Serialize a [`ParsedWorkbook`] to the output formats parse mode offers:
//! a values-grid JSON, the round-trippable typed turbo-xlsx workbook JSON, CSV
//! (RFC-4180 quoting), and a GitHub-flavoured Markdown table. CSV/Markdown render
//! one sheet (selectable); JSON returns every sheet.

use serde_json::{json, Value};

use super::read::{CellValue, ParsedAnchor, ParsedImage, ParsedSheet, ParsedWorkbook};

/// `{ "sheets": [ { "name", "rows": [[scalar, ...]], "images": [...] } ] }`.
pub fn to_json_grid(wb: &ParsedWorkbook) -> String {
    let sheets: Vec<Value> = wb
        .sheets
        .iter()
        .map(|s| json!({ "name": s.name, "rows": grid_rows(s), "images": images_json(s) }))
        .collect();
    serde_json::to_string(&json!({ "sheets": sheets })).unwrap_or_default()
}

/// The sheet's images as model-shaped JSON objects (omitted-empty by the caller's
/// consumer; always an array here). Round-trippable straight into `writeFromJson`.
fn images_json(sheet: &ParsedSheet) -> Vec<Value> {
    sheet.images.iter().map(image_to_json).collect()
}

/// One parsed image as a `{ data, format, anchor }` model object.
fn image_to_json(img: &ParsedImage) -> Value {
    json!({ "data": img.data, "format": img.format, "anchor": anchor_to_json(&img.anchor) })
}

/// One anchor as its tagged `{ kind, ... }` model object.
fn anchor_to_json(anchor: &ParsedAnchor) -> Value {
    match anchor {
        ParsedAnchor::TwoCell {
            from_col,
            from_row,
            to_col,
            to_row,
        } => json!({
            "kind": "twoCell",
            "from": { "col": from_col, "row": from_row },
            "to": { "col": to_col, "row": to_row },
        }),
        ParsedAnchor::OneCell {
            col,
            row,
            width,
            height,
        } => json!({
            "kind": "oneCell",
            "at": { "col": col, "row": row },
            "width": width,
            "height": height,
        }),
    }
}

/// The value-array rows of one sheet.
fn grid_rows(sheet: &ParsedSheet) -> Vec<Value> {
    sheet
        .rows
        .iter()
        .map(|row| Value::Array(row.iter().map(cell_to_json).collect()))
        .collect()
}

/// One cell as a JSON scalar (null / string / number / bool).
fn cell_to_json(cell: &CellValue) -> Value {
    match cell {
        CellValue::Empty => Value::Null,
        CellValue::Text(s) | CellValue::Date(s) => json!(s),
        CellValue::Number(n) => json!(n),
        CellValue::Bool(b) => json!(b),
    }
}

/// The typed turbo-xlsx workbook JSON — round-trippable back into `writeFromJson`.
pub fn to_json_typed(wb: &ParsedWorkbook) -> String {
    let sheets: Vec<Value> = wb
        .sheets
        .iter()
        .map(|s| json!({ "name": s.name, "rows": typed_rows(s), "images": images_json(s) }))
        .collect();
    serde_json::to_string(&json!({ "schemaVersion": "1.0", "sheets": sheets })).unwrap_or_default()
}

/// The `{ "cells": [...] }` rows of one sheet in the typed model.
fn typed_rows(sheet: &ParsedSheet) -> Vec<Value> {
    sheet
        .rows
        .iter()
        .map(|row| json!({ "cells": row.iter().map(cell_to_typed).collect::<Vec<_>>() }))
        .collect()
}

/// One cell as a typed turbo-xlsx cell object.
fn cell_to_typed(cell: &CellValue) -> Value {
    match cell {
        CellValue::Empty => json!({ "type": "blank" }),
        CellValue::Text(s) => json!({ "type": "string", "value": s }),
        CellValue::Number(n) => json!({ "type": "number", "value": n }),
        CellValue::Bool(b) => json!({ "type": "boolean", "value": b }),
        CellValue::Date(s) => json!({ "type": "date", "value": s }),
    }
}

/// One sheet as CSV (RFC-4180: quote fields with `,` `"` or newlines).
pub fn to_csv(sheet: &ParsedSheet) -> String {
    let mut out = String::new();
    for row in &sheet.rows {
        let fields: Vec<String> = row.iter().map(csv_field).collect();
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

/// Quote a CSV field when it contains a delimiter, quote, or newline.
fn csv_field(cell: &CellValue) -> String {
    let text = cell_text(cell);
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// One sheet as a GitHub-flavoured Markdown table (first row = header).
pub fn to_markdown(sheet: &ParsedSheet) -> String {
    if sheet.rows.is_empty() {
        return String::new();
    }
    let cols = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = String::new();
    md_row(&mut out, &sheet.rows[0], cols);
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in &sheet.rows[1..] {
        md_row(&mut out, row, cols);
    }
    out
}

/// Append one Markdown table row, padded to `cols` cells.
fn md_row(out: &mut String, row: &[CellValue], cols: usize) {
    out.push('|');
    for i in 0..cols {
        let text = row.get(i).map(cell_text).unwrap_or_default();
        out.push(' ');
        out.push_str(&text.replace('|', "\\|").replace('\n', " "));
        out.push_str(" |");
    }
    out.push('\n');
}

/// A cell's plain-text rendering (shared by CSV + Markdown).
fn cell_text(cell: &CellValue) -> String {
    match cell {
        CellValue::Empty => String::new(),
        CellValue::Text(s) | CellValue::Date(s) => s.clone(),
        CellValue::Number(n) => format!("{n}"),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
    }
}
