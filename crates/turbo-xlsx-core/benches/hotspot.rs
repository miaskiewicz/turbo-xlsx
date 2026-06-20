//! Phase-by-phase hotspot profiler. Splits the 50k-row write into its stages so
//! we can see where the time actually goes before optimizing:
//!   - model build (constructing the typed Workbook)
//!   - worksheet emit (style interning + per-cell XML — the hot inner loop)
//!   - crc32 (the zip per-entry checksum over the emitted bytes)
//!   - package (styles + worksheets + zip assembly)
//!   - full write (end to end)
//!
//! Run: `cargo bench --features bench-internals --bench hotspot`.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use turbo_xlsx_core::internals::{crc32, package as package_phase, write_sheet, StyleTable};
use turbo_xlsx_core::{
    package, Cell, CellStyle, CurrencyFormat, Diagnostics, Font, Row, Sheet, Workbook, WriteOptions,
};

const ROWS: usize = 50_000;
const COLS: usize = 30;

fn currency(value: i64) -> Cell {
    Cell::Currency {
        value,
        currency: CurrencyFormat {
            code: "MXN".into(),
            locale: Some("es-MX".into()),
            decimals: None,
            negative: None,
            symbol: None,
        },
        style: None,
    }
}

fn data_row(i: usize, cols: usize) -> Row {
    let mut cells = Vec::with_capacity(cols);
    cells.push(Cell::String {
        value: format!("Empleado {i}"),
        style: None,
    });
    for c in 1..cols {
        cells.push(currency((i * c) as i64 * 100));
    }
    Row {
        cells,
        ..Default::default()
    }
}

fn header_row(cols: usize) -> Row {
    let style = CellStyle {
        font: Some(Font {
            bold: Some(true),
            ..Default::default()
        }),
        fill: Some("#dddddd".into()),
        ..Default::default()
    };
    let cells = (0..cols)
        .map(|c| Cell::String {
            value: format!("Col {c}"),
            style: Some(style.clone()),
        })
        .collect();
    Row {
        cells,
        ..Default::default()
    }
}

fn build_sheet(rows: usize, cols: usize) -> Sheet {
    let mut sheet = Sheet {
        name: "Bench".into(),
        rows: Vec::with_capacity(rows + 1),
        ..Default::default()
    };
    sheet.rows.push(header_row(cols));
    for i in 0..rows {
        sheet.rows.push(data_row(i, cols));
    }
    sheet
}

fn workbook(sheet: Sheet) -> Workbook {
    Workbook {
        locale: Some("es-MX".into()),
        sheets: vec![sheet],
        ..Default::default()
    }
}

fn hotspots(c: &mut Criterion) {
    let mut group = c.benchmark_group("hotspot_50k_x30");
    group.sample_size(10);

    group.bench_function("1_model_build", |b| {
        b.iter(|| black_box(build_sheet(ROWS, COLS)))
    });

    let sheet = build_sheet(ROWS, COLS);
    group.bench_function("2_worksheet_emit", |b| {
        b.iter(|| {
            let mut table = StyleTable::new();
            let mut diags = Diagnostics::default();
            black_box(
                write_sheet(&sheet, "es-MX", &mut table, &mut diags)
                    .unwrap()
                    .len(),
            )
        })
    });

    // The emitted worksheet bytes, used to isolate the crc32 cost.
    let xml = {
        let mut table = StyleTable::new();
        let mut diags = Diagnostics::default();
        write_sheet(&sheet, "es-MX", &mut table, &mut diags).unwrap()
    };
    let bytes = xml.into_bytes();
    group.bench_function("3_crc32_over_sheet", |b| {
        b.iter(|| black_box(crc32(black_box(&bytes))))
    });

    let wb = workbook(build_sheet(ROWS, COLS));
    group.bench_function("4_package", |b| {
        b.iter(|| {
            let mut diags = Diagnostics::default();
            black_box(
                package_phase(&wb, &WriteOptions::default(), &mut diags)
                    .unwrap()
                    .len(),
            )
        })
    });

    group.bench_function("5_full_write", |b| {
        b.iter(|| {
            black_box(
                turbo_xlsx_core::write(&wb, &WriteOptions::default())
                    .unwrap()
                    .xlsx
                    .len(),
            )
        })
    });

    group.finish();
}

criterion_group!(benches, hotspots);
criterion_main!(benches);
