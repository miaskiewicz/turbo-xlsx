//! Performance benchmarks for the two shapes the spec's targets call out:
//!   - 1,000 rows × 20 cols, styled, single `write` → target < 50 ms
//!   - 50,000 rows × 30 cols, streamed → target < 1.5 s, O(one row) retention
//!
//! Run with `cargo bench`. These measure the writer in isolation (model → bytes),
//! the same boundary `ReportExportService` calls.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use turbo_xlsx_core::{
    write, Cell, CellStyle, CurrencyFormat, Font, Row, Sheet, Workbook, WorkbookWriter,
    WriteOptions,
};

/// A currency cell in minor units with a locale-driven format.
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

/// One styled data row of `cols` cells: a label, then currency amounts.
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
        is_total: None,
        ..Default::default()
    }
}

/// A bold header row of `cols` cells.
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

/// Build an in-memory styled workbook of `rows` × `cols`.
fn build_workbook(rows: usize, cols: usize) -> Workbook {
    let mut sheet = Sheet {
        name: "Bench".into(),
        rows: Vec::with_capacity(rows + 1),
        ..Default::default()
    };
    sheet.rows.push(header_row(cols));
    for i in 0..rows {
        sheet.rows.push(data_row(i, cols));
    }
    Workbook {
        locale: Some("es-MX".into()),
        sheets: vec![sheet],
        ..Default::default()
    }
}

/// Batch `write`: 1,000 rows × 20 styled columns.
fn bench_batch(c: &mut Criterion) {
    let wb = build_workbook(1_000, 20);
    let opts = WriteOptions::default();
    c.bench_function("batch_1k_x20_styled", |b| {
        b.iter(|| {
            let result = write(black_box(&wb), &opts).unwrap();
            black_box(result.xlsx.len())
        })
    });
}

/// Streamed write: 50,000 rows × 30 columns, pushed row by row.
fn bench_stream(c: &mut Criterion) {
    let mut group = c.benchmark_group("stream_50k_x30");
    group.sample_size(10);
    group.bench_function("stream", |b| b.iter(|| black_box(stream_rows(50_000, 30))));
    group.finish();
}

/// Stream `rows` × `cols` through the streaming writer and return the byte count.
fn stream_rows(rows: usize, cols: usize) -> usize {
    let mut w = WorkbookWriter::new(Some("es-MX".into()), WriteOptions::default());
    w.start_sheet(Sheet {
        name: "Bench".into(),
        ..Default::default()
    })
    .unwrap();
    w.write_row(&header_row(cols)).unwrap();
    for i in 0..rows {
        w.write_row(&data_row(i, cols)).unwrap();
    }
    w.end_sheet().unwrap();
    w.finish().unwrap().xlsx.len()
}

criterion_group!(benches, bench_batch, bench_stream);
criterion_main!(benches);
