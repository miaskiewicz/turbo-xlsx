//! End-to-end tests of the public surface: the `write*` entry points, the
//! streaming writer, and the OPC package structure.

use turbo_xlsx_core::{
    write, write_from_json_str, write_from_json_value, write_rows, Cell, Column, ColumnData,
    CurrencyFormat, DocMeta, Row, Sheet, Workbook, WorkbookWriter, WriteOptions,
};

/// Assert the bytes are a ZIP (the OPC container) and contain a part name.
fn assert_xlsx(bytes: &[u8], must_contain: &[&str]) {
    assert_eq!(&bytes[0..2], b"PK", "not a zip");
    let text = String::from_utf8_lossy(bytes);
    for needle in must_contain {
        assert!(text.contains(needle), "missing {needle}");
    }
}

fn currency_cell(value: i64, code: &str) -> Cell {
    Cell::Currency {
        value,
        currency: CurrencyFormat {
            code: code.to_string(),
            locale: None,
            decimals: None,
            negative: None,
            symbol: None,
        },
        style: None,
    }
}

#[test]
fn writes_a_styled_multi_sheet_workbook_with_metadata() {
    let wb = Workbook {
        schema_version: Some("1.0".into()),
        locale: Some("es-MX".into()),
        sheets: vec![
            Sheet {
                name: "Resumen".into(),
                columns: vec![Column {
                    width: Some(24.0),
                    ..Default::default()
                }],
                rows: vec![
                    Row {
                        cells: vec![
                            Cell::String {
                                value: "Ingeniería".into(),
                                style: None,
                            },
                            currency_cell(1234567, "MXN"),
                        ],
                        ..Default::default()
                    },
                    Row {
                        cells: vec![
                            Cell::String {
                                value: "Total".into(),
                                style: None,
                            },
                            currency_cell(9876543, "MXN"),
                        ],
                        is_total: Some(true),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            Sheet {
                name: "Detalle".into(),
                rows: vec![],
                ..Default::default()
            },
        ],
    };
    let opts = WriteOptions {
        meta: DocMeta {
            title: Some("Reporte".into()),
            author: Some("Flux".into()),
            subject: Some("Nómina".into()),
            company: Some("FluxPayroll".into()),
        },
        password: None,
    };
    let result = write(&wb, &opts).unwrap();
    assert!(result.diagnostics.is_empty());
    assert_xlsx(
        &result.xlsx,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "<dc:title>Reporte</dc:title>",
            "<Company>FluxPayroll</Company>",
            "rId2",
        ],
    );
}

#[test]
fn writes_minimal_workbook_without_metadata() {
    let wb = Workbook {
        sheets: vec![Sheet {
            name: "S".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = write(&wb, &WriteOptions::default()).unwrap();
    let text = String::from_utf8_lossy(&result.xlsx);
    assert!(!text.contains("<dc:title>"));
    assert!(!text.contains("<Company>"));
    assert!(text.contains("turbo-xlsx"));
}

#[test]
fn rows_fast_path_with_and_without_name() {
    let cols = vec![Column {
        width: Some(10.0),
        ..Default::default()
    }];
    let rows = vec![Row {
        cells: vec![Cell::Number {
            value: 42.0,
            format: None,
            style: None,
        }],
        ..Default::default()
    }];
    let named = write_rows(
        Some("Data".into()),
        cols.clone(),
        rows.clone(),
        Some("en-US".into()),
        &WriteOptions::default(),
    )
    .unwrap();
    assert_xlsx(&named.xlsx, &["name=\"Data\""]);
    let unnamed = write_rows(None, cols, rows, None, &WriteOptions::default()).unwrap();
    assert_xlsx(&unnamed.xlsx, &["name=\"Sheet1\""]);
}

#[test]
fn json_entry_points() {
    let json = r#"{"schemaVersion":"1.0","locale":"pt-PT","sheets":[{"name":"S","rows":[
        {"cells":[{"type":"currency","value":100000,"currency":{"code":"EUR","locale":"pt-PT"}}]}]}]}"#;
    let from_str = write_from_json_str(json, &WriteOptions::default()).unwrap();
    assert_xlsx(&from_str.xlsx, &["xl/workbook.xml"]);
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    let from_val = write_from_json_value(value, &WriteOptions::default()).unwrap();
    assert_eq!(from_str.xlsx, from_val.xlsx);
}

#[test]
fn write_rejects_invalid_workbook() {
    let empty = Workbook::default();
    assert!(write(&empty, &WriteOptions::default()).is_err());
    assert!(write_from_json_str("{bad", &WriteOptions::default()).is_err());
}

#[test]
fn streaming_writer_round_trip() {
    let mut w = WorkbookWriter::new(Some("en-US".into()), WriteOptions::default());
    w.start_sheet(Sheet {
        name: "A".into(),
        columns: vec![Column {
            width: Some(12.0),
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap();
    w.write_row(&Row {
        cells: vec![Cell::String {
            value: "x".into(),
            style: None,
        }],
        ..Default::default()
    })
    .unwrap();
    w.write_row(&Row {
        cells: vec![currency_cell(500, "USD")],
        ..Default::default()
    })
    .unwrap();
    // starting a second sheet auto-closes the first
    w.start_sheet(Sheet {
        name: "B".into(),
        ..Default::default()
    })
    .unwrap();
    w.write_row(&Row {
        cells: vec![Cell::Number {
            value: 1.0,
            format: None,
            style: None,
        }],
        ..Default::default()
    })
    .unwrap();
    w.end_sheet().unwrap();
    w.end_sheet().unwrap(); // idempotent
    let result = w.finish().unwrap();
    assert_xlsx(
        &result.xlsx,
        &[
            "xl/worksheets/sheet1.xml",
            "xl/worksheets/sheet2.xml",
            "name=\"A\"",
            "name=\"B\"",
        ],
    );
}

#[test]
fn streaming_write_row_without_open_sheet_is_noop() {
    let mut w = WorkbookWriter::new(None, WriteOptions::default());
    w.start_sheet(Sheet {
        name: "S".into(),
        ..Default::default()
    })
    .unwrap();
    w.write_row(&Row {
        cells: vec![Cell::Blank { style: None }],
        ..Default::default()
    })
    .unwrap();
    w.end_sheet().unwrap();
    // no sheet open now — this row is dropped without error
    w.write_row(&Row {
        cells: vec![Cell::Blank { style: None }],
        ..Default::default()
    })
    .unwrap();
    assert!(w.finish().is_ok());
}

#[test]
fn streaming_bad_merge_surfaces_on_next_start() {
    let mut w = WorkbookWriter::new(None, WriteOptions::default());
    w.start_sheet(Sheet {
        name: "A".into(),
        merges: vec!["nope".into()],
        ..Default::default()
    })
    .unwrap();
    // closing A (triggered by starting B) fails on the bad merge
    assert!(w
        .start_sheet(Sheet {
            name: "B".into(),
            ..Default::default()
        })
        .is_err());
}

#[test]
fn streaming_columnar_round_trip() {
    let mut w = WorkbookWriter::new(Some("es-MX".into()), WriteOptions::default());
    // no sheet open yet → no-op
    w.write_columns(vec![ColumnData::Strings(vec!["x".into()])])
        .unwrap();
    w.start_sheet(Sheet {
        name: "Cols".into(),
        ..Default::default()
    })
    .unwrap();
    w.write_columns(vec![
        ColumnData::Strings(vec!["Alice".into(), "Bob".into()]),
        ColumnData::Currency {
            values: vec![123456.0, 654321.0],
            format: CurrencyFormat {
                code: "MXN".into(),
                locale: Some("es-MX".into()),
                decimals: None,
                negative: None,
                symbol: None,
            },
        },
        ColumnData::Numbers {
            values: vec![1.5, 2.5],
            format: None,
        },
        ColumnData::Percents {
            values: vec![0.1, 0.2],
            decimals: Some(1),
        },
    ])
    .unwrap();
    w.end_sheet().unwrap();
    let result = w.finish().unwrap();
    assert_xlsx(&result.xlsx, &["name=\"Cols\"", "<v>1234.56</v>", "Alice"]);
}

#[test]
fn streaming_bad_merge_surfaces_on_finish() {
    let mut w = WorkbookWriter::new(None, WriteOptions::default());
    w.start_sheet(Sheet {
        name: "A".into(),
        merges: vec!["nope".into()],
        ..Default::default()
    })
    .unwrap();
    assert!(w.finish().is_err());
}
