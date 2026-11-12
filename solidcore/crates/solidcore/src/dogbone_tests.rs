use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI, SQRT_2};

use super::*;

const R: f64 = 1.5;

fn opts(style: Style) -> Options {
    Options {
        radius: R,
        style,
        max_angle: PI,
    }
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn close3(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| close(*x, *y, 1e-12))
}

/// The unit wall directions of a corner of angle `theta`: `a` along +x, `b` turned `theta` from it.
fn walls(theta: f64) -> ([f64; 3], [f64; 3]) {
    ([1.0, 0.0, 0.0], [theta.cos(), theta.sin(), 0.0])
}

fn wedge(theta: f64, style: Style) -> Wedge {
    let (a, b) = walls(theta);
    wedge_relief(a, b, 10.0, 5.0, &opts(style))
        .unwrap()
        .unwrap()
}

/// The area of the disc about `c` (radius `r`) outside the wedge of angle `theta` at the origin
/// between +x and the direction at `theta`, by a fine midpoint grid: an independent check of
/// [`removed_area`].
fn grid_area(c: [f64; 3], r: f64, theta: f64) -> f64 {
    let n = 1000u32;
    let h = 2.0 * r / f64::from(n);
    let mut inside_disc_outside_wedge = 0usize;
    for i in 0..n {
        for j in 0..n {
            let x = c[0] - r + (f64::from(i) + 0.5) * h;
            let y = c[1] - r + (f64::from(j) + 0.5) * h;
            if (x - c[0]).powi(2) + (y - c[1]).powi(2) > r * r {
                continue;
            }
            let ang = y.atan2(x);
            let in_wedge = ang >= 0.0 && ang <= theta;
            if !in_wedge {
                inside_disc_outside_wedge += 1;
            }
        }
    }
    inside_disc_outside_wedge as f64 * h * h
}

#[test]
fn a_normal_relief_of_a_square_corner_sits_on_the_bisector_through_the_corner() {
    let w = wedge(FRAC_PI_2, Style::Normal);
    assert!(close(w.angle, FRAC_PI_2, 1e-15));
    assert!(close3(
        w.centre,
        [R * FRAC_1_SQRT_2, R * FRAC_1_SQRT_2, 0.0]
    ));
    // The circle crosses each wall at the corner and √2 r from it.
    for bite in [w.bite_a, w.bite_b] {
        let [near, far] = bite.unwrap();
        assert_eq!(near, 0.0);
        assert!(close(far, SQRT_2 * R, 1e-12), "{far}");
    }
    // (π/2 − 1) r² beyond the walls.
    assert!(close(
        removed_area(FRAC_PI_2, &opts(Style::Normal)).unwrap(),
        (FRAC_PI_2 - 1.0) * R * R,
        1e-14
    ));
}

#[test]
fn normal_reliefs_at_60_and_120_degrees_follow_the_bisector() {
    for theta in [PI / 3.0, 2.0 * PI / 3.0] {
        let w = wedge(theta, Style::Normal);
        let half = theta / 2.0;
        assert!(
            close3(w.centre, [R * half.cos(), R * half.sin(), 0.0]),
            "{theta}: {:?}",
            w.centre
        );
        // A chord through the corner at angle θ/2 from the line to the centre: 2 r cos(θ/2).
        for bite in [w.bite_a, w.bite_b] {
            let [near, far] = bite.unwrap();
            assert_eq!(near, 0.0);
            assert!(close(far, 2.0 * R * half.cos(), 1e-12));
        }
    }
    // r² (π − θ − sin θ): 60° → (2π/3 − √3/2) r², 120° → (π/3 − √3/2) r².
    let s3 = 3f64.sqrt() / 2.0;
    assert!(close(
        removed_area(PI / 3.0, &opts(Style::Normal)).unwrap(),
        (2.0 * PI / 3.0 - s3) * R * R,
        1e-13
    ));
    assert!(close(
        removed_area(2.0 * PI / 3.0, &opts(Style::Normal)).unwrap(),
        (PI / 3.0 - s3) * R * R,
        1e-13
    ));
}

