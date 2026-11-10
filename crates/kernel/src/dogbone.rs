//! Dogbone reliefs: the bite a router's round tool cuts into an inside corner so that a square
//! part seats in it, through `solidcore_ops::dogbone` (the recipe) and the kernel's
//! `solidcore::dogbone` (every point and length). This crate computes nothing of its own here:
//! it guards the calls and words the refusals.
//!
//! - [`plan`]: the relief cylinders of a body's picked inside edges, or of the inside corners of
//!   picked planar faces (a pocket's floor), and the edges skipped with the reason. The caller
//!   cuts the cylinders from the body.
//! - [`ring_reliefs`], [`relieved_ring`]: the reliefs of a planar loop's inside corners (an
//!   outline counter-clockwise, a hole clockwise), for flat parts cut from sheet.

pub use solidcore_ops::dogbone::{BodyOptions, Corner2, Options, Plan, Relief2, ReliefCylinder, Side, Skip, Skipped, Style, Tool, Why};

use crate::body::Body;
use crate::{KernelError, Result, guard};

/// The reliefs of `body` at its record edges `edges` and at the corners of its record faces
/// `faces` (see `solidcore_ops::dogbone::plan`).
///
/// # Errors
/// [`KernelError::Invalid`] for a mesh body, an index past the body's edges or faces, or
/// options out of range; [`KernelError::Internal`] for a panic in the kernel.
pub fn plan(body: &Body, edges: &[usize], faces: &[usize], options: &BodyOptions) -> Result<Plan> {
    body.require_brep("a dogbone")?;
    guard("dogbone", || solidcore_ops::dogbone::plan(body.record(), edges, faces, options).map_err(|e| KernelError::Invalid(format!("dogbone: {e}"))))
}

/// The relief (or why none) at each vertex of planar loop `ring`, the material on its left.
///
/// # Errors
/// [`KernelError::Invalid`] for options or points out of range.
pub fn ring_reliefs(ring: &[[f64; 2]], options: &Options) -> Result<Vec<Corner2>> {
    guard("dogbone ring", || solidcore_ops::dogbone::ring_reliefs(ring, options).map_err(|e| KernelError::Invalid(format!("dogbone: {e}"))))
}

/// `ring` with its reliefs cut in, each arc within `chord_tolerance` of its circle.
///
/// # Errors
/// [`KernelError::Invalid`] for options or points out of range, or a relief that runs past its
/// edge (cut those with a boolean of the circles instead).
pub fn relieved_ring(ring: &[[f64; 2]], options: &Options, chord_tolerance: f64) -> Result<Vec<[f64; 2]>> {
    guard("dogbone ring", || {
        solidcore_ops::dogbone::relieved_ring(ring, options, chord_tolerance).map_err(|e| KernelError::Invalid(format!("dogbone: {e}")))
    })
}
