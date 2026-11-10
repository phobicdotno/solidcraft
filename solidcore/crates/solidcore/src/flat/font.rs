//! Single-stroke lettering for engraving part names: each stroke is cut once, where outline text
//! would cut both sides of every letter. The glyphs are this project's own, drawn on a grid of
//! capital height 1.

use super::Ring;

/// The width of a glyph's cell, as a fraction of the capital height.
pub const WIDTH: f64 = 0.6;

/// The distance from one character to the next, as a fraction of the capital height.
pub const ADVANCE: f64 = 0.8;

/// The most characters drawn; the rest of a longer text is left out.
pub const MAX_CHARS: usize = 256;

/// A stroke on the design grid: x from 0 to 6, y from 0 (baseline) to 10 (capital height).
type Stroke = &'static [(f64, f64)];

const O_RING: Stroke = &[
    (1.5, 0.0),
    (0.0, 1.5),
    (0.0, 8.5),
    (1.5, 10.0),
    (4.5, 10.0),
    (6.0, 8.5),
    (6.0, 1.5),
    (4.5, 0.0),
    (1.5, 0.0),
];
const P_BOWL: Stroke = &[
    (0.0, 0.0),
    (0.0, 10.0),
    (4.5, 10.0),
    (6.0, 8.5),
    (6.0, 6.5),
    (4.5, 5.0),
    (0.0, 5.0),
];
const C_ARC: Stroke = &[
    (6.0, 8.5),
    (4.5, 10.0),
    (1.5, 10.0),
    (0.0, 8.5),
    (0.0, 1.5),
    (1.5, 0.0),
    (4.5, 0.0),
    (6.0, 1.5),
];
const DOT: Stroke = &[(2.5, 0.0), (3.5, 0.0), (3.5, 1.0), (2.5, 1.0), (2.5, 0.0)];

