//! Style resolution and the `xl/styles.xml` table.
//!
//! Two jobs live here. First, **resolution**: the effective style of a cell is
//! the column default, overlaid by the `isTotal` row convention (bold + top
//! border), overlaid by the cell's own style — most specific wins, merged
//! field-by-field. Second, **interning**: every distinct font / fill / border /
//! number-format / combined record is deduplicated into the flat tables OOXML's
//! `styles.xml` requires, and each cell gets back the `s=""` index of its
//! combined record (an `xf`).

use std::collections::HashMap;

use crate::error::{ErrorCode, Result, TurboXlsxError};
use crate::model::{Align, Border, BorderEdge, BorderStyle, CellStyle, Font, HAlign, VAlign};
use crate::numfmt::ResolvedFmt;
use crate::xml::escape;

/// First id OOXML reserves for caller-defined (custom) number formats.
const CUSTOM_NUMFMT_BASE: u32 = 164;

/// Merge the layered styles into one effective [`CellStyle`]: column default →
/// `isTotal` convention → the cell's own style, least specific first.
pub fn resolve(column: Option<&CellStyle>, is_total: bool, cell: Option<&CellStyle>) -> CellStyle {
    let mut out = column.cloned().unwrap_or_default();
    if is_total {
        overlay(&mut out, &total_style());
    }
    if let Some(c) = cell {
        overlay(&mut out, c);
    }
    out
}

/// Whether a resolved style carries nothing — no font, fill, alignment or
/// border. Such cells differ only by number format, the fast-path cache key.
fn is_empty_style(s: &CellStyle) -> bool {
    s.font.is_none() && s.fill.is_none() && s.align.is_none() && s.border.is_none()
}

/// The implicit style a totals row carries: bold text and a thin top border.
fn total_style() -> CellStyle {
    CellStyle {
        font: Some(Font {
            bold: Some(true),
            ..Font::default()
        }),
        border: Some(Border {
            top: Some(BorderEdge {
                style: Some(BorderStyle::Thin),
                color: None,
            }),
            ..Border::default()
        }),
        ..CellStyle::default()
    }
}

/// Overlay `ov` onto `base`, deep-merging nested font/align/border records so an
/// outer layer's bold and an inner layer's colour both survive.
fn overlay(base: &mut CellStyle, ov: &CellStyle) {
    merge_font(&mut base.font, &ov.font);
    if ov.fill.is_some() {
        base.fill = ov.fill.clone();
    }
    merge_align(&mut base.align, &ov.align);
    merge_border(&mut base.border, &ov.border);
}

/// Deep-merge an optional font: present sub-fields of `ov` replace `base`'s.
fn merge_font(base: &mut Option<Font>, ov: &Option<Font>) {
    let Some(ov) = ov else { return };
    let b = base.get_or_insert_with(Font::default);
    set_if_some(&mut b.bold, &ov.bold);
    set_if_some(&mut b.italic, &ov.italic);
    set_if_some(&mut b.size, &ov.size);
    set_if_some(&mut b.color, &ov.color);
    set_if_some(&mut b.name, &ov.name);
}

/// Overwrite `base` with `ov` when the overlay supplies a value, else leave it.
fn set_if_some<T: Clone>(base: &mut Option<T>, ov: &Option<T>) {
    if ov.is_some() {
        *base = ov.clone();
    }
}

/// Deep-merge an optional alignment.
fn merge_align(base: &mut Option<Align>, ov: &Option<Align>) {
    let Some(ov) = ov else { return };
    let b = base.get_or_insert_with(Align::default);
    set_if_some(&mut b.horizontal, &ov.horizontal);
    set_if_some(&mut b.vertical, &ov.vertical);
    set_if_some(&mut b.wrap, &ov.wrap);
}