#[test]
fn the_closed_form_areas_match_a_grid_count() {
    for theta in [PI / 3.0, FRAC_PI_2, 2.0 * PI / 3.0, 0.8 * PI] {
        for style in [
            Style::Normal,
            Style::Minimal { gap: 0.2 },
            Style::TBone { side: Side::Longer },
        ] {
            let Some(exact) = removed_area(theta, &opts(style)) else {
                assert!(matches!(style, Style::TBone { .. }) && theta < FRAC_PI_2);
                continue;
            };
            if exact == 0.0 {
                // A minimal circle that reaches no wall at this angle: no relief.
                let (a, b) = walls(theta);
                assert_eq!(
                    wedge_relief(a, b, 10.0, 5.0, &opts(style)).unwrap(),
                    Err(Skip::Unreached)
                );
                continue;
            }
            let w = wedge(theta, style);
            let counted = grid_area(w.centre, R, theta);
            // The grid's error is about the disc's perimeter times a cell.
            assert!(
                close(counted, exact, 2.0 * PI * R * (2.0 * R / 1000.0)),
                "{theta} {style:?}: {counted} vs {exact}"
            );
        }
    }
}

#[test]
fn the_normal_relief_removes_the_least_of_any_circle_through_the_corner() {
    // r² (π − θ − (sin 2φ + sin(2θ − 2φ))/2) for a centre at angle φ from wall a: least at θ/2.
    let theta = FRAC_PI_2;
    let normal = removed_area(theta, &opts(Style::Normal)).unwrap();
    for phi in [0.1, 0.3, 0.6, 1.0, 1.3] {
        let c = [R * f64::cos(phi), R * f64::sin(phi), 0.0];
        assert!(grid_area(c, R, theta) > normal, "{phi}");
    }
}

#[test]
fn a_minimal_relief_stops_its_gap_short_of_the_corner() {
    let gap = 0.15;
    let w = wedge(FRAC_PI_2, Style::Minimal { gap });
    let s = R + gap;
    assert!(close3(
        w.centre,
        [s * FRAC_1_SQRT_2, s * FRAC_1_SQRT_2, 0.0]
    ));
    // On each wall: x² − 2 x (s/√2) + s² − r² = 0.
    let h = s * FRAC_1_SQRT_2;
    let root = (h * h - (s * s - R * R)).sqrt();
    for bite in [w.bite_a, w.bite_b] {
        let [near, far] = bite.unwrap();
        assert!(
            close(near, h - root, 1e-12) && close(far, h + root, 1e-12),
            "{near} {far}"
        );
        assert!(near > 0.0);
    }
    // The nearest point of the circle to the corner is `gap` from it.
    assert!(close(len(w.centre) - R, gap, 1e-12));
    // Gap 0 is the normal relief.
    assert_eq!(
        wedge(FRAC_PI_2, Style::Minimal { gap: 0.0 }).centre,
        wedge(FRAC_PI_2, Style::Normal).centre
    );
    // A gap so large the circle reaches no wall: no relief.
    let (a, b) = walls(FRAC_PI_2);
    assert_eq!(
        wedge_relief(a, b, 1.0, 1.0, &opts(Style::Minimal { gap: R })).unwrap(),
        Err(Skip::Unreached)
    );
}

#[test]
fn a_t_bone_bites_the_wall_it_is_told_to() {
    // Wall a is 10 long, b 5: Longer takes a, Shorter b.
    let long = wedge(FRAC_PI_2, Style::TBone { side: Side::Longer });
    assert!(close3(long.centre, [R, 0.0, 0.0]));
    assert_eq!(long.bite_a, Some([0.0, 2.0 * R]));
    // Tangent to the other wall at 90°: no bite there.
    assert_eq!(long.bite_b, None);
    let short = wedge(
        FRAC_PI_2,
        Style::TBone {
            side: Side::Shorter,
        },
    );
    assert!(close3(short.centre, [0.0, R, 0.0]));
    assert_eq!(short.bite_a, None);
    let [near, far] = short.bite_b.unwrap();
    assert!(near == 0.0 && close(far, 2.0 * R, 1e-12));
    // Equal walls: Longer takes the first, Shorter the second.
    let (a, b) = walls(FRAC_PI_2);
    let eq = |side| {
        wedge_relief(a, b, 3.0, 3.0, &opts(Style::TBone { side }))
            .unwrap()
            .unwrap()
            .centre
    };
    assert!(close3(eq(Side::Longer), [R, 0.0, 0.0]));
    assert!(close3(eq(Side::Shorter), [0.0, R, 0.0]));
    // At 60° the circle also cuts the other wall, 2 r cos θ along it.
    let sharp = wedge(PI / 3.0, Style::TBone { side: Side::Longer });
    let [_, far] = sharp.bite_b.unwrap();
    assert!(close(far, 2.0 * R * (PI / 3.0).cos(), 1e-12));
    assert_eq!(
        removed_area(FRAC_PI_2, &opts(Style::TBone { side: Side::Longer })),
        Some(PI * R * R / 2.0)
    );
}

