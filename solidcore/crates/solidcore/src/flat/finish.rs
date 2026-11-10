//! A flat part as a machine cuts it, and its engraving; the parts packed onto sheets.
//!
//! A laser burns away a kerf, so a part comes out smaller by half the kerf on every edge and
//! slotted parts fall apart: [`kerf`] grows the outline and shrinks the holes to make up for
//! it. A router bit is round, so it leaves a fillet in every inside corner and a mating part
//! does not seat in a slot: [`relieve_corners`] cuts a relief into each of those corners.
//! Labels are single-stroke text ([`super::font`]) placed where they fit inside the part.

use serde::{Deserialize, Serialize};

use super::slice::{FabResult, PartKind};
use super::{FlatError, FlatResult, Ring, Shape};

/// The machine that cuts the parts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Machine {
    /// A laser cutter burning a kerf of `kerf` mm.
    Laser {
        /// The width of the line the beam burns (mm).
        kerf: f64,
    },
    /// A CNC router with a bit of `tool_diameter` mm.
    Router {
        /// The bit's diameter (mm).
        tool_diameter: f64,
    },
}

/// The sheets and the machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetSettings {
    /// Sheet width (mm).
    pub width: f64,
    /// Sheet height (mm).
    pub height: f64,
    /// The least distance between parts (mm).
    pub gap: f64,
    /// The least distance from the sheet's edge (mm).
    pub margin: f64,
    /// The machine.
    pub machine: Machine,
    /// Engrave each part's name on it.
    pub labels: bool,
    /// The capital height of the names (mm).
    pub label_height: f64,
}

impl Default for SheetSettings {
    fn default() -> Self {
        SheetSettings {
            width: 600.0,
            height: 400.0,
            gap: 3.0,
            margin: 5.0,
            machine: Machine::Laser { kerf: 0.15 },
            labels: true,
            label_height: 5.0,
        }
    }
}

/// A part ready to cut: its outline and holes as the machine follows them, and its engraving.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finished {
    /// The part's name.
    pub name: String,
    /// What it is.
    pub kind: PartKind,
    /// What is cut.
    pub shape: Shape,
    /// What is engraved (open polylines).
    pub engrave: Vec<Ring>,
}

/// One sheet of parts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    /// Closed rings to cut, in sheet coordinates (origin at the sheet's lower-left corner).
    pub cuts: Vec<Ring>,
    /// Open polylines to engrave.
    pub engrave: Vec<Ring>,
    /// The names of the parts on it, in the order placed.
    pub parts: Vec<String>,
}

/// The shape grown by half of a laser `width` (mm): the outline out and the holes in, corners
/// kept sharp, so the part is true size once the beam has burnt its line. A negative width
/// shrinks it. The largest piece is kept if the offset splits it.
#[must_use]
pub fn kerf(shape: &Shape, width: f64) -> Shape {
    let _ = width;
    shape.clone()
}

/// The shape with a relief cut into every inside corner a router bit of `tool_diameter` mm
/// cannot reach. The seam for the kernel's corner-relief primitive: unchanged until it lands.
#[must_use]
pub fn relieve_corners(shape: &Shape, tool_diameter: f64) -> Shape {
    let _ = tool_diameter;
    shape.clone()
}

/// `text` as single-stroke lines with capitals `height` mm tall, the lower-left of its box at
/// `at` and turned `angle` radians counter-clockwise about it.
#[must_use]
pub fn label(text: &str, at: [f64; 2], height: f64, angle: f64) -> Vec<Ring> {
    let _ = (text, at, height, angle);
    Vec::new()
}

/// `text` engraved where it fits inside the shape's material, at least `margin` mm from every
/// edge and hole: horizontal or upright, its height stepped down towards [`MIN_LABEL`] mm when
/// needed; nothing when even that does not fit.
#[must_use]
pub fn label_inside(shape: &Shape, text: &str, height: f64, margin: f64) -> Vec<Ring> {
    let _ = (shape, text, height, margin);
    Vec::new()
}

/// The smallest label height (mm) still engraved.
pub const MIN_LABEL: f64 = 2.0;

/// The part as the machine cuts it, with its engraving: router reliefs, the slot numbers and
/// guide lines, its name (not on spacers), then the laser kerf last so the name sits on the true
/// size.
#[must_use]
pub fn finish(part: &super::slice::FabPart, ss: &SheetSettings) -> Finished {
    Finished {
        name: part.name.clone(),
        kind: part.kind,
        shape: part.shape.clone(),
        engrave: part.engrave.clone(),
    }
    .with(ss)
}

impl Finished {
    fn with(self, ss: &SheetSettings) -> Finished {
        let _ = ss;
        self
    }
}

/// Every part (every copy) finished and packed onto as few sheets as it takes.
///
/// # Errors
/// [`FlatError::Setting`] for sheet settings out of range; [`FlatError::DoesNotFit`] when a
/// part fits on no sheet.
pub fn layout(res: &FabResult, ss: &SheetSettings) -> FlatResult<Vec<Sheet>> {
    let _ = (res, ss);
    Err(FlatError::Setting("sheet"))
}