/// Deep-merge an optional border, edge by edge.
fn merge_border(base: &mut Option<Border>, ov: &Option<Border>) {
    let Some(ov) = ov else { return };
    let b = base.get_or_insert_with(Border::default);
    merge_edge(&mut b.top, &ov.top);
    merge_edge(&mut b.bottom, &ov.bottom);
    merge_edge(&mut b.left, &ov.left);
    merge_edge(&mut b.right, &ov.right);
}

/// Replace a border edge when the overlay supplies one.
fn merge_edge(base: &mut Option<BorderEdge>, ov: &Option<BorderEdge>) {
    if ov.is_some() {
        *base = ov.clone();
    }
}

/// Accumulates the deduplicated style tables and hands out `xf` indices.
pub struct StyleTable {
    fonts: Interner,
    fills: Interner,
    borders: Interner,
    xfs: Interner,
    custom_fmts: Vec<(u32, String)>,
    fmt_ids: HashMap<String, u32>,
    /// Fast path: most cells carry no style, so their `xf` index depends only on
    /// the number format. Caching `fmt -> s` collapses the per-cell interning of
    /// a 50k-row sheet (millions of identical lookups) to one entry per format.
    empty_cache: HashMap<ResolvedFmt, usize>,
}

/// A string-keyed deduplicating list: returns a stable index per distinct XML.
#[derive(Default)]
struct Interner {
    items: Vec<String>,
    index: HashMap<String, usize>,
}

impl Interner {
    /// Return the index of `xml`, appending it on first sight.
    fn intern(&mut self, xml: String) -> usize {
        if let Some(&i) = self.index.get(&xml) {
            return i;
        }
        let i = self.items.len();
        self.index.insert(xml.clone(), i);
        self.items.push(xml);
        i
    }
}

impl StyleTable {
    /// A fresh table pre-seeded with the OOXML-mandated defaults: a default font,
    /// the `none` + `gray125` fills, an empty border, and the default `xf` 0.
    pub fn new() -> Self {
        let mut t = StyleTable {
            fonts: Interner::default(),
            fills: Interner::default(),
            borders: Interner::default(),
            xfs: Interner::default(),
            custom_fmts: Vec::new(),
            fmt_ids: HashMap::new(),
            empty_cache: HashMap::new(),
        };
        t.fonts.intern(
            "<font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/></font>"
                .to_string(),
        );
        t.fills
            .intern("<fill><patternFill patternType=\"none\"/></fill>".to_string());
        t.fills
            .intern("<fill><patternFill patternType=\"gray125\"/></fill>".to_string());
        t.borders.intern(empty_border());
        t.xfs.intern(default_xf());
        t
    }

    /// Intern the effective style + number format of a cell, returning its `s`
    /// index. The general-format, unstyled cell maps to `xf` 0 (no `s` needed).
    /// Style-less cells take a cached fast path keyed only by the number format.
    pub fn intern(&mut self, style: &CellStyle, fmt: &ResolvedFmt) -> Result<usize> {
        if is_empty_style(style) {
            if let Some(&idx) = self.empty_cache.get(fmt) {
                return Ok(idx);
            }
            let idx = self.intern_record(style, fmt)?;
            self.empty_cache.insert(fmt.clone(), idx);
            return Ok(idx);
        }
        self.intern_record(style, fmt)
    }

    /// Intern a style-less cell by number format alone (the column-cache miss
    /// path). Infallible: an empty style has no colours to validate.
    pub fn intern_format(&mut self, fmt: &ResolvedFmt) -> usize {
        if let Some(&idx) = self.empty_cache.get(fmt) {
            return idx;
        }
        let idx = self
            .intern_record(&CellStyle::default(), fmt)
            .expect("empty style never fails colour validation");
        self.empty_cache.insert(fmt.clone(), idx);
        idx
    }

