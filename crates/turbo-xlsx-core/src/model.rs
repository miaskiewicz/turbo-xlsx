//! The structured workbook model — the single typed input every entry mode
//! (declarative object, JSON, builder, rows fast-path, streaming) converges on.
//!
//! The types are `serde`-(de)serializable so the exact same shape is both the
//! in-memory Rust model and the documented JSON workbook schema. Top-level
//! structs use `deny_unknown_fields` so a JSON workbook with a misspelled or
//! stray key is rejected fail-closed (see [`crate::validate`]).
//!
//! Money: a `currency` cell's `value` is an **integer in minor units** (e.g.
//! cents) — `123456` with `decimals: 2` renders `1,234.56`. This matches the
//! ledger money contract and keeps amounts off floating point until the final
//! divide-by-10^decimals at emit time.

use serde::{Deserialize, Serialize};

/// A complete workbook: one or more sheets plus an optional default locale.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workbook {
    /// Schema version of a JSON workbook. Ignored by the in-memory path; carried
    /// so a JSON workbook round-trips through the builder unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    /// Default BCP-47 locale for currency/date number formats; per-cell currency
    /// can override it. Falls back to `"en-US"` when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// At least one sheet; names must be unique.
    pub sheets: Vec<Sheet>,
}

/// One worksheet: its columns, rows and sheet-level layout.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sheet {
    /// Tab name. ≤31 chars, unique in the workbook, no `: \ / ? * [ ]`.
    pub name: String,
    /// Column widths / outline / per-column default style.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<Column>,
    /// Header rows + data rows + totals rows, top to bottom.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Row>,
    /// Merged cell ranges, e.g. `"A1:C1"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merges: Vec<String>,
    /// Freeze panes — keep header rows / id columns visible while scrolling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze: Option<Freeze>,
    /// Default outline state for grouped columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Outline>,
    /// Floating embedded images, each anchored to a cell coordinate and drawn
    /// over the grid (one OOXML drawing part per sheet that has any).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<SheetImage>,
}

/// A floating image embedded in a sheet, anchored to a cell coordinate. The
/// bytes are carried base64-encoded so the model stays JSON-serializable; the
/// writer decodes them into an `xl/media` part and emits a drawing anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SheetImage {
    /// Base64-encoded image bytes (standard alphabet; whitespace is ignored).
    pub data: String,
    /// The image encoding — selects the media extension + content type.
    pub format: ImageFormat,
    /// Where and how big the image is drawn.
    pub anchor: ImageAnchor,
    /// Optional alt text / title (accessibility), written to the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

/// The supported embedded-image encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    #[serde(alias = "jpg")]
    Jpeg,
    Gif,
}

/// How a [`SheetImage`] is positioned and sized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImageAnchor {
    /// Spans from one cell's top-left to another's, resizing with the cells.
    TwoCell { from: CellRef, to: CellRef },
    /// Pinned at one cell with a fixed pixel size (does not resize with cells).
    OneCell {
        at: CellRef,
        /// Width in pixels (96 dpi).
        width: u32,
        /// Height in pixels (96 dpi).
        height: u32,
    },
}

/// A zero-based cell coordinate used by image anchors (`col` = column index,
/// `row` = row index; `A1` is `{ col: 0, row: 0 }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellRef {
    pub col: u32,
    pub row: u32,
}

impl ImageFormat {
    /// The `xl/media/imageN.<ext>` file extension.
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Gif => "gif",
        }
    }

    /// The OPC content type for the media part.
    pub fn content_type(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
        }
    }
}

/// Freeze-pane configuration: how many top rows / left columns to pin.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Freeze {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u32>,
}

/// Sheet-level outline defaults.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Outline {
    /// Render grouped columns collapsed by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns_collapsed: Option<bool>,
}

/// A column definition: width, outline grouping, hidden flag, default style.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Column {
    /// A stable key for the column. Carried for the builder's
    /// `updateCell(row, key, cell)` lookup and JSON round-tripping; the emitter
    /// itself addresses columns positionally, so the key has no effect on output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Character width (Excel width units). Omit for the default width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// Outline level (1..=7) for grouped columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Default style for the whole column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<CellStyle>,
}

/// A row: its cells plus optional outline level, height and totals flag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// Outline level (1..=7) for grouped/sub-total rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Marks a totals/footer row — applies a bold + top-border default style
    /// without restating it on every cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_total: Option<bool>,
}

