//! Shared fixture + timing helpers for the native parse benches (`parse-native`
//! head-to-head, `parse-hotspot` phase profiler). Compiled into each bin via
//! `mod common;`.

use std::io::{Cursor, Read, Write};
use std::time::Instant;

use serde_json::json;
use turbo_xlsx_core::{parse, WriteOptions};

/// The single sheet every fixture carries.
pub const SHEET: &str = "Data";

/// A mixed-type grid (number / string / float / bool) as a turbo workbook,
/// written to STORED `.xlsx` bytes by turbo's own writer.
pub fn build_fixture(rows: usize) -> Vec<u8> {
    let mut grid = Vec::with_capacity(rows + 1);
    grid.push(json!({ "cells": [
        cell_str("id"), cell_str("label"), cell_str("amount"), cell_str("flag"),
    ] }));
    for i in 0..rows {
        grid.push(json!({ "cells": [
            json!({ "type": "number", "value": i }),
            cell_str(&format!("row-{i}")),
            json!({ "type": "number", "value": (i as f64) * 1.5 }),
            json!({ "type": "boolean", "value": i % 2 == 0 }),
        ] }));
    }
    let workbook = json!({ "sheets": [ { "name": SHEET, "rows": grid } ] });
    turbo_xlsx_core::write_from_json_value(workbook, &WriteOptions::default())
        .expect("write fixture")
        .xlsx
}

/// A string cell literal.
fn cell_str(s: &str) -> serde_json::Value {
    json!({ "type": "string", "value": s })
}

/// Re-pack a STORED OPC zip into a DEFLATE-compressed one (same parts), so the
/// readers face a realistic Excel-style compressed file (turbo's writer is STORED).
pub fn deflate_xlsx(stored: &[u8]) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(stored)).expect("read stored zip");
    let mut out = Vec::new();
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut out));
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).expect("zip entry");
            let name = file.name().to_string();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).expect("read entry");
            writer.start_file(name, opts).expect("start entry");
            writer.write_all(&buf).expect("write entry");
        }
        writer.finish().expect("finish zip");
    }
    out
}

/// turbo-xlsx-core: full parse to a `ParsedWorkbook`, counting every cell (the
/// parse already materializes the whole grid).
pub fn read_turbo(bytes: &[u8]) -> usize {
    let wb = parse::parse(bytes).expect("turbo parse");
    wb.sheets
        .iter()
        .flat_map(|s| s.rows.iter())
        .map(Vec::len)
        .sum()
}

/// calamine: open from the in-memory bytes and walk the worksheet range.
pub fn read_calamine(bytes: &[u8]) -> usize {
    use calamine::{Reader, Xlsx};
    let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).expect("calamine open");
    let range = wb.worksheet_range(SHEET).expect("calamine range");
    range.rows().map(<[_]>::len).sum()
}

/// Median wall-clock (ms) of `f` over `iters` runs.
pub fn median_ms(iters: usize, mut f: impl FnMut()) -> f64 {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t0 = Instant::now();
        f();
        samples.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}
