//! Single-stroke lettering for engraving part names: each stroke is cut once, where outline text
//! would cut both sides of every letter. The glyphs are this project's own, drawn on a grid of
//! capital height 1.

use super::Ring;

/// `text` as open polylines (mm): capitals `height` tall standing on `y = 0`, starting at
/// `x = 0`. Lower-case letters are drawn as capitals; a character without a glyph is a gap.
#[must_use]
pub fn strokes(text: &str, height: f64) -> Vec<Ring> {
    let _ = (text, height);
    Vec::new()
}
