//! Embedded images: turn a sheet's [`SheetImage`]s into the OOXML drawing parts
//! an `.xlsx` needs — one `xl/drawings/drawingN.xml` per sheet that has images,
//! its `_rels` to the shared `xl/media` blobs, and the worksheet `_rels` that
//! point the sheet at its drawing. Identical image bytes are interned once into a
//! single media part and referenced from every anchor that uses them.
//!
//! Anchors map to SpreadsheetML drawing anchors: a [`ImageAnchor::TwoCell`] spans
//! `from`→`to` cells (resizes with the grid); a [`ImageAnchor::OneCell`] pins the
//! top-left at `at` with a fixed pixel extent (1 px = 9525 EMU at 96 dpi).

use crate::b64;
use crate::model::{CellRef, ImageAnchor, ImageFormat, Sheet, SheetImage};
use crate::xml::escape;
use crate::zip::Part;

/// English Metric Units per pixel at 96 dpi (OOXML's drawing unit).
const EMU_PER_PX: u64 = 9525;

/// The drawing/media/rels parts for a whole package, plus which sheets carry a
/// drawing (so the worksheet emits a `<drawing r:id="rId1"/>` reference).
pub struct ImageParts {
    /// Every image-related part, in deterministic order.
    pub parts: Vec<Part>,
    /// Distinct media extensions present, for `[Content_Types].xml` `Default`s.
    pub media_exts: Vec<&'static str>,
    /// Number of drawing parts emitted, for the content-type overrides.
    pub drawings: usize,
}

/// One interned media blob: its declared extension and decoded bytes.
struct Media {
    ext: &'static str,
    bytes: Vec<u8>,
}

/// Build every image part for `sheets`. Sheets with no images contribute
/// nothing; a sheet with images yields a drawing part, its rels, and a worksheet
/// rels part, all keyed off a per-sheet drawing number.
pub fn build(sheets: &[Sheet]) -> ImageParts {
    let mut media: Vec<Media> = Vec::new();
    let mut parts: Vec<Part> = Vec::new();
    let mut drawings = 0usize;
    for (si, sheet) in sheets.iter().enumerate() {
        if sheet.images.is_empty() {
            continue;
        }
        drawings += 1;
        let refs: Vec<usize> = sheet
            .images
            .iter()
            .map(|img| intern(&mut media, img))
            .collect();
        parts.push(text_part(
            format!("xl/drawings/drawing{drawings}.xml"),
            drawing_xml(&sheet.images),
        ));
        parts.push(text_part(
            format!("xl/drawings/_rels/drawing{drawings}.xml.rels"),
            drawing_rels(&sheet.images, &refs),
        ));
        parts.push(text_part(
            format!("xl/worksheets/_rels/sheet{}.xml.rels", si + 1),
            worksheet_rels(drawings),
        ));
    }
    append_media(&mut parts, &media);
    ImageParts {
        media_exts: distinct_exts(&media),
        drawings,
        parts,
    }
}

/// Intern an image's decoded bytes into `media`, returning its 1-based index.
/// Identical bytes collapse to the same media part. Invalid base64 (only
/// reachable via the unvalidated streaming path) decodes to empty bytes rather
/// than panicking — the part still exists, Excel simply shows a broken image.
fn intern(media: &mut Vec<Media>, img: &SheetImage) -> usize {
    let bytes = b64::decode(&img.data).unwrap_or_default();
    let ext = img.format.ext();
    if let Some(i) = media.iter().position(|m| m.bytes == bytes && m.ext == ext) {
        return i + 1;
    }
    media.push(Media { ext, bytes });
    media.len()
}

/// Push every interned blob as an `xl/media/imageN.<ext>` binary part.
fn append_media(parts: &mut Vec<Part>, media: &[Media]) {
    for (i, m) in media.iter().enumerate() {
        parts.push(Part {
            name: format!("xl/media/image{}.{}", i + 1, m.ext),
            data: m.bytes.clone(),
        });
    }
}

/// The distinct extensions across all media, for content-type `Default`s.
fn distinct_exts(media: &[Media]) -> Vec<&'static str> {
    let mut exts: Vec<&'static str> = Vec::new();
    for m in media {
        if !exts.contains(&m.ext) {
            exts.push(m.ext);
        }
    }
    exts
}

/// The OPC content type for an image extension (drives `[Content_Types].xml`).
pub fn content_type(ext: &str) -> &'static str {
    match ext {
        "png" => ImageFormat::Png.content_type(),
        "jpeg" => ImageFormat::Jpeg.content_type(),
        _ => ImageFormat::Gif.content_type(),
    }
}

/// `xl/drawings/drawingN.xml`: one anchor (two- or one-cell) per image.
fn drawing_xml(images: &[SheetImage]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">",
    );
    for (i, img) in images.iter().enumerate() {
        anchor_xml(&mut s, i, img);
    }
    s.push_str("</xdr:wsDr>");
    s
}

