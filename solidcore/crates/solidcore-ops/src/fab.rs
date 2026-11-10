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
