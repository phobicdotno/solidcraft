//! Slice for Laser / CNC: a body cut into flat parts for sheet stock and those parts packed onto
//! sheets, through solidcore's recipe ([`solidcore_ops::fab`]). Nothing here computes geometry;
//! every call into solidcore runs under [`crate::guard`].

pub use solidcore_ops::fab::{
    Axis, DEFAULT_KERF, DEFAULT_SETTINGS, DEFAULT_SHEETS, DEFAULT_TOOL_DIAMETER, FabResult, FabSettings, FlatError, Frame, Machine, PartKind, Shape,
    Sheet, SheetSettings, Style, Warning,
};

use crate::{Body, KernelError, Result};

/// `body` cut into flat parts as `settings` say: the outer error when the kernel fails, the inner
/// one when the slicing refuses (callers report the [`FlatError`] itself). `body` is unchanged.
///
/// # Errors
/// [`KernelError::Invalid`] for a mesh body; [`KernelError::Failed`] for a body the kernel cannot
/// take as it stands (its record converted once, as every face query converts it) or a slicing
/// it fails; [`KernelError::Internal`] for a panic in the kernel.
pub fn slice_body(body: &Body, settings: &FabSettings) -> Result<std::result::Result<FabResult, FlatError>> {
    body.require_brep("Slice for Laser / CNC")?;
    crate::guard("slice for laser / CNC", || {
        let (ops_body, _) = body.ops_body().ok_or_else(|| KernelError::Failed("slicing: the kernel cannot take this body".into()))?;
        solidcore_ops::fab::slice_body(ops_body, settings).map_err(|e| KernelError::Failed(format!("slicing the body: {e}")))
    })
}

/// The parts of `result` finished for the machine of `sheets` and packed onto sheets: the outer
/// error when the kernel fails, the inner one when the packing refuses (a part that fits on no
/// sheet).
pub fn layout_sheets(result: &FabResult, sheets: &SheetSettings) -> Result<std::result::Result<Vec<Sheet>, FlatError>> {
    crate::guard("packing parts onto sheets", || Ok(solidcore_ops::fab::layout_sheets(result, sheets)))
}

/// The area of a part's material (mm²).
pub fn part_area(shape: &Shape) -> Result<f64> {
    crate::guard("the area of a flat part", || Ok(solidcore_ops::fab::part_area(shape)))
}

/// The width and height `[w, h]` of a part's outline in its own frame (mm), `None` without
/// points.
pub fn part_size(shape: &Shape) -> Result<Option<[f64; 2]>> {
    crate::guard("the size of a flat part", || Ok(solidcore_ops::fab::part_size(shape)))
}
