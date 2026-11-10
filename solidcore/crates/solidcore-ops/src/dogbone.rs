//! Dogbone reliefs: the recipe that finds the inside corners of a body (or of a planar loop) a
//! router's round tool cannot cut sharp, and the relief each gets, from the kernel's
//! [`solidcore::dogbone`] primitive (which computes every point, direction and length).
//!
//! For a body, [`plan`] reads its record: each straight edge picked (or each straight edge along
//! a picked planar face's normal that meets the face, such as the walls of a pocket on its
//! floor) between two planar faces; the kernel says whether it is an inside corner and, if so,
//! the cylinder that relieves it. A relief runs the edge's length, past an end only where the
//! face there faces on along the edge (open: the top of a pocket or both ends of a slot through
//! the plate), so it never cuts below a blind pocket's floor. The application cuts the
//! cylinders from the body; [`plan`] says which edges it skipped and why.
//!
//! For a planar loop, [`ring_reliefs`] and [`relieved_ring`] pass the kernel's answer on.

use solidcore::dogbone as k;
use solidcore_record::brep::{Brep, Curve, Loop, Shell, Surface};

use crate::error::status;
use crate::{Error, Result};

pub use solidcore::dogbone::{
    Corner2, EdgeCorner, Options, Relief2, ReliefCylinder, Side, Skip, Style, ANGLE_SLACK,
};

/// How far (radians) an edge may lean from the tool axis and still run along it.
pub const AXIS_TOLERANCE: f64 = 1.0e-6;

/// Most edges and faces one plan takes.
pub const MAX_PICKS: usize = 10_000;

/// The reliefs (or why none) at each vertex of planar loop `ring`, the material on its left
/// ([`k::ring_reliefs`]).
///
/// # Errors
/// [`Error::Status`] (input refused) for options or points out of range.
pub fn ring_reliefs(ring: &[[f64; 2]], options: &Options) -> Result<Vec<Corner2>> {
    k::ring_reliefs(ring, options).map_err(status)
}

/// `ring` with its reliefs cut in, arcs to within `chord_tolerance` ([`k::relieved_ring`]).
///
/// # Errors
/// [`Error::Status`] (input refused) for options or points out of range, or a relief that runs
/// past its edge.
pub fn relieved_ring(
    ring: &[[f64; 2]],
    options: &Options,
    chord_tolerance: f64,
) -> Result<Vec<[f64; 2]>> {
    k::relieved_ring(ring, options, chord_tolerance).map_err(status)
}

/// What a body's reliefs are cut with, and which edges get one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyOptions {
    /// The tool and the style.
    pub relief: Options,
    /// The tool's axis: a picked edge not along it is skipped. `None`: any direction. A picked
    /// face's corners are always along its normal.
    pub along: Option<[f64; 3]>,
}

/// Why an edge or a face gets no relief.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Why {
    /// The kernel's reason for the corner ([`Skip`]): convex, too open, ….
    Corner(Skip),
    /// The faces on either side continue each other: no corner.
    Smooth,
    /// The edge is not a straight line.
    NotStraight,
    /// The edge is not between exactly two faces of the body (a sheet's edge, a seam).
    NotTwoFaces,
    /// A face at the edge (or the picked face) is not planar.
    NotPlanar,
    /// The edge does not run along the tool's axis.
    NotAlong,
    /// The picked face has no straight edge along its normal at its corners.
    NoCorners,
}

/// A relief to cut: the edge it relieves and the cylinder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tool {
    /// The record edge relieved.
    pub edge: usize,
    /// The middle of that edge.
    pub at: [f64; 3],
    /// The cylinder to cut.
    pub cylinder: ReliefCylinder,
}

/// An edge or a picked face that got no relief.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skipped {
    /// The record edge, for an edge.
    pub edge: Option<usize>,
    /// The record face, for a picked face with no corners.
    pub face: Option<usize>,
    /// A point on it: an edge's middle, a face's first vertex.
    pub at: [f64; 3],
    /// Why.
    pub why: Why,
}

