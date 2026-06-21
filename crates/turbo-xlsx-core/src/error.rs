//! Error and diagnostic types shared across the writer.
//!
//! The split mirrors the sibling `turbo-html2pdf` engine: a fatal fault is a
//! [`TurboXlsxError`] carrying a stable machine-readable [`ErrorCode`]; non-fatal
//! problems (a clamped width, a dropped duplicate merge) are collected as
//! [`Lint`]s in [`Diagnostics`] and returned alongside the bytes, never thrown.

use thiserror::Error;

/// Stable machine-readable code for a fatal write fault. The string form (the
/// variant name) crosses the N-API boundary as `TurboXlsxError.code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The workbook has no sheets (at least one is required).
    EmptyWorkbook,
    /// Two sheets share a name (names must be unique, case-insensitively).
    DuplicateSheetName,
    /// A sheet name is empty, longer than 31 characters, or uses a character
    /// Excel forbids in a tab name (`: \ / ? * [ ]`).
    InvalidSheetName,
    /// A merge range / cell reference could not be parsed (e.g. `"A1:"`).
    BadCellRef,
    /// A colour string was not `#rrggbb` / `rrggbb` hex.
    BadColor,
    /// An embedded image's data was not valid base64, or its anchor was
    /// malformed (e.g. a two-cell range whose `to` is not below-and-right of
    /// `from`, or a one-cell image with zero width/height).
    BadImage,
    /// The JSON input was not valid JSON.
    InvalidJson,
    /// The JSON input parsed but did not match the documented workbook schema
    /// (unknown field, wrong type, missing required field).
    SchemaViolation,
    /// XLSX password encryption failed (only with the `encrypt` feature).
    #[cfg(feature = "encrypt")]
    Encryption,
}

impl ErrorCode {
    /// The stable string form of the code (mirrors the variant name).
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::EmptyWorkbook => "EmptyWorkbook",
            ErrorCode::DuplicateSheetName => "DuplicateSheetName",
            ErrorCode::InvalidSheetName => "InvalidSheetName",
            ErrorCode::BadCellRef => "BadCellRef",
            ErrorCode::BadColor => "BadColor",
            ErrorCode::BadImage => "BadImage",
            ErrorCode::InvalidJson => "InvalidJson",
            ErrorCode::SchemaViolation => "SchemaViolation",
            #[cfg(feature = "encrypt")]
            ErrorCode::Encryption => "Encryption",
        }
    }
}

/// A fatal fault produced while validating or writing a workbook.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{}: {message}", code.as_str())]
pub struct TurboXlsxError {
    pub code: ErrorCode,
    pub message: String,
}

impl TurboXlsxError {
    /// Construct an error from a code and a message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        TurboXlsxError {
            code,
            message: message.into(),
        }
    }
}

/// Stable machine-readable code for a non-fatal lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintCode {
    /// A column width outside Excel's accepted range was clamped.
    ClampedColumnWidth,
    /// An outline level outside `1..=7` was clamped into range.
    ClampedOutlineLevel,
    /// A duplicate merge range was dropped (Excel rejects overlapping merges).
    DroppedDuplicateMerge,
    /// A freeze pane referenced more rows/cols than reasonable; it was clamped.
    ClampedFreeze,
}

impl LintCode {
    /// The stable string form of the lint code (mirrors the variant name).
    pub fn as_str(self) -> &'static str {
        match self {
            LintCode::ClampedColumnWidth => "ClampedColumnWidth",
            LintCode::ClampedOutlineLevel => "ClampedOutlineLevel",
            LintCode::DroppedDuplicateMerge => "DroppedDuplicateMerge",
            LintCode::ClampedFreeze => "ClampedFreeze",
        }
    }
}

/// A non-fatal diagnostic collected during a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    pub code: LintCode,
    pub message: String,
}

/// Collected non-fatal diagnostics returned alongside the bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub lints: Vec<Lint>,
}

impl Diagnostics {
    /// Append a lint to the collection.
    pub fn push(&mut self, code: LintCode, message: impl Into<String>) {
        self.lints.push(Lint {
            code,
            message: message.into(),
        });
    }

    /// True when no lints have been collected.
    pub fn is_empty(&self) -> bool {
        self.lints.is_empty()
    }
}

/// Shorthand result type for fallible writer operations.
pub type Result<T> = std::result::Result<T, TurboXlsxError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_round_trip_to_strings() {
        let codes = [
            ErrorCode::EmptyWorkbook,
            ErrorCode::DuplicateSheetName,
            ErrorCode::InvalidSheetName,
            ErrorCode::BadCellRef,
            ErrorCode::BadColor,
            ErrorCode::BadImage,
            ErrorCode::InvalidJson,
            ErrorCode::SchemaViolation,
        ];
        let names: Vec<&str> = codes.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            [
                "EmptyWorkbook",
                "DuplicateSheetName",
                "InvalidSheetName",
                "BadCellRef",
                "BadColor",
                "BadImage",
                "InvalidJson",
                "SchemaViolation",
            ]
        );
    }

    #[test]
    fn lint_codes_round_trip_to_strings() {
        let codes = [
            LintCode::ClampedColumnWidth,
            LintCode::ClampedOutlineLevel,
            LintCode::DroppedDuplicateMerge,
            LintCode::ClampedFreeze,
        ];
        let names: Vec<&str> = codes.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            [
                "ClampedColumnWidth",
                "ClampedOutlineLevel",
                "DroppedDuplicateMerge",
                "ClampedFreeze"
            ]
        );
    }

    #[test]
    fn error_display_uses_code_and_message() {
        let e = TurboXlsxError::new(ErrorCode::BadColor, "nope");
        assert_eq!(e.to_string(), "BadColor: nope");
    }

    #[cfg(feature = "encrypt")]
    #[test]
    fn encryption_code_string() {
        assert_eq!(ErrorCode::Encryption.as_str(), "Encryption");
    }

    #[test]
    fn diagnostics_push_and_empty() {
        let mut d = Diagnostics::default();
        assert!(d.is_empty());
        d.push(LintCode::ClampedFreeze, "x");
        assert!(!d.is_empty());
        assert_eq!(d.lints.len(), 1);
    }
}