#[test]
fn corners_more_open_than_the_largest_angle_are_skipped() {
    let (a, b) = walls(FRAC_PI_2 + 0.01);
    let square_only = Options {
        radius: R,
        style: Style::Normal,
        max_angle: FRAC_PI_2,
    };
    assert!(
        matches!(wedge_relief(a, b, 1.0, 1.0, &square_only).unwrap(), Err(Skip::TooOpen { angle }) if close(angle, FRAC_PI_2 + 0.01, 1e-12))
    );
    // A square corner computed with rounding is still relieved.
    let (a, b) = walls(FRAC_PI_2 + 1e-9);
    assert!(wedge_relief(a, b, 1.0, 1.0, &square_only).unwrap().is_ok());
    // Walls in line or folded: no corner.
    assert_eq!(
        wedge_relief(
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            1.0,
            1.0,
            &opts(Style::Normal)
        )
        .unwrap(),
        Err(Skip::Straight)
    );
    assert_eq!(
        wedge_relief(
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            1.0,
            1.0,
            &opts(Style::Normal)
        )
        .unwrap(),
        Err(Skip::Degenerate)
    );
    assert_eq!(
        wedge_relief([0.0; 3], [1.0, 0.0, 0.0], 1.0, 1.0, &opts(Style::Normal)).unwrap(),
        Err(Skip::Degenerate)
    );
}

