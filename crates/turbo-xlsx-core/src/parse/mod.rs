//! XLSX **parsing** (read side), behind the off-by-default `parse` feature.
//!
//! turbo-xlsx is primarily a writer; this module adds the inverse — read an
//! `.xlsx` (OPC zip of OOXML parts, possibly DEFLATE-compressed by Excel) into a
//! simple [`ParsedWorkbook`], then serialize it to JSON / CSV / Markdown (see
//! [`serialize`]). It is **dependency-free**: a hand-rolled inflater, zip reader,
//! and XML tokenizer. The parser is excluded from the 100% line-coverage gate
//! (it is branch-heavy on malformed input) and validated functionally — by
//! round-tripping our own writer's output and against reference parsers in
//! `benches/parse-compat`.

mod inflate;
mod read;
pub mod serialize;
mod unzip;
mod xml;

use std::fmt;

pub use read::{parse, CellValue, ParsedSheet, ParsedWorkbook};

/// A fatal parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The DEFLATE stream was malformed or truncated.
    Deflate,
    /// The OPC zip container could not be read.
    Zip,
    /// A required OOXML part was malformed.
    Xml,
    /// The bytes are not a workbook (no `xl/workbook.xml`).
    NotXlsx,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            ParseError::Deflate => "malformed DEFLATE stream",
            ParseError::Zip => "malformed OPC zip container",
            ParseError::Xml => "malformed OOXML part",
            ParseError::NotXlsx => "not an .xlsx (no xl/workbook.xml)",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for ParseError {}