/// The strokes of a character's glyph (upper case), `None` for a character without one.
fn glyph(c: char) -> Option<&'static [Stroke]> {
    let g: &'static [Stroke] = match c {
        'A' => &[
            &[(0.0, 0.0), (3.0, 10.0), (6.0, 0.0)],
            &[(1.2, 4.0), (4.8, 4.0)],
        ],
        'B' => &[
            &[
                (0.0, 5.0),
                (0.0, 10.0),
                (4.0, 10.0),
                (5.5, 8.8),
                (5.5, 6.2),
                (4.0, 5.0),
            ],
            &[
                (0.0, 5.0),
                (0.0, 0.0),
                (4.5, 0.0),
                (6.0, 1.3),
                (6.0, 3.7),
                (4.5, 5.0),
                (0.0, 5.0),
            ],
        ],
        'C' => &[C_ARC],
        'D' => &[&[
            (0.0, 0.0),
            (0.0, 10.0),
            (3.5, 10.0),
            (6.0, 7.5),
            (6.0, 2.5),
            (3.5, 0.0),
            (0.0, 0.0),
        ]],
        'E' => &[
            &[(6.0, 10.0), (0.0, 10.0), (0.0, 0.0), (6.0, 0.0)],
            &[(0.0, 5.0), (4.5, 5.0)],
        ],
        'F' => &[
            &[(6.0, 10.0), (0.0, 10.0), (0.0, 0.0)],
            &[(0.0, 5.0), (4.5, 5.0)],
        ],
        'G' => &[C_ARC, &[(6.0, 1.5), (6.0, 4.5), (3.5, 4.5)]],
        'H' => &[
            &[(0.0, 0.0), (0.0, 10.0)],
            &[(6.0, 0.0), (6.0, 10.0)],
            &[(0.0, 5.0), (6.0, 5.0)],
        ],
        'I' => &[
            &[(3.0, 0.0), (3.0, 10.0)],
            &[(1.5, 10.0), (4.5, 10.0)],
            &[(1.5, 0.0), (4.5, 0.0)],
        ],
        'J' => &[&[
            (6.0, 10.0),
            (6.0, 1.5),
            (4.5, 0.0),
            (1.5, 0.0),
            (0.0, 1.5),
            (0.0, 3.0),
        ]],
        'K' => &[
            &[(0.0, 0.0), (0.0, 10.0)],
            &[(6.0, 10.0), (0.0, 4.0)],
            &[(2.0, 6.0), (6.0, 0.0)],
        ],
        'L' => &[&[(0.0, 10.0), (0.0, 0.0), (6.0, 0.0)]],
        'M' => &[&[(0.0, 0.0), (0.0, 10.0), (3.0, 5.0), (6.0, 10.0), (6.0, 0.0)]],
        'N' => &[&[(0.0, 0.0), (0.0, 10.0), (6.0, 0.0), (6.0, 10.0)]],
        'O' => &[O_RING],
        'P' => &[P_BOWL],
        'Q' => &[O_RING, &[(3.5, 2.5), (6.0, 0.0)]],
        'R' => &[P_BOWL, &[(3.0, 5.0), (6.0, 0.0)]],
        'S' => &[&[
            (6.0, 8.5),
            (4.5, 10.0),
            (1.5, 10.0),
            (0.0, 8.5),
            (0.0, 6.5),
            (1.5, 5.0),
            (4.5, 5.0),
            (6.0, 3.5),
            (6.0, 1.5),
            (4.5, 0.0),
            (1.5, 0.0),
            (0.0, 1.5),
        ]],
        'T' => &[&[(0.0, 10.0), (6.0, 10.0)], &[(3.0, 10.0), (3.0, 0.0)]],
        'U' => &[&[
            (0.0, 10.0),
            (0.0, 1.5),
            (1.5, 0.0),
            (4.5, 0.0),
            (6.0, 1.5),
            (6.0, 10.0),
        ]],
        'V' => &[&[(0.0, 10.0), (3.0, 0.0), (6.0, 10.0)]],
        'W' => &[&[(0.0, 10.0), (1.5, 0.0), (3.0, 6.0), (4.5, 0.0), (6.0, 10.0)]],
        'X' => &[&[(0.0, 0.0), (6.0, 10.0)], &[(0.0, 10.0), (6.0, 0.0)]],
        'Y' => &[
            &[(0.0, 10.0), (3.0, 5.0), (6.0, 10.0)],
            &[(3.0, 5.0), (3.0, 0.0)],
        ],
        'Z' => &[&[(0.0, 10.0), (6.0, 10.0), (0.0, 0.0), (6.0, 0.0)]],
        '0' => &[O_RING, &[(1.0, 2.0), (5.0, 8.0)]],
        '1' => &[
            &[(1.5, 8.0), (3.0, 10.0), (3.0, 0.0)],
            &[(1.5, 0.0), (4.5, 0.0)],
        ],
        '2' => &[&[
            (0.0, 8.5),
            (1.5, 10.0),
            (4.5, 10.0),
            (6.0, 8.5),
            (6.0, 6.5),
            (0.0, 0.0),
            (6.0, 0.0),
        ]],
        '3' => &[
            &[
                (0.0, 8.5),
                (1.5, 10.0),
                (4.5, 10.0),
                (6.0, 8.5),
                (6.0, 6.5),
                (4.5, 5.0),
                (2.0, 5.0),
            ],
            &[
                (4.5, 5.0),
                (6.0, 3.5),
                (6.0, 1.5),
                (4.5, 0.0),
                (1.5, 0.0),
                (0.0, 1.5),
            ],
        ],
        '4' => &[&[(4.5, 0.0), (4.5, 10.0), (0.0, 3.0), (6.0, 3.0)]],
        '5' => &[&[
            (6.0, 10.0),
            (0.0, 10.0),
            (0.0, 5.5),
            (4.5, 5.5),
            (6.0, 4.0),
            (6.0, 1.5),
            (4.5, 0.0),
            (1.5, 0.0),
            (0.0, 1.5),
        ]],
        '6' => &[&[
            (5.5, 10.0),
            (2.0, 10.0),
            (0.0, 7.5),
            (0.0, 1.5),
            (1.5, 0.0),
            (4.5, 0.0),
            (6.0, 1.5),
            (6.0, 4.0),
            (4.5, 5.5),
            (0.0, 5.5),
        ]],
        '7' => &[&[(0.0, 10.0), (6.0, 10.0), (2.0, 0.0)]],
        '8' => &[&[
            (1.5, 5.0),
            (0.0, 6.5),
            (0.0, 8.5),
            (1.5, 10.0),
            (4.5, 10.0),
            (6.0, 8.5),
            (6.0, 6.5),
            (4.5, 5.0),
            (1.5, 5.0),
            (0.0, 3.5),
            (0.0, 1.5),
            (1.5, 0.0),
            (4.5, 0.0),
            (6.0, 1.5),
            (6.0, 3.5),
            (4.5, 5.0),
        ]],
        '9' => &[&[
            (6.0, 4.5),
            (1.5, 4.5),
            (0.0, 6.0),
            (0.0, 8.5),
            (1.5, 10.0),
            (4.5, 10.0),
            (6.0, 8.5),
            (6.0, 2.5),
            (4.0, 0.0),
            (0.5, 0.0),
        ]],
        ' ' => &[],
        '-' => &[&[(1.0, 5.0), (5.0, 5.0)]],
        '.' => &[DOT],
        ',' => &[&[(3.5, 2.0), (3.5, 1.0), (2.5, 0.0)]],
        ':' => &[
            DOT,
            &[(2.5, 6.0), (3.5, 6.0), (3.5, 7.0), (2.5, 7.0), (2.5, 6.0)],
        ],
        '/' => &[&[(0.0, 0.0), (6.0, 10.0)]],
        '(' => &[&[(4.0, 10.0), (2.5, 8.0), (2.0, 5.0), (2.5, 2.0), (4.0, 0.0)]],
        ')' => &[&[(2.0, 10.0), (3.5, 8.0), (4.0, 5.0), (3.5, 2.0), (2.0, 0.0)]],
        '#' => &[
            &[(2.0, 0.0), (2.0, 10.0)],
            &[(4.0, 0.0), (4.0, 10.0)],
            &[(0.0, 3.5), (6.0, 3.5)],
            &[(0.0, 6.5), (6.0, 6.5)],
        ],
        '+' => &[&[(3.0, 2.0), (3.0, 8.0)], &[(0.0, 5.0), (6.0, 5.0)]],
        '_' => &[&[(0.0, 0.0), (6.0, 0.0)]],
        '\'' => &[&[(3.0, 10.0), (3.0, 7.0)]],
        _ => return None,
    };
    Some(g)
}