/// The reliefs of a body: what to cut, and what was skipped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// The cylinders to cut, one per relieved edge, in the order the edges were taken.
    pub tools: Vec<Tool>,
    /// The edges and faces skipped, and why.
    pub skipped: Vec<Skipped>,
}

/// The reliefs of record `body` at record edges `edges` and at the corners of record faces
/// `faces`, each edge once.
///
/// # Errors
/// [`Error::Argument`] for an index past the body's edges or faces, more than [`MAX_PICKS`]
/// of either, or options the kernel refuses; the kernel's refusal of a point that is not finite.
pub fn plan(body: &Brep, edges: &[usize], faces: &[usize], options: &BodyOptions) -> Result<Plan> {
    let relief = options.relief.checked().map_err(|_| Error::Argument)?;
    if edges.len() > MAX_PICKS || faces.len() > MAX_PICKS {
        return Err(Error::Argument);
    }
    if edges.iter().any(|e| *e >= body.edges.len()) || faces.iter().any(|f| *f >= body.faces.len())
    {
        return Err(Error::Argument);
    }
    let mut out = Plan::default();
    let mut seen: Vec<usize> = Vec::new();
    for e in edges {
        if seen.contains(e) {
            continue;
        }
        seen.push(*e);
        relieve(body, *e, options.along, &relief, &mut out)?;
    }
    for f in faces {
        let Some(axis) = face_outward(body, *f)? else {
            out.skipped.push(Skipped {
                edge: None,
                face: Some(*f),
                at: face_point(body, *f),
                why: Why::NotPlanar,
            });
            continue;
        };
        let corners = face_corners(body, *f, axis)?;
        if corners.is_empty() {
            out.skipped.push(Skipped {
                edge: None,
                face: Some(*f),
                at: face_point(body, *f),
                why: Why::NoCorners,
            });
        }
        for e in corners {
            if seen.contains(&e) {
                continue;
            }
            seen.push(e);
            relieve(body, e, Some(axis), &relief, &mut out)?;
        }
    }
    Ok(out)
}

/// The relief of edge `e`, or why none, added to `out`.
fn relieve(
    body: &Brep,
    e: usize,
    along: Option<[f64; 3]>,
    options: &Options,
    out: &mut Plan,
) -> Result<()> {
    let Some((start, end)) = ends(body, e) else {
        return Err(Error::Argument);
    };
    let skip = |why: Why, out: &mut Plan| {
        out.skipped.push(Skipped {
            edge: Some(e),
            face: None,
            at: k::midpoint(start, end),
            why,
        });
    };
    let Some(edge) = body.edges.get(e) else {
        return Err(Error::Argument);
    };
    if !matches!(edge.curve, Curve::Line { .. }) || edge.nurbs.is_some() {
        skip(Why::NotStraight, out);
        return Ok(());
    }
    if let Some(axis) = along {
        if !k::is_along(start, end, axis, AXIS_TOLERANCE).map_err(status)? {
            skip(Why::NotAlong, out);
            return Ok(());
        }
    }
    let [first, second] = match edge.uses.as_slice() {
        [a, b] if a.face != b.face => [a.face as usize, b.face as usize],
        _ => {
            skip(Why::NotTwoFaces, out);
            return Ok(());
        }
    };
    let (Some(fa), Some(fb)) = (edge_face(body, first, e)?, edge_face(body, second, e)?) else {
        skip(Why::NotPlanar, out);
        return Ok(());
    };
    let (walls, _) = match k::edge_corner(start, end, [fa, fb]).map_err(status)? {
        EdgeCorner::Concave { walls, angle } => (walls, angle),
        EdgeCorner::Convex => {
            skip(Why::Corner(Skip::Convex), out);
            return Ok(());
        }
        EdgeCorner::Smooth => {
            skip(Why::Smooth, out);
            return Ok(());
        }
    };
    let lengths = [wall_length(body, first, e), wall_length(body, second, e)];
    let [v0, v1] = edge.vertices;
    let extend = [
        extension(
            body,
            v0 as usize,
            [first, second],
            end,
            start,
            options.radius,
        )?,
        extension(
            body,
            v1 as usize,
            [first, second],
            start,
            end,
            options.radius,
        )?,
    ];
    match k::edge_relief(start, end, walls, lengths, extend, options).map_err(status)? {
        Ok(cylinder) => out.tools.push(Tool {
            edge: e,
            at: k::midpoint(start, end),
            cylinder,
        }),
        Err(why) => skip(Why::Corner(why), out),
    }
    Ok(())
}

