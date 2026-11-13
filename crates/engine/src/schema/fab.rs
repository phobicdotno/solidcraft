//! Contracts and result types for the commands in `cmd/fab.rs`: `fab.slice` and
//! `fab.export_sheets` (Slice for Laser / CNC).

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;
use crate::params::bad;

/// The parameters of `cmd` as its contract reads them, with the contract's own words for a value
/// of the wrong shape (so a call refused in strict mode gets the same error with the check off).
/// Keys the contract does not list are left to the parameter check (warned or refused there).
pub(crate) fn read<P: DeserializeOwned>(cmd: &str, p: &Value) -> Result<P> {
    let empty = Value::Object(serde_json::Map::new());
    let p = if p.is_null() { &empty } else { p };
    serde_path_to_error::deserialize::<_, P>(p).map_err(|e| bad(cmd, super::check::parse_issue(&e).message))
}

/// How the model is built back up from flat parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FabStyle {
    /// Layers one sheet thick, glued on top of each other, with dowel holes and engraved guides.
    #[default]
    Stacked,
    /// Slices threaded on a rod, spacer rings between them.
    Rod,
    /// Upright slices with tabs in slots of a base plate.
    Base,
    /// Upright slices with tabs in slots of a back plate (wall mounted).
    Back,
    /// Two sets of upright slices at right angles, slotted half way.
    Waffle,
}

/// A model axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum FabAxis {
    X,
    Y,
    Z,
}

/// The machine that cuts the parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FabMachine {
    /// A laser cutter: parts grow by half the kerf.
    #[default]
    Laser,
    /// A CNC router: inside corners get reliefs for the round bit.
    Cnc,
}

/// The slicing and sheet settings both commands read. Every length is in mm; a key left out
/// takes its default.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
pub struct FabSettingsParams {
    /// The body to slice, by name. Default: the design's only solid body.
    #[serde(default)]
    pub body: Option<String>,
    /// How the model is built back up. Default: stacked.
    #[serde(default)]
    pub style: Option<FabStyle>,
    /// The slicing axis: the stacking direction (stacked), the rod's direction (rod), the slicing
    /// axis of a base or back plate (X or Y only). Default: Z for stacked, X for the other styles.
    /// Ignored for a waffle, which always slices across X and Y.
    #[serde(default)]
    pub axis: Option<FabAxis>,
    /// Sheet thickness (mm), more than 0 and at most 1000. Default: 3.
    #[serde(default)]
    pub thickness: Option<f64>,
    /// Centre-to-centre distance of upright or rod slices (mm): 0 (automatic: four sheets or
    /// an eighth of the body, whichever is more, in whole sheets for rod slices), or from the
    /// sheet thickness to 10000. Default: 0. Ignored for stacked layers, which are one sheet
    /// apart and centred on the body, and when `count` is above 0.
    #[serde(default)]
    pub spacing: Option<f64>,
    /// The number of upright or rod slices (per direction in a waffle), 0 to 500 and no more
    /// than the body's length divided by the sheet thickness; overrides `spacing` when above 0.
    /// Default: 0. Ignored for stacked layers.
    #[serde(default)]
    pub count: Option<u32>,
    /// Play in slots and holes (mm), from 0 to the sheet thickness. Default: 0.2.
    #[serde(default)]
    pub clearance: Option<f64>,
    /// Rod: the rod's diameter (mm), more than 0 and at most 10000. Default: 8.
    #[serde(default)]
    pub rod_diameter: Option<f64>,
    /// Stacked: dowel holes through every layer, 0 (none) to 64. Default: 2.
    #[serde(default)]
    pub dowels: Option<u32>,
    /// Stacked: the dowels' diameter (mm), more than 0 and at most 10000. Default: 6.
    #[serde(default)]
    pub dowel_diameter: Option<f64>,
    /// Material kept around holes, and the width of spacer rings (mm), 0 to 10000; for rod
    /// slices more than half the clearance. Default: 4.
    #[serde(default)]
    pub wall: Option<f64>,
    /// Base or back: how far the plate reaches past the slices (mm), at least; the plate grows
    /// to hold the slice numbers. 0 to 10000. Default: 10.
    #[serde(default)]
    pub plate_margin: Option<f64>,
    /// Stacked: engrave where the smaller neighbouring layer goes. Default: true.
    #[serde(default)]
    pub guide_outline: Option<bool>,
    /// The machine. Default: laser.
    #[serde(default)]
    pub machine: Option<FabMachine>,
    /// Laser: the kerf, the width of the line the beam burns (mm), 0 to 5. Default: 0.15.
    /// Ignored with `machine` cnc.
    #[serde(default)]
    pub kerf: Option<f64>,
    /// CNC router: the bit's diameter (mm), 0 to 50. Default: 3.175. Ignored with `machine` laser.
    #[serde(default)]
    pub tool_diameter: Option<f64>,
    /// Sheet width (mm), more than 0 and at most 10000. Default: 600.
    #[serde(default)]
    pub sheet_width: Option<f64>,
    /// Sheet height (mm), more than 0 and at most 10000. Default: 400.
    #[serde(default)]
    pub sheet_height: Option<f64>,
    /// The least distance between the machine's paths round two parts on a sheet (mm), 0 to
    /// 10000; with `machine` cnc the parts end up `tool_diameter` further apart. Default: 3.
    #[serde(default)]
    pub gap: Option<f64>,
    /// The least distance from a sheet's edge (mm), 0 to 10000 and less than half the sheet's
    /// width and height. Default: 5.
    #[serde(default)]
    pub margin: Option<f64>,
    /// Engrave each part's name on it where the name fits (stepped down to 2 mm high if it
    /// must; a part with no room gets no name). Default: true.
    #[serde(default)]
    pub labels: Option<bool>,
    /// The capital height of the engraved names (mm), more than 0 and at most 10000. Default: 5.
    #[serde(default)]
    pub label_height: Option<f64>,
}

