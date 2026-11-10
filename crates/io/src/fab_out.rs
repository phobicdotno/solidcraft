//! Sheets of flat parts for a laser cutter or a CNC router (Slice for Laser / CNC), as DXF and
//! SVG: what is cut in red on layer CUT, what is engraved in blue on layer ENGRAVE (the colours
//! laser software maps to cut and engrave by default), and the sheet's own outline in grey on
//! layer SHEET, which is not cut.

use std::fmt::Write as _;

use solidcore_record::Vec2;

use crate::dxf_out::{DxfEntity, DxfLayer, DxfShape, write_dxf_drawing};
use crate::{IoError, Result};

/// The DXF layer of the cuts (AutoCAD colour 1, red).
pub const CUT_LAYER: &str = "CUT";
/// The DXF layer of the engraving (colour 5, blue).
pub const ENGRAVE_LAYER: &str = "ENGRAVE";
/// The DXF layer of the sheet's outline (colour 8, grey): not cut.
pub const SHEET_LAYER: &str = "SHEET";
/// The SVG stroke of the cuts.
pub const CUT_COLOUR: &str = "#ff0000";
/// The SVG stroke of the engraving.
pub const ENGRAVE_COLOUR: &str = "#0000ff";

/// The most points one sheet's file may carry.
pub const MAX_SHEET_POINTS: usize = 2_000_000;

/// One sheet's geometry in millimetres, its origin at the sheet's lower-left corner.
#[derive(Clone, Copy, Debug)]
pub struct SheetDrawing<'a> {
    /// The sheet's width.
    pub width: f64,
    /// The sheet's height.
    pub height: f64,
    /// Closed rings to cut (the closing point not repeated).
    pub cuts: &'a [Vec<[f64; 2]>],
    /// Open polylines to engrave.
    pub engrave: &'a [Vec<[f64; 2]>],
    /// The names of the parts on the sheet (the SVG's title).
    pub parts: &'a [String],
}

impl SheetDrawing<'_> {
    fn check(&self) -> Result<()> {
        let finite = |p: &[f64; 2]| p.iter().all(|c| c.is_finite());
        if !(self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0) {
            return Err(IoError::Invalid("the sheet size must be finite and positive".into()));
        }
        let points: usize = self.cuts.iter().chain(self.engrave).map(Vec::len).sum();
        if points > MAX_SHEET_POINTS {
            return Err(IoError::Invalid("the sheet has too many points to write".into()));
        }
        if !self.cuts.iter().chain(self.engrave).flatten().all(finite) {
            return Err(IoError::Invalid("the sheet has a point that is not a number".into()));
        }
        Ok(())
    }
}

