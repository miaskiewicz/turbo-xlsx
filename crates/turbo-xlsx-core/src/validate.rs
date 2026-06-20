//! Fail-closed validation of a workbook before it is written, plus the JSON entry
//! points (parse + schema-shape check). The same checks run no matter how the
//! workbook arrived — declarative object, JSON, or builder.

use serde_json::Value;

use crate::error::{ErrorCode, Result, TurboXlsxError};
use crate::model::Workbook;

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
        check_name(&sheet.name)?;
        let lower = sheet.name.to_lowercase();
        if seen.contains(&lower) {
            return Err(TurboXlsxError::new(
                ErrorCode::DuplicateSheetName,
                format!("duplicate sheet name {:?}", sheet.name),
            ));
        }
        seen.push(lower);
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
    use crate::model::{Sheet, Workbook};

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
