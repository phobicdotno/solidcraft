//! Flat plates for display: a part cut from sheet stock (its outline and holes as polygons in
//! its own plane) drawn as a slab `thickness` thick, centred on that plane, and the fill of
//! polygons on a flat page (a sheet's cut rings). The faces are the constrained Delaunay
//! triangulation the face mesher uses for planar faces; the walls are one quad per side. A display
//! mesh only: nothing is decided from it.

use solidcore_record::{Vec2, Vec3};

use crate::mesh::Mesh;
use crate::mesher::cdt::{Fill, triangulate};
use crate::vec::VecExt as _;

/// Where a plate sits: its point `(x, y)` is at `origin + x u + y v`, and the slab reaches
/// `thickness / 2` to each side along `n`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateFrame {
    /// The plane's point for `(0, 0)`.
    pub origin: Vec3,
    /// The plane's x direction.
    pub u: Vec3,
    /// The plane's y direction.
    pub v: Vec3,
    /// The direction the slab is thick along.
    pub n: Vec3,
}

/// Most corners a plate or a fill takes, all rings together; past it nothing is drawn.
pub const MAX_PLATE_CORNERS: usize = 20_000;

/// Corners closer than this (mm) are one.
const SAME: f64 = 1e-9;

/// The plate a part makes: `rings` are its outline then its holes (`[x, y]` corners, either
/// way round), `frame` places it and `thickness` is the sheet's. Triangles face outwards, each
/// with its own corners and flat normal; the outline and holes are drawn as edges on both faces.
/// Empty for a ring with fewer than three corners, a non-finite value, a thickness not above 0
/// or more than [`MAX_PLATE_CORNERS`] corners.
#[must_use]
pub fn plate(rings: &[Vec<[f64; 2]>], frame: &PlateFrame, thickness: f64) -> Mesh {
    let mut mesh = Mesh::default();
    let Some(n) = frame.n.normalized() else {
        return mesh;
    };
    let sound = thickness.is_finite()
        && thickness > 0.0
        && frame.origin.is_finite()
        && frame.u.is_finite()
        && frame.v.is_finite();
    if !sound {
        return mesh;
    }
    let Some(loops) = polygons(rings) else {
        return mesh;
    };
    let half = n * (thickness * 0.5);
    let at = |p: [f64; 2]| frame.origin + frame.u * p[0] + frame.v * p[1];
    for [a, b, c] in triangles(&loops) {
        let (a, b, c) = (at(a), at(b), at(c));
        push_facing(&mut mesh, [a + half, b + half, c + half], n);
        push_facing(&mut mesh, [a - half, b - half, c - half], -n);
    }
    for pts in &loops {
        for (p, q) in pts.iter().zip(pts.iter().cycle().skip(1)) {
            // The material is on the left, so outwards is to the right of p -> q.
            let out = frame.u * (q[1] - p[1]) - frame.v * (q[0] - p[0]);
            let (a, b) = (at(*p), at(*q));
            push_facing(&mut mesh, [a - half, b - half, b + half], out);
            push_facing(&mut mesh, [a - half, b + half, a + half], out);
        }
        let ring: Vec<Vec3> = pts.iter().chain(pts.first()).map(|p| at(*p)).collect();
        mesh.edges.push(ring.iter().map(|p| *p + half).collect());
        mesh.edges.push(ring.iter().map(|p| *p - half).collect());
    }
    mesh.display_only_edges = true;
    mesh.tri_face = vec![0; mesh.triangles.len()];
    mesh
}

/// Triangles filling closed polygons `[x, y]` by the even-odd rule: a part's outline and holes,
/// or every ring cut on a sheet (a part inside another's hole fills again). For drawing; empty
/// when the rings are not polygons (as for [`plate`]).
#[must_use]
pub fn fill(rings: &[Vec<[f64; 2]>]) -> Vec<[Vec2; 3]> {
    polygons(rings)
        .map(|loops| {
            triangles(&loops)
                .into_iter()
                .map(|t| t.map(Vec2::from))
                .collect()
        })
        .unwrap_or_default()
}

