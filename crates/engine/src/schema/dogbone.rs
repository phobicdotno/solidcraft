//! Contract and result type for `solid.dogbone` (`cmd/dogbone.rs`).
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::features::FeatureResult;
use super::types::{AngleExpr, AxisDir, EdgeRefs, FaceRefs, FeatureCommon, LengthExpr, feature_common, present};

/// Where a relief sits in its corner.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DogboneStyleParam {
    /// On the corner's bisector, the circle through the sharp corner.
    #[default]
    Normal,
    /// On the bisector, stopping `gap` short of the sharp corner.
    Minimal,
    /// Along one wall, the circle through the corner: the bite goes into that wall only.
    TBone,
}

/// Which wall a T-bone relief bites.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DogboneSideParam {
    /// The longer wall at each corner.
    #[default]
    Longer,
    /// The shorter wall at each corner.
    Shorter,
}

/// Cut dogbone reliefs into inside corners. Give `edges`, `faces`, or both.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct DogboneParams {
    /// The inside corner edges to relieve: a point on each, or `{body, index}`.
    #[serde(default, deserialize_with = "present")]
    pub edges: Option<EdgeRefs>,
    /// Planar faces (a pocket's floor, a plate's face a slot runs through) whose inside corners,
    /// the straight edges along the face's normal at its corners, are relieved.
    #[serde(default, deserialize_with = "present")]
    pub faces: Option<FaceRefs>,
    /// The router bit's diameter: each relief is a round hole of this diameter.
    pub diameter: LengthExpr,
    /// Default: normal.
    #[serde(default)]
    pub style: Option<DogboneStyleParam>,
    /// Minimal style only: how far short of the sharp corner the relief stops. Default: 5% of
    /// the diameter.
    #[serde(default)]
    pub gap: Option<LengthExpr>,
    /// T-bone style only: which wall each relief bites. Default: longer.
    #[serde(default)]
    pub side: Option<DogboneSideParam>,
    /// The largest corner angle relieved: corners more open are skipped. Default: 90 deg (square
    /// and sharper corners).
    #[serde(default)]
    pub max_angle: Option<AngleExpr>,
    /// The tool's axis: picked edges not along it are skipped. Default: any direction (a picked
    /// face's corners are always along its normal).
    #[serde(default)]
    pub along: Option<AxisDir>,
    /// The body the edges and faces are on. Default: the one the picks find.
    #[serde(default)]
    pub body: Option<String>,
    #[serde(flatten, deserialize_with = "feature_common")]
    pub common: FeatureCommon,
}

/// An edge or face the dogbone skipped.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DogboneSkip {
    /// A point on it: an edge's middle, a face's corner.
    pub at: [f64; 3],
    /// Why: convex (an outside corner), flat, too_open (more open than `max_angle`),
    /// gap_too_large, degenerate, not_straight, not_between_two_faces, not_planar,
    /// not_along_axis, no_corners (a picked face with none).
    pub reason: String,
}

/// What `solid.dogbone` returns: a feature command's reply, with the corners relieved and the
/// edges skipped.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DogboneResult {
    #[serde(flatten)]
    pub feature: FeatureResult,
    /// How many corners were relieved.
    pub relieved: usize,
    /// The middle of each relieved corner edge.
    pub relieved_edges: Vec<[f64; 3]>,
    /// The edges and faces skipped, and why.
    pub skipped: Vec<DogboneSkip>,
}