#[test]
fn hostile_options_and_points_are_refused() {
    let (a, b) = walls(FRAC_PI_2);
    for radius in [0.0, -1.0, f64::NAN, f64::INFINITY, 2e6] {
        let o = Options {
            radius,
            style: Style::Normal,
            max_angle: PI,
        };
        assert!(wedge_relief(a, b, 1.0, 1.0, &o).is_err(), "{radius}");
        assert!(ring_reliefs(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &o).is_err());
    }
    for gap in [-0.1, f64::NAN, f64::INFINITY] {
        assert!(opts(Style::Minimal { gap }).checked().is_err());
    }
    for max_angle in [0.0, -1.0, 4.0, f64::NAN] {
        assert!(Options {
            radius: 1.0,
            style: Style::Normal,
            max_angle
        }
        .checked()
        .is_err());
    }
    assert!(wedge_relief([f64::NAN, 0.0, 0.0], b, 1.0, 1.0, &opts(Style::Normal)).is_err());
    assert!(wedge_relief(a, b, f64::NAN, 1.0, &opts(Style::Normal)).is_err());
    assert!(ring_reliefs(&[[0.0, 0.0], [1.0, 0.0]], &opts(Style::Normal)).is_err());
    assert!(ring_reliefs(
        &[[0.0, 0.0], [1.0, f64::NAN], [0.0, 1.0]],
        &opts(Style::Normal)
    )
    .is_err());
    assert!(ring_reliefs(&[[0.0, 0.0], [1.0, 2e6], [0.0, 1.0]], &opts(Style::Normal)).is_err());
    assert!(relieved_ring(
        &[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
        &opts(Style::Normal),
        0.0
    )
    .is_err());
    assert!(relieved_ring(
        &[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
        &opts(Style::Normal),
        f64::NAN
    )
    .is_err());
    assert!(edge_corner(
        [0.0; 3],
        [0.0; 3],
        [EdgeFace {
            outward: [1.0, 0.0, 0.0],
            left: true
        }; 2]
    )
    .is_err());
    assert!(edge_corner(
        [0.0; 3],
        [0.0, 0.0, 1.0],
        [EdgeFace {
            outward: [0.0; 3],
            left: true
        }; 2]
    )
    .is_err());
    assert!(edge_corner(
        [0.0; 3],
        [0.0, 0.0, f64::NAN],
        [EdgeFace {
            outward: [1.0, 0.0, 0.0],
            left: true
        }; 2]
    )
    .is_err());
    let w = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    assert!(edge_relief(
        [0.0; 3],
        [0.0, 0.0, 1.0],
        w,
        [1.0; 2],
        [-1.0, 0.0],
        &opts(Style::Normal)
    )
    .is_err());
    assert!(edge_relief(
        [0.0; 3],
        [0.0; 3],
        w,
        [1.0; 2],
        [0.0; 2],
        &opts(Style::Normal)
    )
    .is_err());
    assert!(end_is_open([0.0; 3], [0.0; 3], [0.0, 0.0, 1.0]).is_err());
    assert!(end_is_open([0.0; 3], [0.0, 0.0, 1.0], [0.0; 3]).is_err());
    assert!(is_along([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], -0.1).is_err());
    assert!(plane_normal([1.0, 0.0, 0.0], [2.0, 0.0, 0.0]).is_err());
}

/// The shoelace area of a ring (positive counter-clockwise).
fn area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>()
        / 2.0
}

#[test]
fn a_rectangular_hole_gets_four_reliefs_and_an_outline_none() {
    // A 20 x 12 hole runs clockwise: every corner turns right, an inside corner.
    let hole = [[0.0, 0.0], [0.0, 12.0], [20.0, 12.0], [20.0, 0.0]];
    let o = Options {
        radius: R,
        style: Style::Normal,
        max_angle: FRAC_PI_2,
    };
    let corners = ring_reliefs(&hole, &o).unwrap();
    assert_eq!(corners.len(), 4);
    let mut centres = Vec::new();
    for c in &corners {
        let Corner2::Relief(r) = c else {
            panic!("{c:?}")
        };
        assert!(close(r.angle, FRAC_PI_2, 1e-12));
        assert_eq!(r.radius, R);
        centres.push(r.centre);
    }
    let d = R * FRAC_1_SQRT_2;
    let want = [[d, d], [d, 12.0 - d], [20.0 - d, 12.0 - d], [20.0 - d, d]];
    for (c, w) in centres.iter().zip(&want) {
        assert!(
            close(c[0], w[0], 1e-12) && close(c[1], w[1], 1e-12),
            "{c:?} {w:?}"
        );
    }
    // The same rectangle as an outline (counter-clockwise): four convex corners.
    let outline: Vec<[f64; 2]> = hole.iter().rev().copied().collect();
    assert!(ring_reliefs(&outline, &o)
        .unwrap()
        .iter()
        .all(|c| *c == Corner2::Skipped(Skip::Convex)));
    // The relieved hole is larger by four reliefs' areas (to the chords' error).
    let tol = 1e-4;
    let relieved = relieved_ring(&hole, &o, tol).unwrap();
    let grown = -area(&relieved) - (-area(&hole));
    let want = 4.0 * removed_area(FRAC_PI_2, &o).unwrap();
    assert!(
        close(grown, want, 4.0 * 2.0 * PI * R * tol),
        "{grown} vs {want}"
    );
    // It still runs clockwise and passes through no corner point.
    assert!(area(&relieved) < 0.0);
    assert!(!relieved.iter().any(|p| hole.contains(p)));
    // Every arc point is on its circle.
    for p in &relieved {
        let on_some = centres.iter().any(|c| {
            close(
                ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt(),
                R,
                1e-9,
            )
        });
        let on_edge = p[0].abs() < 1e-12
            || p[1].abs() < 1e-12
            || close(p[0], 20.0, 1e-12)
            || close(p[1], 12.0, 1e-12);
        assert!(on_some || on_edge, "{p:?}");
    }
}

#[test]
fn an_l_shaped_outline_has_one_inside_corner() {
    // Counter-clockwise L: five convex corners and the inside one at (10, 10).
    let l = [
        [0.0, 0.0],
        [30.0, 0.0],
        [30.0, 10.0],
        [10.0, 10.0],
        [10.0, 30.0],
        [0.0, 30.0],
    ];
    let o = opts(Style::Normal);
    let corners = ring_reliefs(&l, &o).unwrap();
    let inside: Vec<usize> = corners
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c, Corner2::Relief(_)))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(inside, vec![3]);
    assert_eq!(
        corners
            .iter()
            .filter(|c| **c == Corner2::Skipped(Skip::Convex))
            .count(),
        5
    );
    let Corner2::Relief(r) = corners[3] else {
        unreachable!()
    };
    // The void at (10, 10) is the quadrant x > 10, y > 10.
    let d = R * FRAC_1_SQRT_2;
    assert!(close(r.centre[0], 10.0 + d, 1e-12) && close(r.centre[1], 10.0 + d, 1e-12));
    // The material shrinks by one relief.
    let tol = 1e-5;
    let relieved = relieved_ring(&l, &o, tol).unwrap();
    let lost = area(&l) - area(&relieved);
    assert!(
        close(
            lost,
            removed_area(FRAC_PI_2, &o).unwrap(),
            2.0 * PI * R * tol
        ),
        "{lost}"
    );
}

