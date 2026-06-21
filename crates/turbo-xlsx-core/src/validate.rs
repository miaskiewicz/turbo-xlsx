//! Fail-closed validation of a workbook before it is written, plus the JSON entry
//! points (parse + schema-shape check). The same checks run no matter how the
//! workbook arrived — declarative object, JSON, or builder.

use serde_json::Value;

use crate::error::{ErrorCode, Result, TurboXlsxError};
use crate::model::{CellRef, ImageAnchor, Sheet, SheetImage, Workbook};

/// Characters Excel forbids in a worksheet tab name.
const FORBIDDEN_NAME_CHARS: &[char] = &[':', '\\', '/', '?', '*', '[', ']'];
/// Excel's maximum worksheet name length.
const MAX_SHEET_NAME: usize = 31;

/// Validate workbook-level invariants: at least one sheet, and sheet names that
/// are non-empty, within Excel's length/character limits, and unique.
pub fn validate(workbook: &Workbook) -> Result<()> {
    if workbook.sheets.is_empty() {
        return Err(TurboXlsxError::new(
            ErrorCode::EmptyWorkbook,
            "a workbook needs at least one sheet",
        ));
    }
    let mut seen = Vec::new();
    for sheet in &workbook.sheets {
        check_sheet(sheet, &mut seen)?;
    }
    Ok(())
}

/// Validate one sheet (name, images) and reject a case-insensitive duplicate
/// name against the running `seen` set.
fn check_sheet(sheet: &Sheet, seen: &mut Vec<String>) -> Result<()> {
    check_name(&sheet.name)?;
    check_images(sheet)?;
    let lower = sheet.name.to_lowercase();
    if seen.contains(&lower) {
        return Err(TurboXlsxError::new(
            ErrorCode::DuplicateSheetName,
            format!("duplicate sheet name {:?}", sheet.name),
        ));
    }
    seen.push(lower);
    Ok(())
}

/// Validate every embedded image on a sheet: decodable, non-empty bytes and a
/// well-formed anchor.
fn check_images(sheet: &Sheet) -> Result<()> {
    for img in &sheet.images {
        check_image(img)?;
    }
    Ok(())
}

/// One image: its base64 must decode to non-empty bytes and its anchor be sane.
fn check_image(img: &SheetImage) -> Result<()> {
    match crate::b64::decode(&img.data) {
        Some(bytes) if !bytes.is_empty() => check_anchor(&img.anchor),
        _ => Err(TurboXlsxError::new(
            ErrorCode::BadImage,
            "image data is not valid, non-empty base64",
        )),
    }
}

/// An anchor: a two-cell range must extend down/right; a one-cell image needs a
/// non-zero pixel extent.
fn check_anchor(anchor: &ImageAnchor) -> Result<()> {
    match anchor {
        ImageAnchor::TwoCell { from, to } => check_two_cell(from, to),
        ImageAnchor::OneCell { width, height, .. } => check_one_cell(*width, *height),
    }
}

/// A two-cell anchor's `to` must be strictly below-and/or-right of `from`.
fn check_two_cell(from: &CellRef, to: &CellRef) -> Result<()> {
    if to.col < from.col || to.row < from.row || (to.col == from.col && to.row == from.row) {
        return Err(TurboXlsxError::new(
            ErrorCode::BadImage,
            "two-cell image anchor 'to' must be below-and/or-right of 'from'",
        ));
    }
    Ok(())
}

/// A one-cell anchor needs a non-zero width and height.
fn check_one_cell(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(TurboXlsxError::new(
            ErrorCode::BadImage,
            "one-cell image needs non-zero width and height",
        ));
    }
    Ok(())
}

/// Validate a single sheet name against Excel's rules.
fn check_name(name: &str) -> Result<()> {
    let bad = |msg: String| Err(TurboXlsxError::new(ErrorCode::InvalidSheetName, msg));
    if name.is_empty() {
        return bad("sheet name must not be empty".to_string());
    }
    if name.chars().count() > MAX_SHEET_NAME {
        return bad(format!("sheet name {name:?} exceeds 31 characters"));
    }
    if name.contains(FORBIDDEN_NAME_CHARS) {
        return bad(format!(
            "sheet name {name:?} contains a forbidden character (: \\ / ? * [ ])"
        ));
    }
    Ok(())
}

/// Parse a JSON workbook from a string, distinguishing a JSON syntax error
/// (`InvalidJson`) from a well-formed-but-wrong shape (`SchemaViolation`).
pub fn from_json_str(input: &str) -> Result<Workbook> {
    serde_json::from_str(input).map_err(|e| map_json_err(&e))
}

/// Parse a JSON workbook from an already-parsed `serde_json::Value`. A shape
/// mismatch is a `SchemaViolation`.
pub fn from_json_value(input: Value) -> Result<Workbook> {
    serde_json::from_value(input)
        .map_err(|e| TurboXlsxError::new(ErrorCode::SchemaViolation, e.to_string()))
}

