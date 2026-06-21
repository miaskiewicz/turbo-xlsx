//! OPC packaging: turn a validated [`Workbook`] into the set of XML parts an
//! `.xlsx` contains, then ZIP them. Also the home of the public [`WriteResult`].
//!
//! The part graph is the minimal conformant set Excel needs: a content-type map,
//! the package + workbook relationship files, `docProps` metadata, the shared
//! `styles.xml`, and one `worksheets/sheetN.xml` per sheet.

use crate::drawing;
use crate::error::{Diagnostics, Result};
use crate::model::{DocMeta, Sheet, Workbook, WriteOptions};
use crate::style::StyleTable;
use crate::worksheet::write_sheet;
use crate::xml::escape;
use crate::zip::{self, Part};

/// The default locale used when neither the workbook nor a cell supplies one.
pub const DEFAULT_LOCALE: &str = "en-US";

/// The result of a write: the `.xlsx` bytes plus non-fatal diagnostics.
#[derive(Debug, Clone)]
pub struct WriteResult {
    /// The OOXML SpreadsheetML (OPC-zipped) document.
    pub xlsx: Vec<u8>,
    /// Non-fatal diagnostics (clamped width, dropped duplicate merge, …).
    pub diagnostics: Diagnostics,
}

/// Build every OPC part for `workbook`, intern its styles, and ZIP it. Assumes
/// the workbook has already passed [`crate::validate::validate`].
pub fn package(
    workbook: &Workbook,
    opts: &WriteOptions,
    diags: &mut Diagnostics,
) -> Result<Vec<u8>> {
    let locale = workbook.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let mut table = StyleTable::new();
    let mut worksheets = Vec::with_capacity(workbook.sheets.len());
    for sheet in &workbook.sheets {
        worksheets.push(write_sheet(sheet, locale, &mut table, diags)?);
    }
    Ok(finish_package(
        &workbook.sheets,
        opts,
        &table.to_xml(),
        &worksheets,
    ))
}

/// ZIP a package from already-emitted worksheet parts + styles. Shared by the
/// batch path and the streaming writer (which emits worksheets incrementally).
pub fn finish_package(
    sheets: &[Sheet],
    opts: &WriteOptions,
    styles_xml: &str,
    worksheets: &[String],
) -> Vec<u8> {
    let parts = assemble_parts(sheets, opts, styles_xml, worksheets);
    zip::build(&parts)
}

/// Lay out every part of the package in a deterministic order.
fn assemble_parts(
    sheets: &[Sheet],
    opts: &WriteOptions,
    styles_xml: &str,
    worksheets: &[String],
) -> Vec<Part> {
    let n = worksheets.len();
    let images = drawing::build(sheets);
    let mut parts = vec![
        text_part(
            "[Content_Types].xml",
            content_types(n, &images.media_exts, images.drawings),
        ),
        text_part("_rels/.rels", root_rels()),
        text_part("docProps/core.xml", core_props(&opts.meta)),
        text_part("docProps/app.xml", app_props(&opts.meta)),
        text_part("xl/workbook.xml", workbook_xml(sheets)),
        text_part("xl/_rels/workbook.xml.rels", workbook_rels(n)),
        text_part("xl/styles.xml", styles_xml.to_string()),
    ];
    for (i, xml) in worksheets.iter().enumerate() {
        parts.push(text_part(
            &format!("xl/worksheets/sheet{}.xml", i + 1),
            xml.clone(),
        ));
    }
    parts.extend(images.parts);
    parts
}

/// Build a ZIP part from a part name and its UTF-8 text.
fn text_part(name: &str, body: String) -> Part {
    Part {
        name: name.to_string(),
        data: body.into_bytes(),
    }
}

/// The `[Content_Types].xml` map: one worksheet override per sheet, plus an image
/// `Default` per distinct media extension and a `drawingN.xml` override per
/// drawing part when the workbook embeds images.
fn content_types(sheets: usize, media_exts: &[&str], drawings: usize) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    );
    s.push_str("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>");
    s.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
    for ext in media_exts {
        s.push_str(&format!(
            "<Default Extension=\"{ext}\" ContentType=\"{}\"/>",
            drawing::content_type(ext)
        ));
    }
    s.push_str("<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>");
    s.push_str("<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");
    for i in 1..=sheets {
        s.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
    }
    for i in 1..=drawings {
        s.push_str(&format!(
            "<Override PartName=\"/xl/drawings/drawing{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\"/>"
        ));
    }
    s.push_str("<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>");
    s.push_str("<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>");
    s.push_str("</Types>");
    s
}

/// The package root relationships: workbook + the two docProps parts.
fn root_rels() -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    s.push_str("<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>");
    s.push_str("<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>");
    s.push_str("<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>");
    s.push_str("</Relationships>");
    s
}

/// `xl/workbook.xml`: the sheet list with stable `sheetId`s and relationship ids.
fn workbook_xml(sheets: &[Sheet]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>",
    );
    for (i, sheet) in sheets.iter().enumerate() {
        let id = i + 1;
        s.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{id}\" r:id=\"rId{id}\"/>",
            escape(&sheet.name)
        ));
    }
    s.push_str("</sheets></workbook>");
    s
}

/// `xl/_rels/workbook.xml.rels`: one relationship per worksheet plus styles.
fn workbook_rels(sheets: usize) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for i in 1..=sheets {
        s.push_str(&format!(
            "<Relationship Id=\"rId{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{i}.xml\"/>"
        ));
    }
    s.push_str(&format!(
        "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
        sheets + 1
    ));
    s.push_str("</Relationships>");
    s
}

/// `docProps/core.xml`: title / author / subject, each emitted only when set.
fn core_props(meta: &DocMeta) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">",
    );
    push_opt(&mut s, "dc:title", meta.title.as_deref());
    push_opt(&mut s, "dc:creator", meta.author.as_deref());
    push_opt(&mut s, "dc:subject", meta.subject.as_deref());
    s.push_str("</cp:coreProperties>");
    s
}

/// `docProps/app.xml`: the producing application and optional company.
fn app_props(meta: &DocMeta) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>turbo-xlsx</Application>",
    );
    push_opt(&mut s, "Company", meta.company.as_deref());
    s.push_str("</Properties>");
    s
}

/// Append `<tag>escaped</tag>` only when `value` is present.
fn push_opt(out: &mut String, tag: &str, value: Option<&str>) {
    if let Some(v) = value {
        out.push_str(&format!("<{tag}>{}</{tag}>", escape(v)));
    }
}
