//! A sheet's paper scene (`geom::paper`) in a file: SVG first (A3, #4890); DXF, PNG and PDF follow.

mod svg;

pub use svg::{SvgReport, paper_svg};
/// The SVG number writer, shared with the sheets of `fab_out`.
pub(crate) use svg::{nano, number, push_number};