    /// Build (or look up) the `xf` record for a style + number format.
    fn intern_record(&mut self, style: &CellStyle, fmt: &ResolvedFmt) -> Result<usize> {
        let num_fmt_id = self.fmt_id(fmt);
        let font_id = self.font_id(style.font.as_ref())?;
        let fill_id = self.fill_id(style.fill.as_deref())?;
        let border_id = self.border_id(style.border.as_ref())?;
        let xml = xf_xml(
            num_fmt_id,
            font_id,
            fill_id,
            border_id,
            style.align.as_ref(),
        );
        Ok(self.xfs.intern(xml))
    }

    /// Resolve a number format to its OOXML id, registering a custom code once.
    fn fmt_id(&mut self, fmt: &ResolvedFmt) -> u32 {
        let code = match fmt {
            ResolvedFmt::Builtin(id) => return *id,
            ResolvedFmt::Custom(code) => code,
        };
        if let Some(&id) = self.fmt_ids.get(code) {
            return id;
        }
        let id = CUSTOM_NUMFMT_BASE + self.custom_fmts.len() as u32;
        self.fmt_ids.insert(code.clone(), id);
        self.custom_fmts.push((id, code.clone()));
        id
    }

    /// Intern a font record (or the default font when `None`).
    fn font_id(&mut self, font: Option<&Font>) -> Result<usize> {
        match font {
            None => Ok(0),
            Some(f) => Ok(self.fonts.intern(font_xml(f)?)),
        }
    }

    /// Intern a solid fill of `color` (or the `none` fill 0 when `None`).
    fn fill_id(&mut self, color: Option<&str>) -> Result<usize> {
        match color {
            None => Ok(0),
            Some(c) => Ok(self.fills.intern(solid_fill_xml(c)?)),
        }
    }

    /// Intern a border record (or the empty border 0 when `None`).
    fn border_id(&mut self, border: Option<&Border>) -> Result<usize> {
        match border {
            None => Ok(0),
            Some(b) => Ok(self.borders.intern(border_xml(b)?)),
        }
    }

    /// Serialize the whole table to `xl/styles.xml`.
    pub fn to_xml(&self) -> String {
        let mut s = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
        );
        s.push_str(&num_fmts_xml(&self.custom_fmts));
        push_list(&mut s, "fonts", &self.fonts.items);
        push_list(&mut s, "fills", &self.fills.items);
        push_list(&mut s, "borders", &self.borders.items);
        s.push_str("<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>");
        push_list(&mut s, "cellXfs", &self.xfs.items);
        s.push_str("<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>");
        s.push_str("</styleSheet>");
        s
    }
}

impl Default for StyleTable {
    fn default() -> Self {
        Self::new()
    }
}

/// The empty (all-`none`) border record.
fn empty_border() -> String {
    "<border><left/><right/><top/><bottom/><diagonal/></border>".to_string()
}

/// The default `xf` (general format, default font/fill/border).
fn default_xf() -> String {
    "<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>".to_string()
}

/// The `<numFmts>` block listing every registered custom format code.
fn num_fmts_xml(fmts: &[(u32, String)]) -> String {
    if fmts.is_empty() {
        return String::new();
    }
    let mut s = format!("<numFmts count=\"{}\">", fmts.len());
    for (id, code) in fmts {
        s.push_str(&format!(
            "<numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>",
            escape(code)
        ));
    }
    s.push_str("</numFmts>");
    s
}

/// Wrap a list of interned records in its `<name count="N">…</name>` container.
fn push_list(out: &mut String, name: &str, items: &[String]) {
    out.push_str(&format!("<{name} count=\"{}\">", items.len()));
    for item in items {
        out.push_str(item);
    }
    out.push_str(&format!("</{name}>"));
}