/// The points of edge `e`'s start and end vertices.
fn ends(body: &Brep, e: usize) -> Option<([f64; 3], [f64; 3])> {
    let edge = body.edges.get(e)?;
    let [a, b] = edge.vertices;
    Some((
        body.vertices.get(a as usize)?.point,
        body.vertices.get(b as usize)?.point,
    ))
}

/// Face `f` as [`k::edge_corner`] reads it along edge `e`: `None` when it is not planar or not
/// used once by one shell.
fn edge_face(body: &Brep, f: usize, e: usize) -> Result<Option<k::EdgeFace>> {
    let Some(outward) = face_outward(body, f)? else {
        return Ok(None);
    };
    let (Some(shell_forward), Some(loop_forward)) = (shell_sense(body, f), loop_sense(body, f, e))
    else {
        return Ok(None);
    };
    // Loops run counter-clockwise about the natural normal, the face on their left; the outward
    // normal is the natural one when the shell uses the face forward.
    Ok(Some(k::EdgeFace {
        outward,
        left: loop_forward == shell_forward,
    }))
}

/// Face `f`'s outward unit normal, if it is planar and one shell uses it once.
fn face_outward(body: &Brep, f: usize) -> Result<Option<[f64; 3]>> {
    let Some(face) = body.faces.get(f) else {
        return Err(Error::Argument);
    };
    if face.nurbs.is_some() {
        return Ok(None);
    }
    let natural = match &face.surface {
        Surface::Plane { u_axis, v_axis, .. } => k::plane_normal(*u_axis, *v_axis),
        Surface::PlaneThrough {
            origin,
            u_point,
            v_point,
        } => k::plane_normal_through(*origin, *u_point, *v_point),
        _ => return Ok(None),
    };
    let Ok(natural) = natural else {
        return Ok(None);
    };
    Ok(shell_sense(body, f).map(|forward| k::flipped(natural, !forward)))
}

/// Whether the one shell using face `f` uses it along its natural normal; `None` when no shell,
/// or more than one use, has it.
fn shell_sense(body: &Brep, f: usize) -> Option<bool> {
    let mut found = None;
    for shell in &body.shells {
        if let Shell::Faces { faces, .. } = shell {
            for of in faces.iter().filter(|of| of.face as usize == f) {
                if found.is_some() {
                    return None;
                }
                found = Some(of.forward);
            }
        }
    }
    found
}

/// Whether face `f`'s loop runs along edge `e` from its start to its end; `None` when the face
/// doesn't use it, or uses it twice (a seam).
fn loop_sense(body: &Brep, f: usize, e: usize) -> Option<bool> {
    let face = body.faces.get(f)?;
    let mut found = None;
    for lp in &face.loops {
        if let Loop::Edges { edges } = lp {
            for oe in edges.iter().filter(|oe| oe.edge as usize == e) {
                if found.is_some() {
                    return None;
                }
                found = Some(oe.forward);
            }
        }
    }
    found
}

/// How long the wall face `f` is beside edge `e`: the longest straight edge of its loop that
/// meets `e` at a vertex (the wall's edges along the floor and the top); 0 when none is straight.
fn wall_length(body: &Brep, f: usize, e: usize) -> f64 {
    let Some(edge) = body.edges.get(e) else {
        return 0.0;
    };
    let Some(face) = body.faces.get(f) else {
        return 0.0;
    };
    let mut longest = 0.0;
    for lp in &face.loops {
        let Loop::Edges { edges } = lp else { continue };
        for oe in edges.iter().filter(|oe| oe.edge as usize != e) {
            let Some(other) = body.edges.get(oe.edge as usize) else {
                continue;
            };
            let meets = other.vertices.iter().any(|v| edge.vertices.contains(v));
            if !meets || !matches!(other.curve, Curve::Line { .. }) {
                continue;
            }
            if let Some((a, b)) = ends(body, oe.edge as usize) {
                let l = k::distance(a, b);
                if l > longest {
                    longest = l;
                }
            }
        }
    }
    longest
}