/// What `fab.slice` reads: the settings, and what to return besides the parts.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct FabSliceParams {
    /// The slicing and sheet settings.
    #[serde(flatten)]
    pub settings: FabSettingsParams,
    /// Also pack the parts onto sheets. Default: true.
    #[serde(default)]
    pub sheets: Option<bool>,
    /// Include each part's outline and holes, its 3D frame and places, and each
    /// sheet's cut and engraved lines, for a preview. Default: false.
    #[serde(default)]
    pub geometry: Option<bool>,
}

/// What `fab.export_sheets` reads: the settings, and where and how to write the sheets.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct FabExportParams {
    /// The slicing and sheet settings.
    #[serde(flatten)]
    pub settings: FabSettingsParams,
    /// Where to write, as a path without extension: one file per sheet and
    /// format, `<path>-sheet1.dxf`, `<path>-sheet1.svg`, ... Files of an earlier export to the
    /// same path and format numbered past the new last sheet are removed. Without it the files'
    /// texts are returned.
    #[serde(default)]
    pub path: Option<String>,
    /// The formats to write, `dxf` and/or `svg`. Default: both.
    #[serde(default)]
    #[schemars(length(min = 1, max = 2))]
    pub formats: Option<Vec<FabFormat>>,
}

/// A sheet file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FabFormat {
    /// DXF (AutoCAD 2000, mm): layers CUT (red) and ENGRAVE (blue), only the cut and engrave
    /// paths (no sheet outline).
    Dxf,
    /// SVG (mm): cuts stroked red, engraving blue.
    Svg,
}

/// Something about a slicing to know; the parts are still usable.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FabWarning {
    /// `no_material`, `falls_apart`, `dowels_short`, `rod_hub`, `gaps_not_whole` or `never_cross`.
    pub code: String,
    /// The warning in English.
    pub message: String,
    /// Its details (part, pieces, axis, at, placed, wanted, thickness), as the code has them.
    #[serde(flatten)]
    #[schemars(with = "serde_json::Map<String, Value>")]
    pub details: serde_json::Map<String, Value>,
}

