//! Parse-mode round trip: write a workbook, read it back, and check the values
//! + the JSON/CSV/Markdown serializers. Gated on the `parse` feature.
#![cfg(feature = "parse")]

use turbo_xlsx_core::parse::serialize::{to_csv, to_json_grid, to_json_typed, to_markdown};
use turbo_xlsx_core::parse::{parse, CellValue};
use turbo_xlsx_core::{
    write, Cell, CurrencyFormat, DateFormat, DateKind, DateValue, Row, Sheet, Workbook,
    WriteOptions,
};

fn sample() -> Vec<u8> {
    let wb = Workbook {
        locale: Some("es-MX".into()),
        sheets: vec![Sheet {
            name: "Data".into(),
            rows: vec![
                Row {
                    cells: vec![
                        Cell::String {
                            value: "Name".into(),
                            style: None,
                        },
                        Cell::String {
                            value: "Amt".into(),
                            style: None,
                        },
                    ],
                    ..Default::default()
                },
                Row {
                    cells: vec![
                        Cell::String {
                            value: "Alice, \"A\"".into(),
                            style: None,
                        },
                        Cell::Currency {
                            value: 123456,
                            currency: CurrencyFormat {
                                code: "MXN".into(),
                                locale: Some("es-MX".into()),
                                decimals: None,
                                negative: None,
                                symbol: None,
                            },
                            style: None,
                        },
                    ],
                    ..Default::default()
                },
                Row {
                    cells: vec![
                        Cell::Boolean {
                            value: true,
                            style: None,
                        },
                        Cell::Number {
                            value: 3.5,
                            format: None,
                            style: None,
                        },
                    ],
                    ..Default::default()
                },
                Row {
                    cells: vec![
                        Cell::Date {
                            value: DateValue::Iso("2026-06-20".into()),
                            format: Some(DateFormat {
                                kind: Some(DateKind::Date),
                                raw: None,
                            }),
                            style: None,
                        },
                        Cell::Blank { style: None },
                    ],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    write(&wb, &WriteOptions::default()).unwrap().xlsx
}

#[test]
fn round_trip_values() {
    let parsed = parse(&sample()).unwrap();
    assert_eq!(parsed.sheets.len(), 1);
    let s = &parsed.sheets[0];
    assert_eq!(s.name, "Data");
    assert_eq!(s.rows[0][0], CellValue::Text("Name".into()));
    assert_eq!(s.rows[1][0], CellValue::Text("Alice, \"A\"".into())); // escaped string survives
    assert_eq!(s.rows[1][1], CellValue::Number(1234.56)); // currency minor units / 100
    assert_eq!(s.rows[2][0], CellValue::Bool(true));
    assert_eq!(s.rows[2][1], CellValue::Number(3.5));
    assert_eq!(s.rows[3][0], CellValue::Date("2026-06-20".into())); // builtin date numFmt
    assert_eq!(s.rows[3][1], CellValue::Empty); // self-closing blank cell
}

#[test]
fn serializers_render() {
    let parsed = parse(&sample()).unwrap();
    let sheet = &parsed.sheets[0];

    let grid = to_json_grid(&parsed);
    assert!(
        grid.contains("\"name\":\"Data\"")
            && grid.contains("1234.56")
            && grid.contains("\"2026-06-20\"")
    );

    let typed = to_json_typed(&parsed);
    assert!(
        typed.contains("\"type\":\"date\"")
            && typed.contains("\"type\":\"boolean\"")
            && typed.contains("schemaVersion")
    );

    let csv = to_csv(sheet);
    // the field with a comma + quote must be RFC-4180 quoted/escaped.
    assert!(csv.contains("\"Alice, \"\"A\"\"\""));
    assert!(csv.contains("1234.56"));

    let md = to_markdown(sheet);
    assert!(md.contains("| Name | Amt |"));
    assert!(md.contains("| --- |"));
    assert!(md.contains("TRUE"));
}

#[test]
fn rejects_non_xlsx() {
    assert!(parse(b"not a zip at all").is_err());
}

// ---- crafted-XLSX edge cases (sheet XML our writer never emits) -------------

fn pu16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn pu32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// Assemble a STORED (crc-less; the reader ignores crc) OPC zip from parts.
fn stored_zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in parts {
        let off = out.len() as u32;
        pu32(&mut out, 0x0403_4b50);
        for _ in 0..3 {
            pu16(&mut out, 0);
        }
        pu16(&mut out, 0);
        pu16(&mut out, 0x21);
        pu32(&mut out, 0);
        pu32(&mut out, data.len() as u32);
        pu32(&mut out, data.len() as u32);
        pu16(&mut out, name.len() as u16);
        pu16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        pu32(&mut central, 0x0201_4b50);
        pu16(&mut central, 20);
        pu16(&mut central, 20);
        pu16(&mut central, 0);
        pu16(&mut central, 0);
        pu16(&mut central, 0);
        pu16(&mut central, 0x21);
        pu32(&mut central, 0);
        pu32(&mut central, data.len() as u32);
        pu32(&mut central, data.len() as u32);
        pu16(&mut central, name.len() as u16);
        for _ in 0..4 {
            pu16(&mut central, 0);
        }
        pu32(&mut central, 0);
        pu32(&mut central, off);
        central.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    pu32(&mut out, 0x0605_4b50);
    pu16(&mut out, 0);
    pu16(&mut out, 0);
    pu16(&mut out, parts.len() as u16);
    pu16(&mut out, parts.len() as u16);
    pu32(&mut out, cd_size);
    pu32(&mut out, cd_off);
    pu16(&mut out, 0);
    out
}

/// A minimal one-sheet xlsx with the given worksheet XML + optional parts.
fn craft(sheet_xml: &str, shared: Option<&str>, styles: Option<&str>) -> Vec<u8> {
    let wb = r#"<?xml version="1.0"?><workbook xmlns:r="r"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let rels = r#"<?xml version="1.0"?><Relationships><Relationship Id="rId1" Type="t" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet_xml.as_bytes()),
    ];
    if let Some(s) = shared {
        parts.push(("xl/sharedStrings.xml", s.as_bytes()));
    }
    if let Some(s) = styles {
        parts.push(("xl/styles.xml", s.as_bytes()));
    }
    stored_zip(&parts)
}

fn first_sheet(bytes: &[u8]) -> Vec<Vec<CellValue>> {
    parse(bytes)
        .unwrap()
        .sheets
        .into_iter()
        .next()
        .unwrap()
        .rows
}

#[test]
fn sparse_cells_are_padded() {
    let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>A</t></is></c><c r="C1"><v>3</v></c></row></sheetData></worksheet>"#;
    let rows = first_sheet(&craft(xml, None, None));
    assert_eq!(
        rows[0],
        vec![
            CellValue::Text("A".into()),
            CellValue::Empty,
            CellValue::Number(3.0)
        ]
    );
}

#[test]
fn multi_letter_column_ref() {
    let xml =
        r#"<worksheet><sheetData><row r="1"><c r="AA1"><v>9</v></c></row></sheetData></worksheet>"#;
    let rows = first_sheet(&craft(xml, None, None));
    assert_eq!(rows[0].len(), 27); // A..Z padded, value at AA (index 26)
    assert_eq!(rows[0][26], CellValue::Number(9.0));
}

#[test]
fn formula_string_uses_cached_value() {
    let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="str"><v>cached</v></c></row></sheetData></worksheet>"#;
    assert_eq!(
        first_sheet(&craft(xml, None, None))[0][0],
        CellValue::Text("cached".into())
    );
}

#[test]
fn shared_string_out_of_range_is_empty() {
    let shared = r#"<sst><si><t>only</t></si></sst>"#;
    let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>9</v></c></row></sheetData></worksheet>"#;
    let rows = first_sheet(&craft(xml, Some(shared), None));
    assert_eq!(
        rows[0],
        vec![CellValue::Text("only".into()), CellValue::Empty]
    );
}

#[test]
fn shared_string_concatenates_runs() {
    let shared = r#"<sst><si><r><t>Hello </t></r><r><t>World</t></r></si></sst>"#;
    let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData></worksheet>"#;
    assert_eq!(
        first_sheet(&craft(xml, Some(shared), None))[0][0],
        CellValue::Text("Hello World".into())
    );
}

#[test]
fn non_numeric_value_falls_back_to_text() {
    let xml = r#"<worksheet><sheetData><row r="1"><c r="A1"><v>not-a-number</v></c></row></sheetData></worksheet>"#;
    assert_eq!(
        first_sheet(&craft(xml, None, None))[0][0],
        CellValue::Text("not-a-number".into())
    );
}

#[test]
fn custom_date_format_infers_date() {
    let styles = r#"<styleSheet><numFmts><numFmt numFmtId="170" formatCode="dd/mm/yyyy"/></numFmts><cellXfs><xf numFmtId="0"/><xf numFmtId="170"/></cellXfs></styleSheet>"#;
    let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" s="1"><v>45292</v></c><c r="B1" s="0"><v>45292</v></c></row></sheetData></worksheet>"#;
    let rows = first_sheet(&craft(xml, None, Some(styles)));
    assert_eq!(rows[0][0], CellValue::Date("2024-01-01".into())); // styled date
    assert_eq!(rows[0][1], CellValue::Number(45292.0)); // same serial, no date style
}

#[test]
fn datetime_serial_keeps_time() {
    // built-in numFmt 22 is a date-time; the .5 fraction becomes 12:00:00.
    let styles = r#"<styleSheet><cellXfs><xf numFmtId="22"/></cellXfs></styleSheet>"#;
    let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" s="0"><v>45292.5</v></c></row></sheetData></worksheet>"#;
    assert_eq!(
        first_sheet(&craft(xml, None, Some(styles)))[0][0],
        CellValue::Date("2024-01-01T12:00:00".into())
    );
}

#[test]
fn missing_workbook_is_not_xlsx() {
    let bytes = stored_zip(&[("random.txt", b"hi")]);
    assert!(parse(&bytes).is_err());
}

#[test]
fn truncated_and_empty_inputs_error() {
    assert!(parse(b"").is_err());
    assert!(parse(b"PK\x03\x04 truncated").is_err());
    assert!(parse(&[0u8; 8]).is_err());
}

// ---- generation edge cases that round-trip through parse --------------------

#[test]
fn round_trip_edge_via_writer() {
    let mut cells = vec![Cell::String {
        value: "a & b < c > \"d\" 'e'".into(),
        style: None,
    }];
    for i in 0..28 {
        cells.push(Cell::Number {
            value: i as f64 - 14.0,
            format: None,
            style: None,
        });
    }
    let wb = Workbook {
        sheets: vec![
            Sheet {
                name: "One".into(),
                rows: vec![Row {
                    cells,
                    ..Default::default()
                }],
                ..Default::default()
            },
            Sheet {
                name: "Empty".into(),
                rows: vec![],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let parsed = parse(&write(&wb, &WriteOptions::default()).unwrap().xlsx).unwrap();
    assert_eq!(parsed.sheets.len(), 2);
    assert_eq!(parsed.sheets[1].name, "Empty");
    assert!(parsed.sheets[1].rows.is_empty());
    let r0 = &parsed.sheets[0].rows[0];
    assert_eq!(r0[0], CellValue::Text("a & b < c > \"d\" 'e'".into())); // XML entities survive
    assert_eq!(r0.len(), 29); // string + 28 numbers (cols A..AC)
    assert_eq!(r0[1], CellValue::Number(-14.0)); // negative
    assert_eq!(r0[15], CellValue::Number(0.0)); // zero
}