#[test]
fn relieved_rings_splice_every_style() {
    let hole = [[0.0, 0.0], [0.0, 12.0], [20.0, 12.0], [20.0, 0.0]];
    for style in [
        Style::Minimal { gap: 0.2 },
        Style::TBone { side: Side::Longer },
        Style::TBone {
            side: Side::Shorter,
        },
    ] {
        let o = opts(style);
        let tol = 1e-5;
        let relieved = relieved_ring(&hole, &o, tol).unwrap();
        let grown = -area(&relieved) + area(&hole);
        let want = 4.0 * removed_area(FRAC_PI_2, &o).unwrap();
        assert!(
            close(grown, want, 4.0 * 2.0 * PI * R * tol),
            "{style:?}: {grown} vs {want}"
        );
        // The minimal style keeps the corner points between its bites; a square T-bone's arc ends
        // at the corner, tangent to the wall it doesn't bite.
        assert!(hole.iter().all(|p| relieved.contains(p)), "{style:?}");
    }
    // A T-bone on the longer (20 mm) walls bites them, not the 12 mm ones.
    let o = opts(Style::TBone { side: Side::Longer });
    for c in ring_reliefs(&hole, &o).unwrap() {
        let Corner2::Relief(r) = c else { panic!() };
        assert!(close(r.centre[1], r.corner[1], 1e-12), "{r:?}");
    }
}

#[test]
fn reliefs_too_big_for_their_edge_are_refused_by_the_splice() {
    // A 2 mm slot: bites of √2 r ≈ 2.1 mm from each end overlap.
    let slot = [[0.0, 0.0], [0.0, 2.0], [20.0, 2.0], [20.0, 0.0]];
    assert!(relieved_ring(&slot, &opts(Style::Normal), 1e-3).is_err());
    // The reliefs themselves are still reported.
    assert_eq!(
        ring_reliefs(&slot, &opts(Style::Normal))
            .unwrap()
            .iter()
            .filter(|c| matches!(c, Corner2::Relief(_)))
            .count(),
        4
    );
}

#[test]
fn the_splice_adds_the_bites_from_both_ends_of_an_edge() {
    // Each bite takes √2 r ≈ 2.12 of an edge: two fit on a 5 mm edge, not on a 4 mm one, though
    // either alone would. The short edge is the ring's last (it closes the ring) or a middle one.
    let o = opts(Style::Normal);
    for width in [4.0, 5.0] {
        let closing = [[0.0, width], [20.0, width], [20.0, 0.0], [0.0, 0.0]];
        let middle = [[0.0, 0.0], [0.0, width], [20.0, width], [20.0, 0.0]];
        for hole in [closing, middle] {
            assert_eq!(
                relieved_ring(&hole, &o, 1e-3).is_ok(),
                width == 5.0,
                "{hole:?}"
            );
        }
    }
}