/// The rings as polygons, the first counter-clockwise and the rest clockwise (so the material of
/// a part is on the left of every side), repeated corners dropped. `None` for no ring, a ring
/// with fewer than three corners, a non-finite corner or more than [`MAX_PLATE_CORNERS`] in all.
fn polygons(rings: &[Vec<[f64; 2]>]) -> Option<Vec<Vec<[f64; 2]>>> {
    if rings.is_empty() || rings.iter().map(Vec::len).sum::<usize>() > MAX_PLATE_CORNERS {
        return None;
    }
    let apart = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) > SAME;
    let mut loops = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for &p in ring {
            if !(p[0].is_finite() && p[1].is_finite()) {
                return None;
            }
            if pts.last().is_none_or(|q| apart(p, *q)) {
                pts.push(p);
            }
        }
        if pts.len() > 1
            && pts
                .first()
                .zip(pts.last())
                .is_some_and(|(a, b)| !apart(*a, *b))
        {
            pts.pop();
        }
        if pts.len() < 3 {
            return None;
        }
        let area2: f64 = pts
            .iter()
            .zip(pts.iter().cycle().skip(1))
            .map(|(p, q)| p[0] * q[1] - q[0] * p[1])
            .sum();
        if (i == 0) != (area2 > 0.0) {
            pts.reverse();
        }
        loops.push(pts);
    }
    Some(loops)
}

/// The triangles inside the loops by the even-odd rule, as corner points. A side crossing one
/// already kept is left out rather than drawing nothing.
fn triangles(loops: &[Vec<[f64; 2]>]) -> Vec<[[f64; 2]; 3]> {
    let boundary: Vec<[f64; 2]> = loops.iter().flatten().copied().collect();
    let mut ranges = Vec::with_capacity(loops.len());
    let mut start = 0;
    for l in loops {
        ranges.push(start..start + l.len());
        start += l.len();
    }
    let fill = Fill {
        seeds: &[],
        refine: None,
        shape: false,
        lenient: true,
        flat: None,
    };
    let Some((extra, tris)) = triangulate(&boundary, &ranges, fill) else {
        return Vec::new();
    };
    let point = |i: u32| {
        let i = usize::try_from(i).ok()?;
        boundary
            .get(i)
            .or_else(|| extra.get(i.checked_sub(boundary.len())?))
            .copied()
    };
    tris.iter()
        .filter_map(|[a, b, c]| Some([point(*a)?, point(*b)?, point(*c)?]))
        .collect()
}

