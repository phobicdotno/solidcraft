//! Sheets of flat parts for a laser cutter or a CNC router (Slice for Laser / CNC), as DXF and
//! SVG. The files hold only what is cut, in red on layer CUT, and what is engraved, in blue on
//! layer ENGRAVE (the colours laser software maps to cut and engrave by default); the sheet's own
//! outline is not written.
//!
//! The SVG has its own writer next to `paper`'s: it groups the paths by colour (cut, then
//! engrave), which is what laser software reads, where a paper scene groups them by object. It
//! writes its numbers with `paper`'s nanometre writer and refuses a file over
//! [`MAX_DRAWING_BYTES`], as `paper` does.

use std::fmt::Write as _;

use solidcore_record::Vec2;

use crate::dxf_out::{DxfEntity, DxfLayer, DxfShape, write_dxf_drawing};
use crate::paper::{nano, number, push_number};
use crate::{IoError, MAX_DRAWING_BYTES, Result};

/// The DXF layer of the cuts (AutoCAD colour 1, red).
pub const CUT_LAYER: &str = "CUT";
/// The DXF layer of the engraving (colour 5, blue).
pub const ENGRAVE_LAYER: &str = "ENGRAVE";
/// The SVG stroke of the cuts.
pub const CUT_COLOUR: &str = "#ff0000";
/// The SVG stroke of the engraving.
pub const ENGRAVE_COLOUR: &str = "#0000ff";
/// The SVG stroke width, in millimetres. Laser software reads the stroke's colour, not its width;
/// the width only has to show on screen at the sheet's scale.
const STROKE_MM: f64 = 0.1;

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
        if !self.cuts.iter().chain(self.engrave).flatten().all(finite) {
            return Err(IoError::Invalid("the sheet has a point that is not a number".into()));
        }
        Ok(())
    }
}

/// A file over [`MAX_DRAWING_BYTES`] is refused.
fn fits(text: &str, format: &str) -> Result<()> {
    if text.len() > MAX_DRAWING_BYTES {
        return Err(IoError::Invalid(format!("the sheet's {format} would be over {} MiB", MAX_DRAWING_BYTES >> 20)));
    }
    Ok(())
}

