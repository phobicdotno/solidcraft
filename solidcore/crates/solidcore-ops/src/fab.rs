//! Slice for Laser / CNC: a solid cut into flat parts to cut from sheet stock, the parts finished
//! for the machine and packed onto sheets.
//!
//! A recipe of kernel calls (`solidcore::flat`): the body's section by planes
//! ([`section::BrepSection`]), the build style that turns the sections into parts
//! ([`slice::make`]), then each part finished and every part packed ([`finish::layout`]). It
//! computes no geometric quantity.
//!
//! Each call answers twice: the outer [`Result`] fails when the session or the kernel does, the
//! inner one when the slicing itself refuses (a setting out of range, nothing to cut, a part that
//! fits on no sheet), with the [`FlatError`] that says which.

use solidcore::flat::{finish, section, slice};

pub use solidcore::flat::finish::{Finished, Machine, Sheet, SheetSettings};
pub use solidcore::flat::nest::Placement;
pub use solidcore::flat::slice::{
    Axis, FabPart, FabResult, FabSettings, Frame, Mark, PartKind, Style, Warning,
};
pub use solidcore::flat::{FlatError, Region, Ring, Shape, P2};

use crate::body::Body;
use crate::session::{with_body, Answer};
use crate::Result;

/// A slicing or its refusal, as a kernel call answers inside the session.
struct Out<T>(std::result::Result<T, FlatError>);

impl<T> Answer for Out<T> {
    fn refusal(&self) -> Option<String> {
        self.0.as_ref().err().map(ToString::to_string)
    }
}

/// `body` cut into flat parts as `settings` say. `body` is unchanged.
///
/// # Errors
/// The session's and the kernel's (outer); the slicing's refusal (inner).
pub fn slice_body(
    body: &Body,
    settings: &FabSettings,
) -> Result<std::result::Result<FabResult, FlatError>> {
    let out = with_body("fab_slice_body", body, |k, b| {
        let mut model = match section::BrepSection::new(k, b, settings.chord) {
            Ok(m) => m,
            Err(e) => return Ok(Out(Err(e))),
        };
        Ok(Out(slice::make(&mut model, settings)))
    })?;
    Ok(out.0)
}

/// The parts of `result` finished for the machine of `sheets` and packed onto sheets.
///
/// # Errors
/// The slicing's refusal: sheet settings out of range, a part that fits on no sheet.
pub fn layout_sheets(
    result: &FabResult,
    sheets: &SheetSettings,
) -> std::result::Result<Vec<Sheet>, FlatError> {
    finish::layout(result, sheets)
}

/// One part finished for the machine of `sheets`: what is cut and what is engraved, in the
/// part's own frame.
#[must_use]
pub fn finish_part(part: &FabPart, sheets: &SheetSettings) -> Finished {
    finish::finish(part, sheets)
}

/// The area of a part's material (mm²): its outline less its holes.
#[must_use]
pub fn part_area(shape: &Shape) -> f64 {
    solidcore::flat::region::shape_area(shape)
}

/// The bounds `[xmin, ymin, xmax, ymax]` of a part's outline in its own frame, `None` without
/// points.
#[must_use]
pub fn part_bounds(shape: &Shape) -> Option<[f64; 4]> {
    solidcore::flat::region::ring_bounds(std::iter::once(&shape.outer))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ball of radius 50 on the XY plane, cut by the recipe in every style, finished for both
    /// machines and packed: the parts are those the kernel makes, the body is untouched.
    #[test]
    fn a_ball_is_sliced_finished_and_packed_in_every_style() {
        let ball = crate::body::make_sphere([0.0, 0.0, 50.0], 50.0).unwrap();
        let volume = ball.volume(1e-6).unwrap();
        for style in [Style::Stacked, Style::Rod, Style::Base, Style::Back, Style::Waffle] {
            let axis = if style == Style::Stacked { Axis::Z } else { Axis::X };
            let s = FabSettings { style, axis, thickness: 5.0, count: 5, ..FabSettings::default() };
            let res = slice_body(&ball, &s).unwrap().unwrap();
            let slices = res.parts.iter().filter(|p| p.kind == PartKind::Slice).count();
            assert_eq!(slices, if style == Style::Stacked { 20 } else if style == Style::Waffle { 10 } else { 5 }, "{style:?}");
            for p in &res.parts {
                assert!(part_area(&p.shape) > 0.0);
                assert!(part_bounds(&p.shape).is_some());
            }
            for machine in [Machine::Laser { kerf: 0.15 }, Machine::Router { tool_diameter: 3.175 }] {
                let ss = SheetSettings { machine, ..SheetSettings::default() };
                let sheets = layout_sheets(&res, &ss).unwrap();
                let placed: usize = sheets.iter().map(|sh| sh.parts.len()).sum();
                assert_eq!(placed as u64, res.count(), "{style:?} {machine:?}");
            }
        }
        assert!((ball.volume(1e-6).unwrap() - volume).abs() < 1e-9 * volume);
    }

    #[test]
    fn a_bad_setting_is_the_slicings_refusal_not_the_sessions() {
        let ball = crate::body::make_sphere([0.0, 0.0, 0.0], 10.0).unwrap();
        let s = FabSettings { thickness: -1.0, ..FabSettings::default() };
        assert_eq!(slice_body(&ball, &s).unwrap().unwrap_err(), FlatError::Setting("thickness"));
        let s = FabSettings { chord: 0.0, ..FabSettings::default() };
        assert!(matches!(slice_body(&ball, &s).unwrap(), Err(FlatError::Setting(_))));
    }
}