/// A typed cell value. The `type` tag selects the variant; each carries a native
/// Excel type so Excel sorts/sums/filters correctly (never pre-formatted text).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Cell {
    /// Inline string text.
    String {
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<CellStyle>,
    },
    /// A real number with an optional number format.
    Number {
        value: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<NumberFormat>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<CellStyle>,
    },
    /// A money amount in integer minor units, rendered via `currency`.
    Currency {
        value: i64,
        currency: CurrencyFormat,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<CellStyle>,
    },
    /// A ratio rendered as a percentage (`0.15` → `15%`).
    Percent {
        value: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decimals: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<CellStyle>,
    },
    /// A real date: an ISO-8601 string (`"2026-06-20"`) or an Excel serial number.
    Date {
        value: DateValue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<DateFormat>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<CellStyle>,
    },
    /// A native boolean (`TRUE`/`FALSE`).
    Boolean {
        value: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<CellStyle>,
    },
    /// An empty cell that still carries a style (e.g. a filled header gap).
    Blank {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<CellStyle>,
    },
}

/// A date cell's value: either an Excel serial day-number or an ISO-8601 string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DateValue {
    /// An already-computed Excel serial number (days since 1899-12-30).
    Serial(f64),
    /// An ISO-8601 date (`YYYY-MM-DD`) or datetime (`YYYY-MM-DDTHH:MM:SS`).
    Iso(String),
}

/// How a negative number renders, the accountant conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Negative {
    /// Red text, leading minus.
    Red,
    /// Parenthesised, black.
    Parens,
    /// Leading minus, black (Excel default).
    Minus,
    /// Red and parenthesised.
    RedParens,
}

/// Currency formatting intent. `code` + `locale` are inputs; the writer maps them
/// to the correct OOXML number-format code (symbol, placement, separators).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CurrencyFormat {
    /// ISO-4217 code, e.g. `"MXN"`, `"USD"`, `"EUR"`, `"BRL"`.
    pub code: String,
    /// BCP-47 locale for grouping/symbol placement. Falls back to the sheet /
    /// workbook locale, then `"en-US"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Decimal places. Defaults to 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u32>,
    /// How negatives render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative: Option<Negative>,
    /// Show the currency symbol (`true`, default) or the ISO code (`false`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<bool>,
}

/// Plain-number formatting intent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumberFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u32>,
    /// Thousands grouping. Defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grouped: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative: Option<Negative>,
    /// Escape hatch: a raw Excel number-format code, used verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// Date formatting intent: a semantic kind, or a raw Excel date code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DateFormat {
    /// Semantic kind (`date` / `datetime` / `month-year`). Defaults to `date`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DateKind>,
    /// Escape hatch: a raw Excel date code, e.g. `"dd/mm/yyyy"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// Semantic date format kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DateKind {
    /// Locale short date.
    Date,
    /// Locale date + time.
    Datetime,
    /// `mmm-yy`.
    MonthYear,
}

/// Per-cell / per-row / per-column / per-sheet visual styling. Resolution is
/// most-specific-wins: cell → row (`isTotal`) → column → sheet default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<Font>,
    /// Background fill `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
}

/// Font styling. All fields optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Font {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Font colour `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Font family name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Cell alignment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Align {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<HAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<VAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
}

/// Horizontal alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HAlign {
    Left,
    Center,
    Right,
}

/// Vertical alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

/// The four cell borders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Border {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<BorderEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<BorderEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<BorderEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<BorderEdge>,
}

/// One border edge: line weight and colour.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BorderEdge {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<BorderStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Border line weights supported in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BorderStyle {
    Thin,
    Medium,
    Thick,
    Double,
}

/// Document-level metadata + global write options.
#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    pub meta: DocMeta,
    /// When set (and the crate is built with the `encrypt` feature), the written
    /// `.xlsx` is wrapped in ECMA-376 Agile Encryption protected by this password.
    /// Without the feature it is ignored. Encrypting makes output non-deterministic
    /// (random salts/keys), unlike the plain writer.
    pub password: Option<String>,
}

/// Workbook metadata written to the OPC core/app parts. All fields optional.
#[derive(Debug, Clone, Default)]
pub struct DocMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub company: Option<String>,
}