/// Serialize a font record.
fn font_xml(f: &Font) -> Result<String> {
    let mut s = String::from("<font>");
    if f.bold == Some(true) {
        s.push_str("<b/>");
    }
    if f.italic == Some(true) {
        s.push_str("<i/>");
    }
    s.push_str(&format!(
        "<sz val=\"{}\"/>",
        trim_num(f.size.unwrap_or(11.0))
    ));
    if let Some(color) = &f.color {
        s.push_str(&format!("<color rgb=\"{}\"/>", normalize_color(color)?));
    } else {
        s.push_str("<color theme=\"1\"/>");
    }
    let name = f.name.as_deref().unwrap_or("Calibri");
    s.push_str(&format!(
        "<name val=\"{}\"/><family val=\"2\"/></font>",
        escape(name)
    ));
    Ok(s)
}

/// Serialize a solid-fill record of the given colour.
fn solid_fill_xml(color: &str) -> Result<String> {
    Ok(format!(
        "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"{}\"/><bgColor indexed=\"64\"/></patternFill></fill>",
        normalize_color(color)?
    ))
}

/// Serialize a border record, edge by edge.
fn border_xml(b: &Border) -> Result<String> {
    let mut s = String::from("<border>");
    s.push_str(&edge_xml("left", b.left.as_ref())?);
    s.push_str(&edge_xml("right", b.right.as_ref())?);
    s.push_str(&edge_xml("top", b.top.as_ref())?);
    s.push_str(&edge_xml("bottom", b.bottom.as_ref())?);
    s.push_str("<diagonal/></border>");
    Ok(s)
}

/// Serialize one border edge (`<top style="thin"><color…/></top>` or `<top/>`).
fn edge_xml(name: &str, edge: Option<&BorderEdge>) -> Result<String> {
    let Some(edge) = edge else {
        return Ok(format!("<{name}/>"));
    };
    let Some(style) = edge.style else {
        return Ok(format!("<{name}/>"));
    };
    let color = match &edge.color {
        Some(c) => normalize_color(c)?,
        None => "FF000000".to_string(),
    };
    Ok(format!(
        "<{name} style=\"{}\"><color rgb=\"{color}\"/></{name}>",
        border_style_str(style)
    ))
}

/// The OOXML token for a border weight.
fn border_style_str(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::Thin => "thin",
        BorderStyle::Medium => "medium",
        BorderStyle::Thick => "thick",
        BorderStyle::Double => "double",
    }
}

/// Serialize a cell `xf`, emitting only the `apply*` flags that are non-default.
fn xf_xml(
    num_fmt_id: u32,
    font: usize,
    fill: usize,
    border: usize,
    align: Option<&Align>,
) -> String {
    let mut s = format!(
        "<xf numFmtId=\"{num_fmt_id}\" fontId=\"{font}\" fillId=\"{fill}\" borderId=\"{border}\" xfId=\"0\""
    );
    apply_flag(&mut s, "applyNumberFormat", num_fmt_id != 0);
    apply_flag(&mut s, "applyFont", font != 0);
    apply_flag(&mut s, "applyFill", fill != 0);
    apply_flag(&mut s, "applyBorder", border != 0);
    match align.map(alignment_xml) {
        Some(a) => s.push_str(&format!(" applyAlignment=\"1\">{a}</xf>")),
        None => s.push_str("/>"),
    }
    s
}

/// Append ` name="1"` when `on`.
fn apply_flag(s: &mut String, name: &str, on: bool) {
    if on {
        s.push_str(&format!(" {name}=\"1\""));
    }
}

/// Serialize an `<alignment>` element from the model align record.
fn alignment_xml(a: &Align) -> String {
    let mut s = String::from("<alignment");
    if let Some(h) = a.horizontal {
        s.push_str(&format!(" horizontal=\"{}\"", h_align_str(h)));
    }
    if let Some(v) = a.vertical {
        s.push_str(&format!(" vertical=\"{}\"", v_align_str(v)));
    }
    if a.wrap == Some(true) {
        s.push_str(" wrapText=\"1\"");
    }
    s.push_str("/>");
    s
}