#[test]
fn degenerate_ring_corners_are_skipped() {
    let o = opts(Style::Normal);
    // A repeated point, and a point in line with its neighbours.
    assert_eq!(
        corner_2d([0.0, 0.0], [0.0, 0.0], [1.0, 0.0], &o).unwrap(),
        Corner2::Skipped(Skip::Degenerate)
    );
    assert_eq!(
        corner_2d([0.0, 0.0], [1.0, 0.0], [2.0, 0.0], &o).unwrap(),
        Corner2::Skipped(Skip::Straight)
    );
    // Folded back on itself.
    assert_eq!(
        corner_2d([0.0, 0.0], [1.0, 0.0], [0.0, 0.0], &o).unwrap(),
        Corner2::Skipped(Skip::Degenerate)
    );
    assert_eq!(
        corner_2d([0.0, 0.0], [1.0, 0.0], [0.5, 0.0], &o).unwrap(),
        Corner2::Skipped(Skip::Degenerate)
    );
}

#[test]
fn turns_are_measured_on_unit_directions() {
    let o = opts(Style::Normal);
    // Along a diagonal: straight on, and folded back.
    assert_eq!(
        corner_2d([0.0, 0.0], [1.0, 1.0], [2.0, 2.0], &o).unwrap(),
        Corner2::Skipped(Skip::Straight)
    );
    assert_eq!(
        corner_2d([0.0, 0.0], [1.0, 1.0], [0.5, 0.5], &o).unwrap(),
        Corner2::Skipped(Skip::Degenerate)
    );
    assert_eq!(
        corner_2d([0.0, 0.0], [1.0, 1.0], [1.0, 2.0], &o).unwrap(),
        Corner2::Skipped(Skip::Convex)
    );
    assert!(matches!(
        corner_2d([0.0, 0.0], [1.0, 1.0], [2.0, 1.0], &o).unwrap(),
        Corner2::Relief(_)
    ));
    // Long edges turning by less than the straightness threshold (radians, not the cross
    // product of the edges, which is a million times larger here) are in line.
    let tiny = 2e-10f64;
    let long = [1000.0 * tiny.cos(), 1000.0 * tiny.sin()];
    assert_eq!(
        corner_2d([-1000.0, 0.0], [0.0, 0.0], long, &o).unwrap(),
        Corner2::Skipped(Skip::Straight)
    );
    // Short edges turning by more than it are corners.
    let small = [1e-3 * (1e-6f64).cos(), 1e-3 * (1e-6f64).sin()];
    assert_eq!(
        corner_2d([-1e-3, 0.0], [0.0, 0.0], small, &o).unwrap(),
        Corner2::Skipped(Skip::Convex)
    );
    // An inside corner of uneven edges is relieved whatever their lengths.
    let Corner2::Relief(r) = corner_2d([1000.0, 0.0], [0.0, 0.0], [0.0, 0.01], &o).unwrap() else {
        panic!()
    };
    assert!((r.angle - FRAC_PI_2).abs() < 1e-12);
}

#[test]
fn a_pocket_wall_edge_is_concave_and_a_box_edge_convex() {
    // A pocket's corner on the z axis, running up (t = +z): wall y = 0 (normal +y into the
    // pocket) runs along +x, wall x = 0 (normal +x) along +y. A face is on the edge's left when
    // n × t points into it.
    let (start, end) = ([0.0, 0.0, 0.0], [0.0, 0.0, 6.0]);
    // n × t for the wall y = 0: (0,1,0) × (0,0,1) = (1,0,0): into that face, so it is on the left.
    let wy = EdgeFace {
        outward: [0.0, 1.0, 0.0],
        left: true,
    };
    // (1,0,0) × (0,0,1) = (0,-1,0): away from the wall x = 0, which lies along +y: on the right.
    let wx = EdgeFace {
        outward: [1.0, 0.0, 0.0],
        left: false,
    };
    match edge_corner(start, end, [wy, wx]).unwrap() {
        EdgeCorner::Concave { walls, angle } => {
            assert!(close3(walls[0], [1.0, 0.0, 0.0]) && close3(walls[1], [0.0, 1.0, 0.0]));
            assert!(close(angle, FRAC_PI_2, 1e-15));
        }
        c => panic!("{c:?}"),
    }
    // A box's edge on the z axis, the box in x < 0, y < 0: face y = 0 (normal +y) lies along -x
    // (on the right of +z seen from +y), face x = 0 (normal +x) along -y (on the left).
    let by = EdgeFace {
        outward: [0.0, 1.0, 0.0],
        left: false,
    };
    let bx = EdgeFace {
        outward: [1.0, 0.0, 0.0],
        left: true,
    };
    assert_eq!(
        edge_corner(start, end, [by, bx]).unwrap(),
        EdgeCorner::Convex
    );
    // Two faces in one plane: smooth.
    let flat_a = EdgeFace {
        outward: [0.0, 1.0, 0.0],
        left: true,
    };
    let flat_b = EdgeFace {
        outward: [0.0, 1.0, 0.0],
        left: false,
    };
    assert_eq!(
        edge_corner(start, end, [flat_a, flat_b]).unwrap(),
        EdgeCorner::Smooth
    );
}