/// One flat part.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FabPartInfo {
    /// Its name, engraved on it.
    pub name: String,
    /// `slice`, `plate` or `spacer`.
    pub kind: String,
    /// How many are cut.
    pub copies: u32,
    /// Its material's area (mm²).
    pub area_mm2: f64,
    /// Its width in its own frame (mm).
    pub width_mm: f64,
    /// Its height in its own frame (mm).
    pub height_mm: f64,
    /// With `geometry`: its outline (counter-clockwise) and holes (clockwise), `[[x, y], ...]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rings: Option<Vec<Vec<[f64; 2]>>>,
    /// With `geometry`: where each copy sits in the model, `{origin, u, v, n}`: the part's
    /// `(x, y)` at `origin + x u + y v`, the sheet centred on that plane along `n`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Vec<serde_json::Map<String, Value>>>")]
    pub places: Option<Vec<Value>>,
}

/// One packed sheet.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FabSheetInfo {
    /// The parts on it, in the order placed.
    pub parts: Vec<String>,
    /// How many rings are cut.
    pub cuts: usize,
    /// How many lines are engraved.
    pub engraved: usize,
    /// With `geometry`: the rings cut, in sheet mm from its lower-left corner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_rings: Option<Vec<Vec<[f64; 2]>>>,
    /// With `geometry`: the lines engraved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engrave_lines: Option<Vec<Vec<[f64; 2]>>>,
}

/// A part too big for the sheet: the details of `sheets_error` when the parts could not be packed
/// for that reason.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FabDoesNotFit {
    /// The part's name.
    pub part: String,
    /// The part's width in its own frame, as the machine cuts it (mm).
    pub width_mm: f64,
    /// The part's height in its own frame, as the machine cuts it (mm).
    pub height_mm: f64,
    /// The sheet's width (mm).
    pub sheet_width_mm: f64,
    /// The sheet's height (mm).
    pub sheet_height_mm: f64,
    /// The least distance kept from the sheet's edge (mm).
    pub margin_mm: f64,
}

/// What `fab.slice` returns.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FabSliceResult {
    /// The body sliced.
    pub body: String,
    /// The style.
    pub style: FabStyle,
    /// The sheet thickness (mm).
    pub thickness: f64,
    /// Parts cut, copies counted.
    pub count: u64,
    /// The parts.
    pub parts: Vec<FabPartInfo>,
    /// What to know about them.
    pub warnings: Vec<FabWarning>,
    /// The sheet size `[width, height]` (mm).
    pub sheet_size: [f64; 2],
    /// The sheets the parts are packed onto (empty without `sheets` or when they don't fit).
    pub sheets: Vec<FabSheetInfo>,
    /// Why the parts could not be packed (a part bigger than the sheet), when they could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheets_error: Option<String>,
    /// With `sheets_error`, when a part is bigger than the sheet allows: that part and the sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub does_not_fit: Option<FabDoesNotFit>,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// One written (or returned) sheet file.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FabFile {
    /// The sheet, from 1.
    pub sheet: usize,
    /// `dxf` or `svg`.
    pub format: FabFormat,
    /// The file written, with `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The file's text, without `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// What `fab.export_sheets` returns.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FabExportResult {
    /// The body sliced.
    pub body: String,
    /// Parts cut, copies counted.
    pub count: u64,
    /// How many sheets.
    pub sheets: usize,
    /// The files, sheet by sheet.
    pub files: Vec<FabFile>,
    /// Files of an earlier export to the same path, numbered past the last sheet, that were
    /// removed.
    pub removed: usize,
    /// What to know about the parts.
    pub warnings: Vec<FabWarning>,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "fab_tests.rs"]
mod tests;