/// Emit one image's anchor element with its embedded `<xdr:pic>`.
fn anchor_xml(s: &mut String, i: usize, img: &SheetImage) {
    let rid = i + 1;
    match img.anchor {
        ImageAnchor::TwoCell { from, to } => {
            s.push_str("<xdr:twoCellAnchor editAs=\"oneCell\">");
            s.push_str(&from_to(&from, &to));
            s.push_str(&pic(i, rid, img.alt.as_deref()));
            s.push_str("<xdr:clientData/></xdr:twoCellAnchor>");
        }
        ImageAnchor::OneCell { at, width, height } => {
            s.push_str("<xdr:oneCellAnchor>");
            s.push_str(&from_marker(&at));
            s.push_str(&format!(
                "<xdr:ext cx=\"{}\" cy=\"{}\"/>",
                px_emu(width),
                px_emu(height)
            ));
            s.push_str(&pic(i, rid, img.alt.as_deref()));
            s.push_str("<xdr:clientData/></xdr:oneCellAnchor>");
        }
    }
}

/// `<xdr:from>`/`<xdr:to>` markers for a two-cell anchor.
fn from_to(from: &CellRef, to: &CellRef) -> String {
    format!("{}{}", from_marker(from), to_marker(to))
}

/// `<xdr:from>` cell marker (zero offsets).
fn from_marker(c: &CellRef) -> String {
    format!("<xdr:from>{}</xdr:from>", marker_body(c))
}

/// `<xdr:to>` cell marker (zero offsets).
fn to_marker(c: &CellRef) -> String {
    format!("<xdr:to>{}</xdr:to>", marker_body(c))
}

/// The shared `<xdr:col>/<xdr:colOff>/<xdr:row>/<xdr:rowOff>` body.
fn marker_body(c: &CellRef) -> String {
    format!(
        "<xdr:col>{}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff>",
        c.col, c.row
    )
}

/// The `<xdr:pic>` element: name/alt metadata, the blip referencing the media
/// rel, and a rectangular shape.
fn pic(i: usize, rid: usize, alt: Option<&str>) -> String {
    let id = i + 1;
    let descr = match alt {
        Some(a) => format!(" descr=\"{}\"", escape(a)),
        None => String::new(),
    };
    format!(
        "<xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"{id}\" name=\"Image {id}\"{descr}/><xdr:cNvPicPr/></xdr:nvPicPr>\
<xdr:blipFill><a:blip xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:embed=\"rId{rid}\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill>\
<xdr:spPr><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic>"
    )
}

/// `xl/drawings/_rels/drawingN.xml.rels`: each anchor's blip → a media part.
fn drawing_rels(images: &[SheetImage], refs: &[usize]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (i, img) in images.iter().enumerate() {
        s.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/image{}.{}\"/>",
            i + 1,
            refs[i],
            img.format.ext()
        ));
    }
    s.push_str("</Relationships>");
    s
}

/// `xl/worksheets/_rels/sheetN.xml.rels`: the worksheet → its drawing part.
fn worksheet_rels(drawing: usize) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing\" Target=\"../drawings/drawing{drawing}.xml\"/></Relationships>"
    )
}

/// Convert a pixel length to EMU.
fn px_emu(px: u32) -> u64 {
    px as u64 * EMU_PER_PX
}