/// One triangle with its own corners, wound so its normal points along `out`.
fn push_facing(mesh: &mut Mesh, [a, b, c]: [Vec3; 3], out: Vec3) {
    let normal = (b - a).cross(c - a);
    let Some(unit) = normal.normalized() else {
        return;
    };
    let (b, c, unit) = if normal.dot(out) < 0.0 {
        (c, b, -unit)
    } else {
        (b, c, unit)
    };
    let Ok(first) = u32::try_from(mesh.positions.len()) else {
        return;
    };
    mesh.positions.extend([a, b, c]);
    mesh.normals.extend([unit; 3]);
    mesh.triangles.push([first, first + 1, first + 2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, side: f64) -> Vec<[f64; 2]> {
        vec![
            [x0, y0],
            [x0 + side, y0],
            [x0 + side, y0 + side],
            [x0, y0 + side],
        ]
    }

    fn frame() -> PlateFrame {
        PlateFrame {
            origin: Vec3::new(0.0, 0.0, 0.0),
            u: Vec3::new(1.0, 0.0, 0.0),
            v: Vec3::new(0.0, 1.0, 0.0),
            n: Vec3::new(0.0, 0.0, 1.0),
        }
    }

    fn corners(mesh: &Mesh) -> Vec<[Vec3; 3]> {
        mesh.triangles
            .iter()
            .map(|t| t.map(|i| mesh.positions[i as usize]))
            .collect()
    }

    /// The enclosed volume, signed: a triangle facing inwards takes its share off.
    fn volume(mesh: &Mesh) -> f64 {
        corners(mesh)
            .iter()
            .map(|[a, b, c]| a.dot(b.cross(*c)) / 6.0)
            .sum()
    }

    fn area(mesh: &Mesh) -> f64 {
        corners(mesh)
            .iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).len() / 2.0)
            .sum()
    }

    #[test]
    fn a_plate_holds_its_area_times_the_thickness() {
        // 40 x 30 less a 10 mm square hole, 3 mm thick: (1200 - 100) * 3.
        let rings = vec![
            vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
            square(10.0, 10.0, 10.0),
        ];
        let mesh = plate(&rings, &frame(), 3.0);
        assert!((volume(&mesh) - 3300.0).abs() < 1e-6, "{}", volume(&mesh));
        // Faces 2 * 1100, outer walls 140 * 3, hole walls 40 * 3.
        assert!(
            (area(&mesh) - (2200.0 + 420.0 + 120.0)).abs() < 1e-6,
            "{}",
            area(&mesh)
        );
        assert_eq!(mesh.edges.len(), 4);
        assert_eq!(mesh.tri_face.len(), mesh.triangles.len());
        assert!(mesh.display_only_edges);
    }

    #[test]
    fn a_plate_is_centred_on_its_plane_whatever_the_frame_and_ring_direction() {
        // A frame turned about X, moved off the origin, its normal against u x v, and the
        // outline given clockwise and the hole counter-clockwise.
        let f = PlateFrame {
            origin: Vec3::new(5.0, -7.0, 12.0),
            u: Vec3::new(1.0, 0.0, 0.0),
            v: Vec3::new(0.0, 0.0, 1.0),
            n: Vec3::new(0.0, 1.0, 0.0),
        };
        let mut outer = square(0.0, 0.0, 20.0);
        outer.reverse();
        let rings = vec![outer, square(5.0, 5.0, 4.0)];
        let mesh = plate(&rings, &f, 6.0);
        assert!(
            (volume(&mesh) - (400.0 - 16.0) * 6.0).abs() < 1e-6,
            "{}",
            volume(&mesh)
        );
        let lo = mesh
            .positions
            .iter()
            .fold(Vec3::new(f64::MAX, f64::MAX, f64::MAX), |m, p| m.min(*p));
        let hi = mesh
            .positions
            .iter()
            .fold(Vec3::new(f64::MIN, f64::MIN, f64::MIN), |m, p| m.max(*p));
        assert!(
            (lo.y + 10.0).abs() < 1e-9 && (hi.y + 4.0).abs() < 1e-9,
            "{lo:?} {hi:?}"
        );
        assert!(
            (lo.x - 5.0).abs() < 1e-9 && (hi.z - 32.0).abs() < 1e-9,
            "{lo:?} {hi:?}"
        );
    }

    #[test]
    fn a_sheet_fills_by_the_even_odd_rule() {
        // A 40 mm square with a 20 mm hole, and a 10 mm part packed inside that hole.
        let rings = vec![
            square(0.0, 0.0, 40.0),
            square(10.0, 10.0, 20.0),
            square(15.0, 15.0, 10.0),
        ];
        let area: f64 = fill(&rings)
            .iter()
            .map(|[a, b, c]| ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() / 2.0)
            .sum();
        assert!((area - (1600.0 - 400.0 + 100.0)).abs() < 1e-6, "{area}");
        assert!(fill(&[vec![[0.0, 0.0], [1.0, 1.0]]]).is_empty());
    }

    #[test]
    fn hostile_rings_make_no_plate() {
        let f = frame();
        assert!(plate(&[], &f, 3.0).triangles.is_empty());
        assert!(
            plate(&[vec![[0.0, 0.0], [1.0, 0.0]]], &f, 3.0)
                .triangles
                .is_empty()
        );
        assert!(
            plate(&[square(0.0, 0.0, 1.0)], &f, 0.0)
                .triangles
                .is_empty()
        );
        assert!(
            plate(&[square(0.0, 0.0, 1.0)], &f, f64::NAN)
                .triangles
                .is_empty()
        );
        assert!(
            plate(
                &[vec![[0.0, 0.0], [f64::INFINITY, 0.0], [1.0, 1.0]]],
                &f,
                1.0
            )
            .triangles
            .is_empty()
        );
        let flat = PlateFrame {
            n: Vec3::new(0.0, 0.0, 0.0),
            ..f
        };
        assert!(
            plate(&[square(0.0, 0.0, 1.0)], &flat, 1.0)
                .triangles
                .is_empty()
        );
        let many = vec![square(0.0, 0.0, 1.0); MAX_PLATE_CORNERS / 4 + 1];
        assert!(fill(&many).is_empty());
    }
}