/// Map a `serde_json` error to the right fatal code: a syntactically broken
/// document is `InvalidJson`; a structurally wrong one is `SchemaViolation`.
fn map_json_err(e: &serde_json::Error) -> TurboXlsxError {
    let code = match e.classify() {
        serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
            ErrorCode::InvalidJson
        }
        _ => ErrorCode::SchemaViolation,
    };
    TurboXlsxError::new(code, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ImageFormat, Sheet, Workbook};

    fn wb(names: &[&str]) -> Workbook {
        Workbook {
            sheets: names
                .iter()
                .map(|n| Sheet {
                    name: n.to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn accepts_unique_named_sheets() {
        assert!(validate(&wb(&["One", "Two"])).is_ok());
    }

    #[test]
    fn rejects_empty_workbook() {
        assert_eq!(
            validate(&wb(&[])).unwrap_err().code,
            ErrorCode::EmptyWorkbook
        );
    }

    #[test]
    fn rejects_duplicate_names_case_insensitively() {
        assert_eq!(
            validate(&wb(&["Sheet", "sheet"])).unwrap_err().code,
            ErrorCode::DuplicateSheetName
        );
    }

    #[test]
    fn rejects_bad_names() {
        assert_eq!(
            validate(&wb(&[""])).unwrap_err().code,
            ErrorCode::InvalidSheetName
        );
        let long = "x".repeat(32);
        assert_eq!(
            validate(&wb(&[long.as_str()])).unwrap_err().code,
            ErrorCode::InvalidSheetName
        );
        assert_eq!(
            validate(&wb(&["a/b"])).unwrap_err().code,
            ErrorCode::InvalidSheetName
        );
    }

    #[test]
    fn json_string_parsing() {
        let good = r#"{"sheets":[{"name":"S","rows":[]}]}"#;
        assert!(from_json_str(good).is_ok());
        assert_eq!(
            from_json_str("{ not json").unwrap_err().code,
            ErrorCode::InvalidJson
        );
        let unknown = r#"{"sheets":[],"bogus":1}"#;
        assert_eq!(
            from_json_str(unknown).unwrap_err().code,
            ErrorCode::SchemaViolation
        );
    }

    fn img_wb(img: SheetImage) -> Workbook {
        Workbook {
            sheets: vec![Sheet {
                name: "S".to_string(),
                images: vec![img],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn good_data() -> String {
        crate::b64::encode(b"\x89PNG-bytes")
    }

    fn anchored(anchor: ImageAnchor, data: &str) -> SheetImage {
        SheetImage {
            data: data.to_string(),
            format: ImageFormat::Png,
            anchor,
            alt: None,
        }
    }

    #[test]
    fn accepts_valid_two_cell_and_one_cell_images() {
        let two = anchored(
            ImageAnchor::TwoCell {
                from: CellRef { col: 0, row: 0 },
                to: CellRef { col: 2, row: 3 },
            },
            &good_data(),
        );
        assert!(validate(&img_wb(two)).is_ok());
        let one = anchored(
            ImageAnchor::OneCell {
                at: CellRef { col: 1, row: 1 },
                width: 100,
                height: 50,
            },
            &good_data(),
        );
        assert!(validate(&img_wb(one)).is_ok());
    }

    #[test]
    fn rejects_bad_image_data() {
        let anchor = ImageAnchor::OneCell {
            at: CellRef { col: 0, row: 0 },
            width: 1,
            height: 1,
        };
        // Not valid base64.
        let invalid = anchored(anchor, "%%% not base64 %%%");
        assert_eq!(
            validate(&img_wb(invalid)).unwrap_err().code,
            ErrorCode::BadImage
        );
        // Valid base64 but empty bytes.
        let empty = anchored(anchor, "");
        assert_eq!(
            validate(&img_wb(empty)).unwrap_err().code,
            ErrorCode::BadImage
        );
    }

    #[test]
    fn rejects_bad_two_cell_anchor() {
        // `to` above-and-left of `from`.
        let inverted = anchored(
            ImageAnchor::TwoCell {
                from: CellRef { col: 3, row: 3 },
                to: CellRef { col: 1, row: 1 },
            },
            &good_data(),
        );
        assert_eq!(
            validate(&img_wb(inverted)).unwrap_err().code,
            ErrorCode::BadImage
        );
        // Degenerate zero-area range (from == to).
        let degenerate = anchored(
            ImageAnchor::TwoCell {
                from: CellRef { col: 2, row: 2 },
                to: CellRef { col: 2, row: 2 },
            },
            &good_data(),
        );
        assert_eq!(
            validate(&img_wb(degenerate)).unwrap_err().code,
            ErrorCode::BadImage
        );
    }

    #[test]
    fn rejects_zero_sized_one_cell_anchor() {
        let zero_w = anchored(
            ImageAnchor::OneCell {
                at: CellRef { col: 0, row: 0 },
                width: 0,
                height: 10,
            },
            &good_data(),
        );
        assert_eq!(
            validate(&img_wb(zero_w)).unwrap_err().code,
            ErrorCode::BadImage
        );
        let zero_h = anchored(
            ImageAnchor::OneCell {
                at: CellRef { col: 0, row: 0 },
                width: 10,
                height: 0,
            },
            &good_data(),
        );
        assert_eq!(
            validate(&img_wb(zero_h)).unwrap_err().code,
            ErrorCode::BadImage
        );
    }

    #[test]
    fn json_value_parsing() {
        let v: Value = serde_json::from_str(r#"{"sheets":[{"name":"S","rows":[]}]}"#).unwrap();
        assert!(from_json_value(v).is_ok());
        let bad: Value = serde_json::from_str(r#"{"sheets":"nope"}"#).unwrap();
        assert_eq!(
            from_json_value(bad).unwrap_err().code,
            ErrorCode::SchemaViolation
        );
    }
}