/// How far a relief runs past vertex `v` of an edge (its point `here`, the other end's `there`): `radius` when every face at `v` other than the walls is planar and faces on along
/// the edge (open), else 0 (a blind end, such as a pocket's floor).
fn extension(
    body: &Brep,
    v: usize,
    walls: [usize; 2],
    there: [f64; 3],
    here: [f64; 3],
    radius: f64,
) -> Result<f64> {
    let mut any = false;
    for f in faces_at_vertex(body, v) {
        if walls.contains(&f) {
            continue;
        }
        let Some(outward) = face_outward(body, f)? else {
            return Ok(0.0);
        };
        if !k::end_is_open(there, here, outward).map_err(status)? {
            return Ok(0.0);
        }
        any = true;
    }
    Ok(if any { radius } else { 0.0 })
}

/// The faces with an edge ending at vertex `v`, each once.
fn faces_at_vertex(body: &Brep, v: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, face) in body.faces.iter().enumerate() {
        let touches = face.loops.iter().any(|lp| match lp {
            Loop::Edges { edges } => edges.iter().any(|oe| {
                body.edges
                    .get(oe.edge as usize)
                    .is_some_and(|ed| ed.vertices.iter().any(|x| *x as usize == v))
            }),
            Loop::Vertex { vertex } => *vertex as usize == v,
        });
        if touches {
            out.push(i);
        }
    }
    out
}

/// The straight edges along `axis` with a vertex on face `f`, not on it: the corners of a
/// pocket's walls on its floor, or of a slot's through a plate's face.
fn face_corners(body: &Brep, f: usize, axis: [f64; 3]) -> Result<Vec<usize>> {
    let Some(face) = body.faces.get(f) else {
        return Err(Error::Argument);
    };
    let mut own: Vec<u32> = Vec::new();
    let mut vertices: Vec<u32> = Vec::new();
    for lp in &face.loops {
        match lp {
            Loop::Edges { edges } => {
                for oe in edges {
                    own.push(oe.edge);
                    if let Some(ed) = body.edges.get(oe.edge as usize) {
                        vertices.extend(ed.vertices);
                    }
                }
            }
            Loop::Vertex { vertex } => vertices.push(*vertex),
        }
    }
    let mut out = Vec::new();
    for (i, ed) in body.edges.iter().enumerate() {
        if own.iter().any(|o| *o as usize == i) || !ed.vertices.iter().any(|v| vertices.contains(v))
        {
            continue;
        }
        if !matches!(ed.curve, Curve::Line { .. }) || ed.nurbs.is_some() {
            continue;
        }
        let Some((a, b)) = ends(body, i) else {
            continue;
        };
        if k::is_along(a, b, axis, AXIS_TOLERANCE).map_err(status)? {
            out.push(i);
        }
    }
    Ok(out)
}

/// A point of face `f` to name it by: its first vertex's.
fn face_point(body: &Brep, f: usize) -> [f64; 3] {
    let first = body.faces.get(f).and_then(|face| {
        face.loops.iter().find_map(|lp| match lp {
            Loop::Edges { edges } => edges
                .first()
                .and_then(|oe| body.edges.get(oe.edge as usize))
                .map(|ed| {
                    let [first, _] = ed.vertices;
                    first
                }),
            Loop::Vertex { vertex } => Some(*vertex),
        })
    });
    first
        .and_then(|v| body.vertices.get(v as usize))
        .map_or([0.0; 3], |v| v.point)
}

#[cfg(test)]
#[path = "dogbone_tests.rs"]
mod tests;
