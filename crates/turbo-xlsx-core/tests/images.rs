//! End-to-end image-embedding tests on the public writer. The OPC zip is STORED
//! (uncompressed), so every part name and XML body appears verbatim in the output
//! bytes — we assert on those without needing a zip reader. These run under the
//! default (writer-only) features, so they guard the write side on every
//! `cargo test --workspace`.

use turbo_xlsx_core::{
    write, CellRef, ImageAnchor, ImageFormat, Sheet, SheetImage, Workbook, WriteOptions,
};

/// "hello" base64-encoded — a valid, non-empty payload (its bytes need not be a
/// real PNG for the structural assertions here).
const DATA: &str = "aGVsbG8=";
/// A different payload, to prove distinct bytes get distinct media parts.
const DATA2: &str = "d29ybGQ=";

fn contains(haystack: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    haystack.windows(n.len()).any(|w| w == n)
}

fn two_cell(from: (u32, u32), to: (u32, u32)) -> ImageAnchor {
    ImageAnchor::TwoCell {
        from: CellRef {
            col: from.0,
            row: from.1,
        },
        to: CellRef {
            col: to.0,
            row: to.1,
        },
    }
}

fn one_cell(col: u32, row: u32, width: u32, height: u32) -> ImageAnchor {
    ImageAnchor::OneCell {
        at: CellRef { col, row },
        width,
        height,
    }
}

fn image(data: &str, format: ImageFormat, anchor: ImageAnchor, alt: Option<&str>) -> SheetImage {
    SheetImage {
        data: data.to_string(),
        format,
        anchor,
        alt: alt.map(str::to_string),
    }
}

fn sheet(name: &str, images: Vec<SheetImage>) -> Sheet {
    Sheet {
        name: name.to_string(),
        images,
        ..Default::default()
    }
}

fn write_xlsx(sheets: Vec<Sheet>) -> Vec<u8> {
    write(
        &Workbook {
            sheets,
            ..Default::default()
        },
        &WriteOptions::default(),
    )
    .expect("write should succeed")
    .xlsx
}

#[test]
fn embeds_two_cell_image_with_all_opc_parts() {
    let img = image(
        DATA,
        ImageFormat::Png,
        two_cell((0, 0), (3, 6)),
        Some("Company logo"),
    );
    let bytes = write_xlsx(vec![sheet("S1", vec![img])]);

    // Media + drawing + both rels parts exist.
    assert!(contains(&bytes, "xl/media/image1.png"));
    assert!(contains(&bytes, "xl/drawings/drawing1.xml"));
    assert!(contains(&bytes, "xl/drawings/_rels/drawing1.xml.rels"));
    assert!(contains(&bytes, "xl/worksheets/_rels/sheet1.xml.rels"));

    // Content types: a png Default + a drawing Override.
    assert!(contains(
        &bytes,
        "<Default Extension=\"png\" ContentType=\"image/png\"/>"
    ));
    assert!(contains(
        &bytes,
        "<Override PartName=\"/xl/drawings/drawing1.xml\""
    ));

    // The worksheet references its drawing, and the drawing carries the anchor +
    // escaped alt text + the blip relationship.
    assert!(contains(&bytes, "<drawing xmlns:r=") && contains(&bytes, "r:id=\"rId1\"/>"));
    assert!(contains(&bytes, "<xdr:twoCellAnchor"));
    assert!(contains(&bytes, "descr=\"Company logo\""));
    assert!(contains(&bytes, "r:embed=\"rId1\""));
}

#[test]
fn one_cell_anchor_writes_extent_in_emu() {
    let img = image(DATA, ImageFormat::Jpeg, one_cell(1, 1, 120, 80), None);
    let bytes = write_xlsx(vec![sheet("S1", vec![img])]);

    assert!(contains(&bytes, "xl/media/image1.jpeg"));
    assert!(contains(
        &bytes,
        "<Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>"
    ));
    assert!(contains(&bytes, "<xdr:oneCellAnchor>"));
    // 120 px * 9525 = 1143000 EMU; 80 px * 9525 = 762000 EMU.
    assert!(contains(&bytes, "cx=\"1143000\""));
    assert!(contains(&bytes, "cy=\"762000\""));
}

#[test]
fn identical_image_bytes_are_interned_once() {
    let a = image(DATA, ImageFormat::Png, one_cell(0, 0, 10, 10), None);
    let b = image(DATA, ImageFormat::Png, one_cell(2, 2, 10, 10), None);
    let bytes = write_xlsx(vec![sheet("S1", vec![a, b])]);

    // Both anchors point at one media part; no image2 is emitted.
    assert!(contains(&bytes, "xl/media/image1.png"));
    assert!(!contains(&bytes, "xl/media/image2.png"));
}

#[test]
fn second_sheet_gets_its_own_drawing_number() {
    let s1 = sheet(
        "First",
        vec![image(DATA, ImageFormat::Png, one_cell(0, 0, 10, 10), None)],
    );
    let s2 = sheet(
        "Second",
        vec![image(DATA2, ImageFormat::Gif, one_cell(0, 0, 10, 10), None)],
    );
    let bytes = write_xlsx(vec![s1, s2]);

    assert!(contains(&bytes, "xl/drawings/drawing1.xml"));
    assert!(contains(&bytes, "xl/drawings/drawing2.xml"));
    assert!(contains(&bytes, "xl/worksheets/_rels/sheet2.xml.rels"));
    assert!(contains(
        &bytes,
        "<Default Extension=\"gif\" ContentType=\"image/gif\"/>"
    ));
}

#[test]
fn image_free_workbook_has_no_drawing_or_media_parts() {
    let bytes = write_xlsx(vec![sheet("Plain", vec![])]);
    assert!(!contains(&bytes, "xl/drawings/"));
    assert!(!contains(&bytes, "xl/media/"));
    assert!(!contains(&bytes, "<drawing"));
    // And no spurious image content-type defaults leak in.
    assert!(!contains(&bytes, "image/png"));
}

#[test]
fn invalid_image_is_rejected_fail_closed() {
    let bad = image(DATA, ImageFormat::Png, two_cell((3, 3), (1, 1)), None);
    let err = write(
        &Workbook {
            sheets: vec![sheet("S1", vec![bad])],
            ..Default::default()
        },
        &WriteOptions::default(),
    )
    .unwrap_err();
    assert_eq!(err.code, turbo_xlsx_core::ErrorCode::BadImage);
}
