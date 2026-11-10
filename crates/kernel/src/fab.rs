//! Slice for Laser / CNC: a body cut into flat parts for sheet stock and those parts packed onto
//! sheets, through solidcore's recipe ([`solidcore_ops::fab`]). Nothing here computes geometry.

pub use solidcore_ops::fab::{
    Axis, FabPart, FabResult, FabSettings, Finished, FlatError, Frame, Machine, Mark, P2, PartKind, Ring, Shape, Sheet, SheetSettings,
    Style, Warning,
};

use crate::{Body, KernelError, Result};

/// What a slicing refuses (bad settings, nothing to cut, a part that fits on no sheet) as the
/// kernel's error, with the words the refusal gives.
fn refused(e: &FlatError) -> KernelError {
    match e {
        FlatError::Kernel(k) => KernelError::Failed(format!("slicing the body: {k}")),
        other => KernelError::Invalid(other.to_string()),
    }
}

/// `body` cut into flat parts as `settings` say: the outer error when the kernel fails, the inner
/// one when the slicing refuses (callers report the [`FlatError`] itself). `body` is unchanged.
pub fn slice_body(body: &Body, settings: &FabSettings) -> Result<std::result::Result<FabResult, FlatError>> {
    crate::guard("slice for laser / CNC", || {
        let record = crate::native::brep_record(body)?;
        let (ops_body, _) = solidcore_ops::body::Body::from_record(&record, true).map_err(|e| KernelError::Failed(format!("slicing the body: {e}")))?;
        solidcore_ops::fab::slice_body(&ops_body, settings).map_err(|e| KernelError::Failed(format!("slicing the body: {e}")))
    })
}

/// The parts of `result` finished for the machine of `sheets` and packed onto sheets.
pub fn layout_sheets(result: &FabResult, sheets: &SheetSettings) -> std::result::Result<Vec<Sheet>, FlatError> {
    solidcore_ops::fab::layout_sheets(result, sheets)
}

/// One part finished for the machine of `sheets`, in its own frame.
pub fn finish_part(part: &FabPart, sheets: &SheetSettings) -> Finished {
    solidcore_ops::fab::finish_part(part, sheets)
}

/// The area of a part's material (mm²).
pub fn part_area(shape: &Shape) -> f64 {
    solidcore_ops::fab::part_area(shape)
}

/// The bounds `[xmin, ymin, xmax, ymax]` of a part's outline in its own frame.
pub fn part_bounds(shape: &Shape) -> Option<[f64; 4]> {
    solidcore_ops::fab::part_bounds(shape)
}

/// The refusal as the kernel's error, for callers that need one.
pub fn refusal(e: &FlatError) -> KernelError {
    refused(e)
}
