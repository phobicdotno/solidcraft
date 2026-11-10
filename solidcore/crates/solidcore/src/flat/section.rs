//! The section of a B-rep by a plane as a polygonal region: the kernel's exact planar section
//! curves, sampled within a chord tolerance, chained into closed loops and filled even-odd.

use super::slice::{Axis, Sectioner};
use super::{FlatError, FlatResult, Region};
use crate::kernel::Kernel;
use crate::topology::Brep;

/// A B-rep as a model to slice (stub until the section lands).
pub struct BrepSection<'a> {
    _k: &'a Kernel,
    _b: &'a mut Brep,
}

impl<'a> BrepSection<'a> {
    /// The body `b` to section within `chord` mm.
    ///
    /// # Errors
    /// Always, until the section lands.
    pub fn new(k: &'a Kernel, b: &'a mut Brep, chord: f64) -> FlatResult<Self> {
        let _ = chord;
        Ok(BrepSection { _k: k, _b: b })
    }
}

impl Sectioner for BrepSection<'_> {
    fn bounds(&self) -> [f64; 6] {
        [0.0; 6]
    }
    fn section(&mut self, _axis: Axis, _at: f64) -> FlatResult<Region> {
        Err(FlatError::Nothing)
    }
}