/// Build a text ZIP part from a name and body.
fn text_part(name: String, body: String) -> Part {
    Part {
        name,
        data: body.into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ImageFormat;

    fn img(anchor: ImageAnchor, fmt: ImageFormat, data: &str, alt: Option<&str>) -> SheetImage {
        SheetImage {
            data: data.to_string(),
            format: fmt,
            anchor,
            alt: alt.map(str::to_string),
        }
    }

    fn sheet_with(images: Vec<SheetImage>) -> Sheet {
        Sheet {
            name: "S".to_string(),
            images,
            ..Default::default()
        }
    }

    fn png_b64() -> String {
        b64::encode(b"\x89PNGfake")
    }

    #[test]
    fn no_images_yields_nothing() {
        let parts = build(&[sheet_with(vec![])]);
        assert!(parts.parts.is_empty());
        assert_eq!(parts.drawings, 0);
        assert!(parts.media_exts.is_empty());
    }

    #[test]
    fn two_cell_anchor_emits_drawing_media_and_rels() {
        let anchor = ImageAnchor::TwoCell {
            from: CellRef { col: 0, row: 0 },
            to: CellRef { col: 2, row: 5 },
        };
        let parts = build(&[sheet_with(vec![img(
            anchor,
            ImageFormat::Png,
            &png_b64(),
            Some("a logo & co"),
        )])]);
        assert_eq!(parts.drawings, 1);
        assert_eq!(parts.media_exts, vec!["png"]);
        let names: Vec<&str> = parts.parts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"xl/drawings/drawing1.xml"));
        assert!(names.contains(&"xl/drawings/_rels/drawing1.xml.rels"));
        assert!(names.contains(&"xl/worksheets/_rels/sheet1.xml.rels"));
        assert!(names.contains(&"xl/media/image1.png"));
        let drawing = String::from_utf8(
            parts
                .parts
                .iter()
                .find(|p| p.name == "xl/drawings/drawing1.xml")
                .unwrap()
                .data
                .clone(),
        )
        .unwrap();
        assert!(drawing.contains("twoCellAnchor"));
        assert!(drawing.contains("descr=\"a logo &amp; co\""));
        assert!(drawing.contains("r:embed=\"rId1\""));
    }

    #[test]
    fn one_cell_anchor_emits_ext_in_emu_and_no_descr() {
        let anchor = ImageAnchor::OneCell {
            at: CellRef { col: 1, row: 1 },
            width: 100,
            height: 50,
        };
        let parts = build(&[sheet_with(vec![img(
            anchor,
            ImageFormat::Jpeg,
            &b64::encode(b"jpegbytes"),
            None,
        )])]);
        assert_eq!(parts.media_exts, vec!["jpeg"]);
        let drawing = String::from_utf8(parts.parts[0].data.clone()).unwrap();
        assert!(drawing.contains("oneCellAnchor"));
        assert!(drawing.contains(&format!("cx=\"{}\"", 100 * 9525)));
        assert!(drawing.contains(&format!("cy=\"{}\"", 50 * 9525)));
        assert!(!drawing.contains("descr="));
    }

    #[test]
    fn identical_bytes_are_interned_once() {
        let a = img(
            ImageAnchor::OneCell {
                at: CellRef { col: 0, row: 0 },
                width: 10,
                height: 10,
            },
            ImageFormat::Png,
            &png_b64(),
            None,
        );
        let b = img(
            ImageAnchor::OneCell {
                at: CellRef { col: 1, row: 1 },
                width: 10,
                height: 10,
            },
            ImageFormat::Png,
            &png_b64(),
            None,
        );
        let parts = build(&[sheet_with(vec![a, b])]);
        let media: Vec<&str> = parts
            .parts
            .iter()
            .filter(|p| p.name.starts_with("xl/media/"))
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(media, vec!["xl/media/image1.png"]);
    }

    #[test]
    fn distinct_formats_get_distinct_media() {
        let png = img(
            ImageAnchor::OneCell {
                at: CellRef { col: 0, row: 0 },
                width: 10,
                height: 10,
            },
            ImageFormat::Png,
            &png_b64(),
            None,
        );
        let gif = img(
            ImageAnchor::OneCell {
                at: CellRef { col: 0, row: 1 },
                width: 10,
                height: 10,
            },
            ImageFormat::Gif,
            &b64::encode(b"GIF89a"),
            None,
        );
        let parts = build(&[sheet_with(vec![png, gif])]);
        assert_eq!(parts.media_exts, vec!["png", "gif"]);
    }

    #[test]
    fn second_sheet_gets_its_own_drawing_number() {
        let anchor = ImageAnchor::OneCell {
            at: CellRef { col: 0, row: 0 },
            width: 10,
            height: 10,
        };
        let s0 = sheet_with(vec![]);
        let s1 = sheet_with(vec![img(anchor, ImageFormat::Png, &png_b64(), None)]);
        let parts = build(&[s0, s1]);
        assert_eq!(parts.drawings, 1);
        let names: Vec<&str> = parts.parts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"xl/worksheets/_rels/sheet2.xml.rels"));
        assert!(names.contains(&"xl/drawings/drawing1.xml"));
    }

    #[test]
    fn invalid_base64_decodes_to_empty_media() {
        let anchor = ImageAnchor::OneCell {
            at: CellRef { col: 0, row: 0 },
            width: 10,
            height: 10,
        };
        let parts = build(&[sheet_with(vec![img(
            anchor,
            ImageFormat::Png,
            "not valid base64 $$$",
            None,
        )])]);
        let media = parts
            .parts
            .iter()
            .find(|p| p.name == "xl/media/image1.png")
            .unwrap();
        assert!(media.data.is_empty());
    }

    #[test]
    fn content_type_maps_every_ext() {
        assert_eq!(content_type("png"), "image/png");
        assert_eq!(content_type("jpeg"), "image/jpeg");
        assert_eq!(content_type("gif"), "image/gif");
    }

    #[test]
    fn worksheet_rels_targets_drawing() {
        assert!(worksheet_rels(3).contains("../drawings/drawing3.xml"));
    }
}