/// The OOXML token for a horizontal alignment.
fn h_align_str(h: HAlign) -> &'static str {
    match h {
        HAlign::Left => "left",
        HAlign::Center => "center",
        HAlign::Right => "right",
    }
}

/// The OOXML token for a vertical alignment (`middle` is OOXML `center`).
fn v_align_str(v: VAlign) -> &'static str {
    match v {
        VAlign::Top => "top",
        VAlign::Middle => "center",
        VAlign::Bottom => "bottom",
    }
}

/// Validate and normalize a `#rrggbb` / `rrggbb` colour to OOXML `FFRRGGBB`.
pub fn normalize_color(s: &str) -> Result<String> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    let valid = hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return Err(TurboXlsxError::new(
            ErrorCode::BadColor,
            format!("invalid colour {s:?}; expected #rrggbb hex"),
        ));
    }
    Ok(format!("FF{}", hex.to_ascii_uppercase()))
}

/// Format an `f64` without a trailing `.0`, so `11.0` serializes as `11` while a
/// fractional size keeps its decimals.
pub fn trim_num(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Align, Border, BorderEdge, BorderStyle, CellStyle, Font, HAlign, VAlign};
    use crate::numfmt::ResolvedFmt;

    fn edge(style: Option<BorderStyle>, color: Option<&str>) -> BorderEdge {
        BorderEdge {
            style,
            color: color.map(str::to_string),
        }
    }

    #[test]
    fn resolve_layers_and_deep_merges() {
        let column = CellStyle {
            fill: Some("#eeeeee".into()),
            font: Some(Font {
                size: Some(9.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        let cell = CellStyle {
            font: Some(Font {
                italic: Some(true),
                color: Some("#ff0000".into()),
                name: Some("Arial".into()),
                ..Default::default()
            }),
            align: Some(Align {
                horizontal: Some(HAlign::Center),
                vertical: Some(VAlign::Middle),
                wrap: Some(true),
            }),
            border: Some(Border {
                bottom: Some(edge(Some(BorderStyle::Medium), Some("#000000"))),
                ..Default::default()
            }),
            ..Default::default()
        };
        let merged = resolve(Some(&column), true, Some(&cell));
        let font = merged.font.unwrap();
        assert_eq!(font.bold, Some(true)); // from isTotal
        assert_eq!(font.italic, Some(true)); // from cell
        assert_eq!(font.size, Some(9.0)); // from column
        assert_eq!(font.color.as_deref(), Some("#ff0000"));
        assert_eq!(font.name.as_deref(), Some("Arial"));
        assert_eq!(merged.fill.as_deref(), Some("#eeeeee"));
        let border = merged.border.unwrap();
        assert!(border.top.is_some()); // from isTotal
        assert!(border.bottom.is_some()); // from cell
        let align = merged.align.unwrap();
        assert_eq!(align.horizontal, Some(HAlign::Center));
        assert_eq!(align.wrap, Some(true));
    }

    #[test]
    fn resolve_with_no_layers_is_default() {
        let merged = resolve(None, false, None);
        assert!(merged.font.is_none());
        assert!(merged.border.is_none());
    }

    #[test]
    fn intern_dedups_and_default_is_zero() {
        let mut t = StyleTable::new();
        let plain = CellStyle::default();
        assert_eq!(t.intern(&plain, &ResolvedFmt::Builtin(0)).unwrap(), 0);
        let styled = CellStyle {
            font: Some(Font {
                bold: Some(true),
                italic: Some(true),
                size: Some(14.0),
                color: Some("#112233".into()),
                name: Some("Inter".into()),
            }),
            fill: Some("#abcdef".into()),
            align: Some(Align {
                horizontal: Some(HAlign::Right),
                vertical: Some(VAlign::Top),
                wrap: None,
            }),
            border: Some(Border {
                top: Some(edge(Some(BorderStyle::Thin), None)),
                bottom: Some(edge(Some(BorderStyle::Double), Some("#00ff00"))),
                left: Some(edge(None, Some("#000000"))),
                right: None,
            }),
        };
        let a = t
            .intern(&styled, &ResolvedFmt::Custom("0.00".into()))
            .unwrap();
        let b = t
            .intern(&styled, &ResolvedFmt::Custom("0.00".into()))
            .unwrap();
        assert_eq!(a, b);
        assert_ne!(a, 0);
    }

    #[test]
    fn to_xml_emits_all_tables() {
        let mut t = StyleTable::new();
        let s1 = CellStyle {
            align: Some(Align {
                vertical: Some(VAlign::Bottom),
                horizontal: Some(HAlign::Left),
                wrap: Some(true),
            }),
            border: Some(Border {
                top: Some(edge(Some(BorderStyle::Medium), None)),
                bottom: Some(edge(Some(BorderStyle::Thick), None)),
                left: None,
                right: None,
            }),
            ..Default::default()
        };
        t.intern(&s1, &ResolvedFmt::Custom("\"$\"#,##0.00".into()))
            .unwrap();
        let xml = t.to_xml();
        assert!(xml.contains("<numFmts count=\"1\">"));
        assert!(xml.contains("<fonts count="));
        assert!(xml.contains("<fills count="));
        assert!(xml.contains("<borders count="));
        assert!(xml.contains("<cellXfs count="));
        assert!(xml.contains("medium"));
        assert!(xml.contains("thick"));
        assert!(xml.contains("wrapText=\"1\""));
        assert!(xml.contains("vertical=\"bottom\""));
    }

    #[test]
    fn to_xml_without_custom_formats_omits_numfmts() {
        let t = StyleTable::new();
        assert!(!t.to_xml().contains("<numFmts"));
    }

    #[test]
    fn color_normalization() {
        assert_eq!(normalize_color("#ffffff").unwrap(), "FFFFFFFF");
        assert_eq!(normalize_color("abcdef").unwrap(), "FFABCDEF");
        assert_eq!(
            normalize_color("fff").unwrap_err().code,
            crate::error::ErrorCode::BadColor
        );
        assert_eq!(
            normalize_color("gggggg").unwrap_err().code,
            crate::error::ErrorCode::BadColor
        );
    }

    #[test]
    fn trim_num_strips_integers() {
        assert_eq!(trim_num(11.0), "11");
        assert_eq!(trim_num(11.5), "11.5");
    }

    #[test]
    fn bad_color_in_font_and_border_propagates() {
        let mut t = StyleTable::new();
        let bad_font = CellStyle {
            font: Some(Font {
                color: Some("zzz".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(t.intern(&bad_font, &ResolvedFmt::Builtin(0)).is_err());
        let bad_fill = CellStyle {
            fill: Some("zzz".into()),
            ..Default::default()
        };
        assert!(t.intern(&bad_fill, &ResolvedFmt::Builtin(0)).is_err());
        let bad_border = CellStyle {
            border: Some(Border {
                top: Some(edge(Some(BorderStyle::Thin), Some("zzz"))),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(t.intern(&bad_border, &ResolvedFmt::Builtin(0)).is_err());
    }
}

#[cfg(test)]
mod default_alignment_tests {
    use super::*;
    use crate::model::{Align, CellStyle, HAlign, VAlign};
    use crate::numfmt::ResolvedFmt;

    #[test]
    fn default_table_and_center_middle_alignment() {
        let mut t = StyleTable::default();
        let s = CellStyle {
            align: Some(Align {
                horizontal: Some(HAlign::Center),
                vertical: Some(VAlign::Middle),
                wrap: None,
            }),
            ..Default::default()
        };
        t.intern(&s, &ResolvedFmt::Builtin(0)).unwrap();
        let xml = t.to_xml();
        assert!(xml.contains("horizontal=\"center\""));
        assert!(xml.contains("vertical=\"center\""));
    }
}