fn v(p: &[f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

/// The sheet as DXF text (AutoCAD 2000, millimetres): the sheet's outline on SHEET, each cut ring
/// a closed LWPOLYLINE on CUT, each engraved line an open LWPOLYLINE on ENGRAVE. Rings with fewer
/// than three points and lines with fewer than two are left out.
///
/// # Errors
/// [`IoError::Invalid`] for a sheet size that is not positive, a point that is not finite, or
/// more than [`MAX_SHEET_POINTS`] points.
pub fn sheet_dxf(sheet: &SheetDrawing<'_>) -> Result<String> {
    sheet.check()?;
    let layers = [DxfLayer::new(CUT_LAYER, 1), DxfLayer::new(ENGRAVE_LAYER, 5), DxfLayer::new(SHEET_LAYER, 8)];
    let (w, h) = (sheet.width, sheet.height);
    let mut entities = vec![DxfEntity {
        layer: SHEET_LAYER.into(),
        shape: DxfShape::LwPolyline { verts: [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].iter().map(|p| (v(p), 0.0)).collect(), closed: true },
    }];
    for ring in sheet.cuts.iter().filter(|r| r.len() >= 3) {
        entities.push(DxfEntity { layer: CUT_LAYER.into(), shape: DxfShape::LwPolyline { verts: ring.iter().map(|p| (v(p), 0.0)).collect(), closed: true } });
    }
    for line in sheet.engrave.iter().filter(|l| l.len() >= 2) {
        entities.push(DxfEntity { layer: ENGRAVE_LAYER.into(), shape: DxfShape::LwPolyline { verts: line.iter().map(|p| (v(p), 0.0)).collect(), closed: false } });
    }
    write_dxf_drawing(&layers, &entities)
}

/// Text with `&`, `<`, `>`, `"` and `'` written as XML entities.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// One SVG path: SVG's y runs down, so a sheet point `(x, y)` is drawn at `(x, height - y)`.
fn path_d(points: &[[f64; 2]], height: f64, closed: bool) -> String {
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let _ = write!(d, "{}{:.4} {:.4}", if i == 0 { "M" } else { " L" }, p[0], height - p[1]);
    }
    if closed {
        d.push_str(" Z");
    }
    d
}

/// The sheet as an SVG document a millimetre to the user unit, `width` x `height` mm: a group
/// `cut` stroked red with one closed path per cut ring, a group `engrave` stroked blue with one
/// open path per engraved line, the part names as its title.
///
/// # Errors
/// As [`sheet_dxf`].
pub fn sheet_svg(sheet: &SheetDrawing<'_>) -> Result<String> {
    sheet.check()?;
    let (w, h) = (sheet.width, sheet.height);
    let mut s = String::new();
    let _ = writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}">"#);
    let _ = writeln!(s, "<title>{}</title>", xml_escape(&sheet.parts.join(", ")));
    let _ = writeln!(s, r#"<g id="cut" fill="none" stroke="{CUT_COLOUR}" stroke-width="0.1">"#);
    for ring in sheet.cuts.iter().filter(|r| r.len() >= 3) {
        let _ = writeln!(s, r#"<path d="{}"/>"#, path_d(ring, h, true));
    }
    let _ = writeln!(s, "</g>");
    let _ = writeln!(s, r#"<g id="engrave" fill="none" stroke="{ENGRAVE_COLOUR}" stroke-width="0.1">"#);
    for line in sheet.engrave.iter().filter(|l| l.len() >= 2) {
        let _ = writeln!(s, r#"<path d="{}"/>"#, path_d(line, h, false));
    }
    let _ = writeln!(s, "</g>");
    let _ = writeln!(s, "</svg>");
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Vec<Vec<[f64; 2]>>, Vec<Vec<[f64; 2]>>, Vec<String>) {
        let cuts = vec![vec![[10.0, 10.0], [110.0, 10.0], [110.0, 70.0], [10.0, 70.0]], vec![[50.0, 30.0], [50.0, 40.0], [60.0, 40.0], [60.0, 30.0]]];
        let engrave = vec![vec![[20.0, 20.0], [30.0, 25.0]], vec![[1.0, 1.0]]];
        (cuts, engrave, vec!["L1".into(), "a<b & c".into()])
    }

    #[test]
    fn the_dxf_has_red_cuts_blue_engraving_and_a_grey_sheet() {
        let (cuts, engrave, parts) = sample();
        let text = sheet_dxf(&SheetDrawing { width: 600.0, height: 400.0, cuts: &cuts, engrave: &engrave, parts: &parts }).unwrap();
        // the layer table: name (2) then colour (62)
        for (layer, colour) in [(CUT_LAYER, "1"), (ENGRAVE_LAYER, "5"), (SHEET_LAYER, "8")] {
            let lines: Vec<&str> = text.lines().map(str::trim).collect();
            let at = lines.windows(2).position(|w| w[0] == "2" && w[1] == layer).unwrap_or_else(|| panic!("no layer {layer}"));
            let colour_at = lines[at..].windows(2).position(|w| w[0] == "62").unwrap();
            assert_eq!(lines[at + colour_at + 1], colour, "{layer}");
        }
        let geoms = crate::read_dxf(&text).unwrap();
        // the sheet, two cut rings and one engraved line (the one-point line is left out)
        assert_eq!(geoms.len(), 4, "{geoms:?}");
    }

    #[test]
    fn the_svg_flips_y_and_escapes_the_title() {
        let (cuts, engrave, parts) = sample();
        let svg = sheet_svg(&SheetDrawing { width: 600.0, height: 400.0, cuts: &cuts, engrave: &engrave, parts: &parts }).unwrap();
        assert!(svg.contains(r#"width="600mm" height="400mm" viewBox="0 0 600 400""#));
        assert!(svg.contains("<title>L1, a&lt;b &amp; c</title>"));
        assert_eq!(svg.matches("<path").count(), 3);
        assert!(svg.contains("M10.0000 390.0000 L110.0000 390.0000"));
        assert!(svg.contains(r##"stroke="#ff0000""##) && svg.contains(r##"stroke="#0000ff""##));
        let back = crate::read_svg(&svg).unwrap();
        assert_eq!(back.len(), 3, "{back:?}");
    }

    #[test]
    fn bad_sheets_are_refused() {
        let nan = vec![vec![[f64::NAN, 0.0], [1.0, 0.0], [1.0, 1.0]]];
        for (w, h, cuts) in [(0.0, 10.0, vec![]), (10.0, f64::INFINITY, vec![]), (10.0, 10.0, nan)] {
            let d = SheetDrawing { width: w, height: h, cuts: &cuts, engrave: &[], parts: &[] };
            assert!(sheet_dxf(&d).is_err());
            assert!(sheet_svg(&d).is_err());
        }
    }
}