/// `text` as open polylines (mm): capitals `height` tall standing on `y = 0`, starting at
/// `x = 0`, one [`ADVANCE`] of the height per character. Lower-case letters are drawn as
/// capitals; a character without a glyph is a gap. Only the first [`MAX_CHARS`] characters are
/// drawn; a height that is not finite and positive draws nothing.
#[must_use]
pub fn strokes(text: &str, height: f64) -> Vec<Ring> {
    let span = MAX_CHARS as f64 * ADVANCE * height;
    if !(height.is_finite() && height > 0.0 && span.is_finite()) {
        return Vec::new();
    }
    // the design grid is 10 units to the capital height
    let unit = height / 10.0;
    let mut out = Vec::new();
    for (i, c) in text.chars().take(MAX_CHARS).enumerate() {
        let Some(g) = glyph(c.to_ascii_uppercase()) else {
            continue;
        };
        let x0 = i as f64 * ADVANCE * height;
        out.extend(
            g.iter()
                .map(|s| s.iter().map(|&(x, y)| [x0 + x * unit, y * unit]).collect()),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUPPORTED: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 -.,:/()#+_'";

    fn bounds(lines: &[Ring]) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in lines.iter().flatten() {
            b = [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ];
        }
        b
    }

    #[test]
    fn every_supported_glyph_stays_in_its_cell() {
        for c in SUPPORTED.chars() {
            let g = glyph(c).unwrap();
            if c != ' ' {
                assert!(!g.is_empty(), "{c} has no strokes");
            }
            for s in g {
                assert!(s.len() >= 2, "{c} has a stroke of one point");
            }
            let lines = strokes(&c.to_string(), 1.0);
            for p in lines.iter().flatten() {
                assert!((0.0..=WIDTH + 1e-12).contains(&p[0]), "{c}: x {}", p[0]);
                assert!((0.0..=1.0 + 1e-12).contains(&p[1]), "{c}: y {}", p[1]);
            }
        }
    }

    #[test]
    fn letters_and_digits_reach_baseline_and_capital_height() {
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".chars() {
            let b = bounds(&strokes(&c.to_string(), 1.0));
            assert_eq!(b[1], 0.0, "{c}");
            assert!((b[3] - 1.0).abs() < 1e-12, "{c}");
        }
    }

    #[test]
    fn h_has_exact_bounds() {
        let b = bounds(&strokes("H", 7.0));
        assert_eq!(b[0], 0.0);
        assert_eq!(b[1], 0.0);
        assert!((b[2] - 0.6 * 7.0).abs() < 1e-12);
        assert_eq!(b[3], 7.0);
        assert_eq!(strokes("H", 7.0).len(), 3);
    }

    #[test]
    fn scaling_is_linear() {
        let a = strokes("PART 12", 1.0);
        let b = strokes("PART 12", 3.5);
        assert_eq!(a.len(), b.len());
        for (la, lb) in a.iter().zip(&b) {
            assert_eq!(la.len(), lb.len());
            for (p, q) in la.iter().zip(lb) {
                assert!((p[0] * 3.5 - q[0]).abs() < 1e-12 && (p[1] * 3.5 - q[1]).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn each_character_moves_one_advance() {
        let one = bounds(&strokes("H", 2.0));
        let three = bounds(&strokes("HHH", 2.0));
        assert!((three[2] - (one[2] + 2.0 * ADVANCE * 2.0)).abs() < 1e-12);
        let mut last = 0.0;
        for n in 1..6 {
            let x = bounds(&strokes(&"E".repeat(n), 1.0))[2];
            assert!(x > last);
            last = x;
        }
    }

    #[test]
    fn lower_case_draws_capitals() {
        assert_eq!(strokes("abc xyz", 2.0), strokes("ABC XYZ", 2.0));
    }

    #[test]
    fn unknown_characters_are_gaps() {
        assert_eq!(strokes("A\u{e9}B", 1.0), strokes("A B", 1.0));
        assert!(strokes("\u{e9}\u{263a}~", 1.0).is_empty());
        assert!(strokes("", 1.0).is_empty());
        assert!(strokes("   ", 1.0).is_empty());
    }

    #[test]
    fn hostile_heights_draw_nothing() {
        for h in [
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            1e306,
        ] {
            assert!(strokes("AB", h).is_empty(), "{h}");
        }
        let tiny = strokes("A", 1e-300);
        assert!(tiny
            .iter()
            .flatten()
            .all(|p| p[0].is_finite() && p[1].is_finite()));
    }

    #[test]
    fn long_text_is_capped() {
        let long = "I".repeat(10 * MAX_CHARS);
        let lines = strokes(&long, 1.0);
        assert!(bounds(&lines)[2] < MAX_CHARS as f64 * ADVANCE);
        assert_eq!(lines.len(), 3 * MAX_CHARS);
    }
}
