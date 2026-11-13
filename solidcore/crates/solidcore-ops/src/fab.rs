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
//!
//! The settings a slicing starts from are chosen here ([`DEFAULT_SETTINGS`], [`DEFAULT_SHEETS`]
//! and the machines' [`DEFAULT_KERF`] and [`DEFAULT_TOOL_DIAMETER`]); the kernel holds none.

use solidcore::flat::{finish, section, slice};

pub use solidcore::flat::finish::{Machine, Sheet, SheetSettings};
pub use solidcore::flat::slice::{Axis, FabResult, FabSettings, Frame, PartKind, Style, Warning};
pub use solidcore::flat::{FlatError, Shape};

use crate::body::Body;
use crate::session::{with_body, Answer};
use crate::Result;

/// The slicing settings a slicing starts from: stacked layers along Z from 3 mm sheet.
pub const DEFAULT_SETTINGS: FabSettings = FabSettings {
    style: Style::Stacked,
    axis: Axis::Z,
    thickness: 3.0,
    spacing: 0.0,
    count: 0,
    clearance: 0.2,
    rod_diameter: 8.0,
    rod_at: None,
    dowels: 2,
    dowel_diameter: 6.0,
    wall: 4.0,
    plate_margin: 10.0,
    guide_outline: true,
    min_area: 4.0,
    chord: 0.01,
};

/// A laser's kerf when none is given (mm).
pub const DEFAULT_KERF: f64 = 0.15;

/// A CNC router bit's diameter when none is given (mm): an eighth of an inch.
pub const DEFAULT_TOOL_DIAMETER: f64 = 3.175;

/// The sheets and machine a layout starts from: 600 x 400 mm sheets cut by a laser.
pub const DEFAULT_SHEETS: SheetSettings = SheetSettings {
    width: 600.0,
    height: 400.0,
    gap: 3.0,
    margin: 5.0,
    machine: Machine::Laser { kerf: DEFAULT_KERF },
    labels: true,
    label_height: 5.0,
};

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

/// The area of a part's material (mm²): its outline less its holes.
#[must_use]
pub fn part_area(shape: &Shape) -> f64 {
    solidcore::flat::region::shape_area(shape)
}

/// The width and height `[w, h]` of a part's outline in its own frame (mm), `None` without
/// points.
#[must_use]
pub fn part_size(shape: &Shape) -> Option<[f64; 2]> {
    solidcore::flat::region::shape_size(shape)
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
        for style in [
            Style::Stacked,
            Style::Rod,
            Style::Base,
            Style::Back,
            Style::Waffle,
        ] {
            let axis = if style == Style::Stacked {
                Axis::Z
            } else {
                Axis::X
            };
            let s = FabSettings {
                style,
                axis,
                thickness: 5.0,
                count: 5,
                ..DEFAULT_SETTINGS
            };
            let res = slice_body(&ball, &s).unwrap().unwrap();
            let slices = res
                .parts
                .iter()
                .filter(|p| p.kind == PartKind::Slice)
                .count();
            assert_eq!(
                slices,
                if style == Style::Stacked {
                    20
                } else if style == Style::Waffle {
                    10
                } else {
                    5
                },
                "{style:?}"
            );
            for p in &res.parts {
                assert!(part_area(&p.shape) > 0.0);
                assert!(part_size(&p.shape).is_some_and(|[w, h]| w > 0.0 && h > 0.0));
            }
            for machine in [
                Machine::Laser { kerf: DEFAULT_KERF },
                Machine::Router {
                    tool_diameter: DEFAULT_TOOL_DIAMETER,
                },
            ] {
                let ss = SheetSettings {
                    machine,
                    ..DEFAULT_SHEETS
                };
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
        let s = FabSettings {
            thickness: -1.0,
            ..DEFAULT_SETTINGS
        };
        assert!(matches!(
            slice_body(&ball, &s).unwrap().unwrap_err(),
            FlatError::Setting {
                name: "thickness",
                ..
            }
        ));
        let s = FabSettings {
            chord: 0.0,
            ..DEFAULT_SETTINGS
        };
        assert!(matches!(
            slice_body(&ball, &s).unwrap(),
            Err(FlatError::Setting { .. })
        ));
    }
}