#[test]
fn a_relief_cylinder_runs_the_edge_and_its_extensions() {
    let walls = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let c = edge_relief(
        [2.0, 3.0, 4.0],
        [2.0, 3.0, 10.0],
        walls,
        [5.0, 5.0],
        [0.0, 1.5],
        &opts(Style::Normal),
    )
    .unwrap()
    .unwrap();
    let d = R * FRAC_1_SQRT_2;
    assert!(close3(c.base, [2.0 + d, 3.0 + d, 4.0]));
    assert!(close3(c.axis, [0.0, 0.0, 1.0]));
    assert_eq!((c.radius, c.height), (R, 7.5));
    assert!(close(c.angle, FRAC_PI_2, 1e-15));
    // Extended below the start too, and running the other way.
    let c = edge_relief(
        [0.0, 0.0, 6.0],
        [0.0, 0.0, 0.0],
        walls,
        [5.0, 5.0],
        [2.0, 0.0],
        &opts(Style::Normal),
    )
    .unwrap()
    .unwrap();
    assert!(close3(c.base, [d, d, 8.0]));
    assert!(close3(c.axis, [0.0, 0.0, -1.0]));
    assert_eq!(c.height, 8.0);
    // Skipped like a wedge.
    let open = Options {
        radius: R,
        style: Style::Normal,
        max_angle: 1.0,
    };
    assert!(matches!(
        edge_relief([0.0; 3], [0.0, 0.0, 1.0], walls, [1.0; 2], [0.0; 2], &open).unwrap(),
        Err(Skip::TooOpen { .. })
    ));
}

#[test]
fn ends_axes_and_planes() {
    // A floor facing up at the bottom of an edge running up: blind; the top face: open.
    let (top, floor) = ([0.0, 0.0, 10.0], [0.0, 0.0, 4.0]);
    assert!(!end_is_open(top, floor, [0.0, 0.0, 1.0]).unwrap());
    assert!(end_is_open(floor, top, [0.0, 0.0, 1.0]).unwrap());
    assert!(end_is_open(top, [0.0, 0.0, 1.0], [0.0, 0.3, -1.0]).unwrap());
    assert!(!end_is_open(floor, top, [1.0, 0.0, 0.0]).unwrap());
    assert!(is_along([0.0; 3], [0.0, 0.0, -2.0], [0.0, 0.0, 5.0], 1e-9).unwrap());
    assert!(!is_along([0.0; 3], [0.0, 0.01, 1.0], [0.0, 0.0, 1.0], 1e-3).unwrap());
    assert!(is_along([0.0; 3], [0.0, 0.0001, 1.0], [0.0, 0.0, 1.0], 1e-3).unwrap());
    assert!(close3(
        plane_normal([2.0, 0.0, 0.0], [0.0, 3.0, 0.0]).unwrap(),
        [0.0, 0.0, 1.0]
    ));
    assert!(close3(
        plane_normal_through([1.0, 1.0, 1.0], [1.0, 2.0, 1.0], [2.0, 1.0, 1.0]).unwrap(),
        [0.0, 0.0, -1.0]
    ));
    assert_eq!(distance([0.0; 3], [3.0, 4.0, 0.0]), 5.0);
    assert_eq!(midpoint([0.0; 3], [2.0, 4.0, 6.0]), [1.0, 2.0, 3.0]);
    assert_eq!(flipped([1.0, -2.0, 3.0], true), [-1.0, 2.0, -3.0]);
    assert_eq!(flipped([1.0, -2.0, 3.0], false), [1.0, -2.0, 3.0]);
}