fn v(p: &[f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

/// The sheet as DXF text (AutoCAD 2000, millimetres): each cut ring a closed LWPOLYLINE on CUT,
/// each engraved line an open LWPOLYLINE on ENGRAVE. Rings with fewer than three points and lines
/// with fewer than two are left out.
///
/// # Errors
/// [`IoError::Invalid`] for a sheet size that is not positive, a point that is not finite, or a
/// file over [`MAX_DRAWING_BYTES`].
pub fn sheet_dxf(sheet: &SheetDrawing<'_>) -> Result<String> {
    sheet.check()?;
    let layers = [DxfLayer::new(CUT_LAYER, 1), DxfLayer::new(ENGRAVE_LAYER, 5)];
    let mut entities = Vec::new();
    for ring in sheet.cuts.iter().filter(|r| r.len() >= 3) {
        entities.push(DxfEntity {
            layer: CUT_LAYER.into(),
            shape: DxfShape::LwPolyline { verts: ring.iter().map(|p| (v(p), 0.0)).collect(), closed: true },
        });
    }
    for line in sheet.engrave.iter().filter(|l| l.len() >= 2) {
        entities.push(DxfEntity {
            layer: ENGRAVE_LAYER.into(),
            shape: DxfShape::LwPolyline { verts: line.iter().map(|p| (v(p), 0.0)).collect(), closed: false },
        });
    }
    let text = write_dxf_drawing(&layers, &entities)?;
    fits(&text, "DXF")?;
    Ok(text)
}

/// One SVG path onto `out`: SVG's y runs down, so a sheet point `(x, y)` is drawn at
/// `(x, height - y)`, `height` in nanometres.
fn push_path(out: &mut String, points: &[[f64; 2]], height: i64, closed: bool) -> Result<()> {
    out.push_str(r#"<path d=""#);
    for (i, p) in points.iter().enumerate() {
        out.push_str(if i == 0 { "M" } else { " L" });
        push_number(out, nano(p[0])?);
        out.push(' ');
        push_number(out, height - nano(p[1])?);
    }
    if closed {
        out.push_str(" Z");
    }
    out.push_str("\"/>\n");
    Ok(())
}

/// The sheet as an SVG document a millimetre to the user unit, `width` x `height` mm: a group
/// `cut` stroked red with one closed path per cut ring, a group `engrave` stroked blue with one
/// open path per engraved line, the part names as its title.
///
/// # Errors
/// As [`sheet_dxf`].
pub fn sheet_svg(sheet: &SheetDrawing<'_>) -> Result<String> {
    sheet.check()?;
    let height = nano(sheet.height)?;
    let (w, h) = (number(nano(sheet.width)?), number(height));
    let mut s = String::new();
    let _ = writeln!(s, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}">"#);
    let _ = writeln!(s, "<title>{}</title>", crate::threemf::esc(&sheet.parts.join(", ")));
    // a ring needs three points, a line two
    for (id, colour, paths, closed, least) in [("cut", CUT_COLOUR, sheet.cuts, true, 3), ("engrave", ENGRAVE_COLOUR, sheet.engrave, false, 2)] {
        let _ = writeln!(s, r#"<g id="{id}" fill="none" stroke="{colour}" stroke-width="{STROKE_MM}">"#);
        for path in paths.iter().filter(|p| p.len() >= least) {
            push_path(&mut s, path, height, closed)?;
            fits(&s, "SVG")?;
        }
        let _ = writeln!(s, "</g>");
    }
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
    fn the_dxf_has_red_cuts_blue_engraving_and_no_sheet_outline() {
        let (cuts, engrave, parts) = sample();
        let text = sheet_dxf(&SheetDrawing { width: 600.0, height: 400.0, cuts: &cuts, engrave: &engrave, parts: &parts }).unwrap();
        assert!(!text.contains("SHEET"), "no sheet layer or outline");
        // the layer table: name (2) then colour (62)
        for (layer, colour) in [(CUT_LAYER, "1"), (ENGRAVE_LAYER, "5")] {
            let lines: Vec<&str> = text.lines().map(str::trim).collect();
            let at = lines.windows(2).position(|w| w[0] == "2" && w[1] == layer).unwrap_or_else(|| panic!("no layer {layer}"));
            let colour_at = lines[at..].windows(2).position(|w| w[0] == "62").unwrap();
            assert_eq!(lines[at + colour_at + 1], colour, "{layer}");
        }
        let geoms = crate::read_dxf(&text).unwrap();
        // read back as lines: four sides of each cut ring and the one engraved segment (the
        // one-point line is left out)
        assert_eq!(geoms.len(), 4 + 4 + 1, "{geoms:?}");
    }

    #[test]
    fn the_svg_flips_y_and_escapes_the_title() {
        let (cuts, engrave, parts) = sample();
        let svg = sheet_svg(&SheetDrawing { width: 600.0, height: 400.0, cuts: &cuts, engrave: &engrave, parts: &parts }).unwrap();
        assert!(svg.contains(r#"width="600mm" height="400mm" viewBox="0 0 600 400""#));
        assert!(svg.contains("<title>L1, a&lt;b &amp; c</title>"));
        assert_eq!(svg.matches("<path").count(), 3);
        assert!(svg.contains("M10 390 L110 390"), "{svg}");
        assert!(svg.contains(r#"stroke-width="0.1""#));
        assert!(svg.contains(r##"stroke="#ff0000""##) && svg.contains(r##"stroke="#0000ff""##));
        let back = crate::read_svg(&svg).unwrap();
        // four sides of each cut ring and the engraved segment, read with SVG's y pointing down
        assert_eq!(back.len(), 9, "{back:?}");
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

    #[test]
    fn a_sheet_over_the_drawing_size_is_refused() {
        // about 23 bytes a point in the SVG and more in the DXF: 3.5 million points pass 64 MiB
        let ring: Vec<[f64; 2]> = (0..3_500_000_u32).map(|i| [123.456_789 + f64::from(i % 7), 276.543_211]).collect();
        let cuts = vec![ring];
        let d = SheetDrawing { width: 600.0, height: 400.0, cuts: &cuts, engrave: &[], parts: &[] };
        for text in [sheet_dxf(&d), sheet_svg(&d)] {
            let Err(IoError::Invalid(why)) = text else { panic!("a sheet over the size was written") };
            assert!(why.contains("MiB"), "{why}");
        }
    }
}
