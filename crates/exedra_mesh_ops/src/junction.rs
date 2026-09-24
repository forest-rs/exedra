// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tube junctions: a quad-dominant skin that joins open tube ends at a
//! branch node.
//!
//! A junction takes two or more boundary rings, typically the open inner ends
//! of tubes meeting at a skeleton node, and closes the space between them with
//! a watertight skin. The construction follows the skeletal quad mesh idea
//! (Bærentzen et al., 2012), with the tube rings as the polytope's openings:
//!
//! 1. **Arm graph.** Each ring is an arm with a direction from the junction
//!    center through the ring's centroid. The convex hull of those directions
//!    on the unit sphere triangulates the sphere: its edges are arm pairs
//!    joined by a *bridge*, and its faces are *crotches* between three or more
//!    arms. Hull facets use exact orientation predicates. Coplanar directions
//!    merge into one polygonal crotch. When every arm lies in one hemisphere, a
//!    virtual direction opposite their mean closes the sphere, and the crotches
//!    around it merge into one. Two arms have no crotch: the skin is one
//!    closed strip.
//! 2. **Runs.** Around each arm, its graph neighbours divide the ring by
//!    azimuth into one contiguous run of vertices per neighbour, each facing
//!    that neighbour. One ring edge between consecutive runs belongs to the
//!    crotch between those neighbours.
//! 3. **Bridges.** Each graph edge becomes a strip between the two facing
//!    runs: quads where the runs advance together, triangles where one run has
//!    more vertices.
//! 4. **Crotches.** Each crotch is the polygon of alternating ring edges and
//!    bridge side edges. It becomes a fan of quads around one new center
//!    vertex: the polygon's centroid, domed slightly outward along the crotch
//!    direction.
//!
//! The skin uses only existing ring vertices plus one center vertex per
//! crotch. It traverses every ring edge in the ring's boundary direction, so
//! it welds to the tubes without new seams in the mesh topology.
//!
//! [`plan_junction`] computes the skin from ring positions without a mesh;
//! [`add_junction`] plans from boundary loops of a mesh and adds the faces in
//! an edit session. Every geometric refusal happens while planning, so a
//! refused junction leaves the mesh unchanged.
//!
//! Planning refuses what the construction cannot represent without the skin
//! passing through a tube:
//!
//! - collars closer than their angular radii plus a clearance margin,
//! - a bridge whose arc between two arms passes over a third arm,
//! - a ring lying beyond the plane of a ring it is bridged to, which breaks
//!   the convexity the strips rely on,
//! - a ring whose run toward a neighbour does not face that neighbour,
//! - a twisted skin quad, whose rungs cross,
//! - skin faces that cross each other, tested exactly as a backstop,
//!
//! among the ring and arm-graph degeneracies listed on [`JunctionError`].
//! The exact backstop in [`plan_junction`] sees only the skin; the tube walls
//! are not part of a plan. [`add_junction`] also tests the skin exactly
//! against the tube faces around each ring before it edits the mesh. The
//! geometric refusals are deliberately conservative: they keep the skin off
//! walls that a plan cannot see, and refuse some configurations that would
//! have been clean.
//! The plan also fixes the face insertion order, so the kernel's manifold
//! checks accept every face of an accepted plan. A kernel refusal would leave
//! the faces and center vertices added before it; see
//! [`JunctionError::AddFace`].
//!
//! The coarse skin is quad-dominant and piecewise flat. Opt-in smoothing
//! ([`JunctionSmoothing`]) refines it into rows between the rings and fairs
//! every new vertex with a discrete thin-plate energy whose boundary includes
//! each tube's wall direction, so the skin leaves every ring along its tube
//! and blends into a smooth saddle at each crotch. The smoothed skin passes
//! the same exact checks as the coarse one.
//!
//! Texture coordinates are optional: given the UVs each tube face holds along
//! its ring ([`JunctionOptions::ring_uvs`]), the skin continues every tube's
//! own chart, continuous across each skin/tube boundary; see
//! [`JunctionOptions::ring_uvs`] for where the tubes' seams continue.

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroU32;

use hashbrown::HashSet;

use exedra_mesh::{
    BoundaryLoopError, ChangeSink, CornerId, EditSession, FaceId, HalfEdgeId, VertexId, op,
};
use exedra_triangulate::predicates::{Orientation, Orientation3d, orient2d, orient3d};

/// Largest number of rings one junction accepts.
///
/// Arm graphs are built with an exact hull over the arm directions that is
/// quartic in the ring count (every triple, tested against every direction);
/// the bound keeps that cost trivial.
pub const MAX_JUNCTION_RINGS: usize = 16;

const TAU: f64 = core::f64::consts::TAU;
const PI: f64 = core::f64::consts::PI;

/// Typed junction refusal.
#[derive(Clone, Debug, PartialEq)]
pub enum JunctionError {
    /// Fewer than two rings.
    TooFewRings,
    /// More than [`MAX_JUNCTION_RINGS`] rings.
    TooManyRings {
        /// Supplied ring count.
        count: usize,
    },
    /// A ring seed does not name a traversable boundary loop.
    Boundary {
        /// Ring index.
        ring: usize,
        /// Kernel traversal error.
        error: BoundaryLoopError,
    },
    /// Two rings share a vertex, or one ring repeats a vertex.
    SharedVertex {
        /// Ring index.
        ring: usize,
        /// Ring that already holds the vertex (may equal `ring`).
        other: usize,
    },
    /// A ring has fewer than three vertices or no area.
    DegenerateRing {
        /// Ring index.
        ring: usize,
    },
    /// A ring or the center has a non-finite coordinate.
    NonFinite {
        /// Ring index, or `None` for the center.
        ring: Option<usize>,
    },
    /// A ring opens toward the junction center instead of away from it: its
    /// boundary direction is reversed, or the center lies on the ring's outer
    /// side.
    RingFacesCenter {
        /// Ring index.
        ring: usize,
    },
    /// A ring is not star-shaped around its axis, so its vertices cannot be
    /// divided into azimuth runs.
    RingNotStarShaped {
        /// Ring index.
        ring: usize,
    },
    /// Two rings overlap or nearly touch as seen from the junction center:
    /// their arms are closer in angle than the sum of the rings' angular
    /// radii widened by [`COLLAR_CLEARANCE`].
    OverlappingCollars {
        /// First ring.
        a: usize,
        /// Second ring.
        b: usize,
    },
    /// The arm directions admit no valid arm graph: several coincide in
    /// direction, one lies inside a hull facet, or the rotation around an arm
    /// disagrees with its geometry.
    ArmsDegenerate,
    /// A ring has fewer vertices facing some neighbour than the skin needs:
    /// every neighbour needs at least one.
    RingTooCoarse {
        /// Ring index.
        ring: usize,
        /// Number of graph neighbours of the ring.
        neighbours: usize,
    },
    /// The bridge between rings `a` and `b` would pass over ring `through`:
    /// the great-circle arc between the two arms comes within `through`'s
    /// angular radius.
    BridgeCrossesArm {
        /// First bridged ring.
        a: usize,
        /// Second bridged ring.
        b: usize,
        /// Ring the bridge would pass through.
        through: usize,
    },
    /// A vertex of `neighbour`'s run facing `ring` lies beyond `ring`'s
    /// plane, on its outer side. Bridges assume each ring sees its bridged
    /// neighbours on the junction side; otherwise a strip dips into the tube
    /// wall.
    RingBeyondNeighbour {
        /// Ring whose plane is crossed.
        ring: usize,
        /// Bridged ring with a vertex beyond it.
        neighbour: usize,
    },
    /// The run of `ring`'s vertices that faces `neighbour` does not contain
    /// the neighbour's direction, so the bridge would leave the ring on the
    /// wrong side. This happens when a ring is tilted far from its arm or
    /// when several arms crowd one side of a ring.
    RunMisaligned {
        /// Ring whose runs are misaligned.
        ring: usize,
        /// Neighbour its run fails to face.
        neighbour: usize,
    },
    /// Ring UVs do not list one finite pair per ring edge of every ring.
    InvalidRingUvs {
        /// Offending ring, or `None` when the ring count is wrong.
        ring: Option<usize>,
    },
    /// [`add_junction`] was asked to continue the tubes' UVs, but a tube face
    /// along the ring has no corner UVs.
    MissingTubeUvs {
        /// Ring index.
        ring: usize,
    },
    /// Smoothing tangents do not list one finite direction per ring vertex
    /// that points from the ring toward the junction side of its plane.
    InvalidSmoothing {
        /// Offending ring, or `None` when the ring count is wrong.
        ring: Option<usize>,
    },
    /// Fairing the smoothed skin produced non-finite positions.
    SmoothingDiverged,
    /// Smoothing met a coarse skin face whose shape it cannot refine without
    /// leaving T-vertices. The coarse construction produces only bridge
    /// quads, bridge triangles and crotch quads, so this is an internal
    /// invariant failure rather than an input condition.
    UnrefinableFace {
        /// The coarse face, indexing the unsmoothed plan's faces.
        face: usize,
    },
    /// A crotch center could not be placed: the crotch has no outward
    /// direction.
    CenterDegenerate {
        /// Crotch index in plan order.
        crotch: usize,
    },
    /// A planned skin quad is twisted: along each diagonal, its two triangles
    /// face opposite ways, so the quad's rungs cross. Face-to-face tests do
    /// not see a fold inside one quad.
    SkinTwisted {
        /// The face, indexing [`JunctionPlan::faces`].
        face: usize,
    },
    /// Two planned skin faces intersect. Their triangles are tested exactly,
    /// splitting quads along both diagonals, so the refusal does not depend
    /// on how a consumer triangulates the skin.
    SkinSelfIntersects {
        /// The two faces, indexing [`JunctionPlan::faces`].
        faces: [usize; 2],
    },
    /// A smoothed skin quad is twisted, as [`JunctionError::SkinTwisted`]
    /// for the coarse skin. The coarse skin passed its checks, so the
    /// junction plans without smoothing.
    SmoothedSkinTwisted {
        /// The face, indexing [`JunctionPlan::faces`] of the smoothed plan.
        face: usize,
    },
    /// Two smoothed skin faces intersect, as
    /// [`JunctionError::SkinSelfIntersects`] for the coarse skin. The coarse
    /// skin passed its checks, so the junction plans without smoothing.
    SmoothedSkinSelfIntersects {
        /// The two faces, indexing [`JunctionPlan::faces`] of the smoothed
        /// plan.
        faces: [usize; 2],
    },
    /// A planned skin face intersects a tube face around one of the rings,
    /// tested exactly by [`add_junction`] before any edit.
    SkinCrossesTube {
        /// The skin face, indexing [`JunctionPlan::faces`].
        face: usize,
        /// The tube face it crosses.
        tube_face: FaceId,
    },
    /// The skin faces admit no insertion order that keeps every vertex
    /// manifold while faces are added one at a time.
    NoAttachOrder,
    /// The kernel refused a skin face. Faces and crotch center vertices added
    /// before it remain.
    AddFace(op::AddFaceError),
}

impl fmt::Display for JunctionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewRings => f.write_str("a junction needs at least two rings"),
            Self::TooManyRings { count } => write!(
                f,
                "a junction accepts at most {MAX_JUNCTION_RINGS} rings, got {count}"
            ),
            Self::Boundary { ring, error } => write!(f, "junction ring {ring}: {error}"),
            Self::SharedVertex { ring, other } => {
                write!(f, "junction ring {ring} shares a vertex with ring {other}")
            }
            Self::DegenerateRing { ring } => write!(f, "junction ring {ring} is degenerate"),
            Self::NonFinite { ring: Some(ring) } => {
                write!(f, "junction ring {ring} has a non-finite coordinate")
            }
            Self::NonFinite { ring: None } => {
                f.write_str("junction center has a non-finite coordinate")
            }
            Self::RingFacesCenter { ring } => {
                write!(f, "junction ring {ring} opens toward the junction center")
            }
            Self::RingNotStarShaped { ring } => {
                write!(f, "junction ring {ring} is not star-shaped around its axis")
            }
            Self::OverlappingCollars { a, b } => {
                write!(
                    f,
                    "junction rings {a} and {b} overlap as seen from the center"
                )
            }
            Self::ArmsDegenerate => f.write_str("junction arm directions are degenerate"),
            Self::RingTooCoarse { ring, neighbours } => write!(
                f,
                "junction ring {ring} needs a vertex facing each of its {neighbours} neighbours"
            ),
            Self::BridgeCrossesArm { a, b, through } => write!(
                f,
                "junction bridge between rings {a} and {b} passes over ring {through}"
            ),
            Self::RingBeyondNeighbour { ring, neighbour } => write!(
                f,
                "junction ring {neighbour} reaches beyond the plane of ring {ring}"
            ),
            Self::RunMisaligned { ring, neighbour } => {
                write!(f, "junction ring {ring} has no run facing ring {neighbour}")
            }
            Self::InvalidRingUvs { ring: Some(ring) } => {
                write!(f, "junction ring {ring} has invalid ring UVs")
            }
            Self::InvalidRingUvs { ring: None } => {
                f.write_str("junction ring UVs do not match the rings")
            }
            Self::MissingTubeUvs { ring } => {
                write!(f, "junction ring {ring} has tube faces without UVs")
            }
            Self::InvalidSmoothing { ring: Some(ring) } => {
                write!(f, "junction ring {ring} has invalid smoothing tangents")
            }
            Self::InvalidSmoothing { ring: None } => {
                f.write_str("junction smoothing tangents do not match the rings")
            }
            Self::SmoothingDiverged => f.write_str("junction smoothing diverged"),
            Self::UnrefinableFace { face } => {
                write!(f, "junction skin face {face} cannot be refined")
            }
            Self::SmoothedSkinTwisted { face } => {
                write!(f, "smoothed junction skin face {face} is twisted")
            }
            Self::SmoothedSkinSelfIntersects { faces } => write!(
                f,
                "smoothed junction skin faces {} and {} intersect",
                faces[0], faces[1]
            ),
            Self::CenterDegenerate { crotch } => {
                write!(f, "junction crotch {crotch} has no placeable center")
            }
            Self::SkinTwisted { face } => write!(f, "junction skin face {face} is twisted"),
            Self::SkinSelfIntersects { faces } => write!(
                f,
                "junction skin faces {} and {} intersect",
                faces[0], faces[1]
            ),
            Self::SkinCrossesTube { face, tube_face } => write!(
                f,
                "junction skin face {face} crosses tube face {tube_face:?}"
            ),
            Self::NoAttachOrder => f.write_str("junction faces admit no manifold insertion order"),
            Self::AddFace(error) => write!(f, "junction face insertion: {error}"),
        }
    }
}

impl core::error::Error for JunctionError {}

/// Optional planning stages for [`plan_junction`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JunctionOptions {
    /// Tube UVs to continue across the skin: per ring and per ring edge
    /// `k -> k + 1` (boundary order), the UVs at vertex `k` and vertex
    /// `k + 1` as the tube face across that edge holds them.
    ///
    /// Every skin corner on a ring edge receives exactly those UVs, so the
    /// texture is continuous across each skin/tube boundary whatever chart
    /// the tubes use. New vertices take the discrete harmonic extension of
    /// the ring values. A tube's chart usually jumps across one ring edge
    /// (its U seam); inside the skin that jump continues along a cut, a
    /// shortest path of faces from the seam edge to one sink face: the face
    /// farthest from ring 0 (typically the parent, so the sink falls between
    /// the branches), ties broken by distance from every ring. The texture jumps across one side of each
    /// cut by the tube's own seam jump, which is invisible when the jump is
    /// a whole number of texture repeats. The sink face absorbs whatever the
    /// tubes' jumps fail to cancel (for example a trunk and two branches that
    /// each wrap once), so its texture is compressed. Without smoothing, a
    /// cut through a bridge face that spans both rings also leaves that
    /// face's far ring edge off by the jump; smoothing leaves no face on two
    /// rings.
    pub ring_uvs: Option<Vec<Vec<[[f64; 2]; 2]>>>,
    /// Refines and fairs the skin when set.
    pub smoothing: Option<JunctionSmoothing>,
}

/// Opt-in skin smoothing.
///
/// The coarse skin is refined into `rows` rows between the rings: every
/// non-ring edge is split into `rows` segments (ring edges are never split,
/// as the tube faces own them), bridge faces become stacks of quads, and
/// crotch quads become rows that shorten toward the crotch center. Every new
/// vertex, crotch centers included, is then placed by minimizing a discrete
/// thin-plate energy: the sum of squared umbrella Laplacians over the new
/// vertices and the ring vertices. A ring vertex's umbrella includes a ghost
/// neighbour inside its tube along the wall tangent, so the minimum leaves
/// each ring in the direction of its tube wall: the skin is tangent-continuous
/// with the tubes in the discrete sense, and curvature-minimizing inside.
///
/// The smoothed skin is checked exactly like the coarse one; a smoothed skin
/// that folds or crosses itself is refused as
/// [`JunctionError::SmoothedSkinTwisted`] or
/// [`JunctionError::SmoothedSkinSelfIntersects`].
#[derive(Clone, Debug, PartialEq)]
pub struct JunctionSmoothing {
    /// Segments per bridge rung and crotch spoke. One keeps the coarse faces
    /// and only fairs the crotch centers.
    pub rows: NonZeroU32,
    /// Per ring and ring vertex, the direction in which the tube wall runs
    /// into the vertex, continuing toward the skin. Each must point to the
    /// junction side of its ring's plane. `None` uses each ring's inward
    /// axis, as for a straight tube; [`add_junction`] measures the tube walls
    /// instead.
    pub tangents: Option<Vec<Vec<[f64; 3]>>>,
}

impl Default for JunctionSmoothing {
    fn default() -> Self {
        Self {
            rows: NonZeroU32::new(4).expect("nonzero"),
            tangents: None,
        }
    }
}

/// One vertex of a planned skin face.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum JunctionVertex {
    /// Vertex `index` of ring `ring`, in the ring's boundary order.
    Ring {
        /// Ring index.
        ring: usize,
        /// Vertex index within the ring.
        index: usize,
    },
    /// New skin vertex, indexing [`JunctionPlan::skin_vertices`]: a crotch
    /// center, or a vertex added by smoothing.
    Skin(usize),
}

/// Which part of the skin a face belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JunctionFaceOrigin {
    /// Strip between two neighbouring rings.
    Bridge {
        /// The two rings, ascending.
        rings: [usize; 2],
    },
    /// Fan around a crotch center, between the listed rings in skin order.
    Crotch {
        /// Crotch index in plan order.
        crotch: usize,
        /// Rings around the crotch.
        rings: Vec<usize>,
    },
}

/// One planned skin face.
#[derive(Clone, Debug, PartialEq)]
pub struct JunctionFace {
    /// Vertices in outward counter-clockwise order.
    pub vertices: Vec<JunctionVertex>,
    /// Corner texture coordinates, parallel to `vertices`, when ring UVs
    /// were given.
    pub uvs: Option<Vec<[f64; 2]>>,
    /// Coarse skin part the face belongs to (or was refined from).
    pub origin: JunctionFaceOrigin,
}

/// Counts describing the coarse skin, before any smoothing.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct JunctionStats {
    /// Arm graph edges, one bridge each.
    pub bridges: u64,
    /// Arm graph faces, one crotch each.
    pub crotches: u64,
    /// Quads in bridges.
    pub bridge_quads: u64,
    /// Triangles in bridges, where facing runs differ in length.
    pub bridge_triangles: u64,
    /// Quads in crotches.
    pub crotch_quads: u64,
    /// Whether a virtual arm was needed because every arm lies in one
    /// hemisphere.
    pub virtual_arm: bool,
}

/// A planned junction skin, independent of any mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct JunctionPlan {
    /// Positions of new skin vertices: crotch centers first, then vertices
    /// added by smoothing.
    pub skin_vertices: Vec<[f64; 3]>,
    /// Skin faces in deterministic order: bridges by ascending ring pair,
    /// then crotches; with smoothing, each coarse face's refinement in its
    /// place.
    pub faces: Vec<JunctionFace>,
    /// Arm graph edges as ascending ring pairs.
    pub arm_edges: Vec<[usize; 2]>,
    /// Order in which to add `faces` so each one joins the existing faces at
    /// every one of its vertices along an edge, assuming the rings are open
    /// tube ends whose faces own the ring edges in reverse boundary order.
    pub attach_order: Vec<usize>,
    /// Counts.
    pub stats: JunctionStats,
}

/// Fraction of the summed angular radii that two collars must additionally
/// keep apart.
///
/// Rings whose collars nearly touch leave a lens-shaped gap between their
/// facing runs. The runs are cut by crotch azimuths rather than aligned with
/// each other, so in a narrow gap the bridge rungs can cross and fold. A fold
/// inside one quad is refused as [`JunctionError::SkinTwisted`], and folds
/// between faces as [`JunctionError::SkinSelfIntersects`]; the margin reports
/// the cause, touching collars, before either. It is conservative: many
/// configurations it refuses would plan cleanly. A quarter of the summed
/// radii refuses about a tenth more of the tests' random configurations than
/// no margin.
pub const COLLAR_CLEARANCE: f64 = 0.25;

/// Smallest sine of the angle between a neighbour's direction and a ring's
/// axis for which the neighbour's azimuth around the ring is checked against
/// its run. Nearly axial neighbours, such as the far arm of a T junction, have
/// no meaningful azimuth.
const AZIMUTH_CONDITION: f64 = 0.087;

/// Plans a junction skin between rings given as positions.
///
/// Each ring lists its vertices in boundary order: the direction of the open
/// tube's boundary half-edges, clockwise when viewed from outside along the
/// arm. `center` is the branch node the arms radiate from. `options` adds
/// corner UVs and smoothing.
///
/// # Errors
///
/// Returns a [`JunctionError`] for every configuration the construction
/// cannot represent; see its variants and the module documentation.
pub fn plan_junction(
    rings: &[Vec<[f64; 3]>],
    center: [f64; 3],
    options: &JunctionOptions,
) -> Result<JunctionPlan, JunctionError> {
    if rings.len() < 2 {
        return Err(JunctionError::TooFewRings);
    }
    if rings.len() > MAX_JUNCTION_RINGS {
        return Err(JunctionError::TooManyRings { count: rings.len() });
    }
    if !center.iter().all(|c| c.is_finite()) {
        return Err(JunctionError::NonFinite { ring: None });
    }
    let arms = rings
        .iter()
        .enumerate()
        .map(|(ring, points)| Arm::new(ring, points, center))
        .collect::<Result<Vec<_>, _>>()?;

    for a in 0..arms.len() {
        for b in a + 1..arms.len() {
            let angle = angle_between(arms[a].direction, arms[b].direction);
            let reach =
                (arms[a].angular_radius + arms[b].angular_radius) * (1.0 + COLLAR_CLEARANCE);
            if angle <= reach {
                return Err(JunctionError::OverlappingCollars { a, b });
            }
        }
    }

    if let Some(ring_uvs) = &options.ring_uvs {
        chart::check(ring_uvs, rings)?;
    }
    let tangents = match options.smoothing.as_ref().map(|s| s.tangents.as_ref()) {
        None => None,
        Some(None) => Some(smooth::axial_tangents(
            rings,
            &arms.iter().map(|arm| arm.axis).collect::<Vec<_>>(),
        )),
        Some(Some(tangents)) => Some(smooth::checked_tangents(
            tangents,
            rings,
            &arms.iter().map(|arm| arm.axis).collect::<Vec<_>>(),
        )?),
    };
    let finish = |builder: SkinBuilder<'_>, edges: Vec<[usize; 2]>, virtual_arm: bool| {
        builder.finish(rings, edges, virtual_arm, options, tangents.as_deref())
    };

    let mut builder = SkinBuilder::new(rings, center);
    if arms.len() == 2 {
        let all = |ring: usize| (0..rings[ring].len()).collect::<Vec<_>>();
        check_convex(&arms, rings, 0, 1, &all(1))?;
        check_convex(&arms, rings, 1, 0, &all(0))?;
        builder.closed_strip(&arms);
        return finish(builder, vec![[0, 1]], false);
    }

    let directions = arms.iter().map(|arm| arm.direction).collect::<Vec<_>>();
    let (faces, virtual_arm) = arm_graph_faces(&directions)?;
    let rotations = rotations(arms.len(), &faces)?;

    let mut edges = Vec::new();
    for (i, rotation) in rotations.iter().enumerate() {
        for &(j, _) in rotation {
            if i < j {
                edges.push([i, j]);
            }
        }
    }
    edges.sort_unstable();
    for &[i, j] in &edges {
        check_bridge_arc(&arms, i, j)?;
    }

    let runs = arms
        .iter()
        .zip(&rotations)
        .map(|(arm, rotation)| arm.runs(rotation, &faces, &directions))
        .collect::<Result<Vec<_>, _>>()?;
    for &[i, j] in &edges {
        check_convex(&arms, rings, i, j, run(&runs, j, i))?;
        check_convex(&arms, rings, j, i, run(&runs, i, j))?;
    }

    for &[i, j] in &edges {
        builder.bridge(i, j, run(&runs, i, j), run(&runs, j, i));
    }
    for (crotch, face) in faces.iter().enumerate() {
        builder.crotch(crotch, face, &runs)?;
    }
    finish(builder, edges, virtual_arm)
}

/// Refuses a bridge whose great-circle arc between arms `i` and `j` passes
/// within another arm's angular radius.
///
/// A bridge follows the shorter arc between its arms. Nearly antipodal arms,
/// as in a T junction, have no shorter arc; their bridge follows the runs cut
/// by the crotches on either side, which [`Arm::runs`] checks instead.
fn check_bridge_arc(arms: &[Arm], i: usize, j: usize) -> Result<(), JunctionError> {
    let (a, b) = (arms[i].direction, arms[j].direction);
    let Some(normal) = normalize(cross(a, b)).filter(|_| norm(cross(a, b)) > 1e-6) else {
        return Ok(());
    };
    for (k, arm) in arms.iter().enumerate() {
        if k == i || k == j {
            continue;
        }
        let d = arm.direction;
        let height = dot(d, normal);
        let projected = sub(d, scale(normal, height));
        let on_arc =
            dot(cross(a, projected), normal) >= 0.0 && dot(cross(projected, b), normal) >= 0.0;
        let distance = if on_arc {
            atan2(height.abs(), norm(projected))
        } else {
            angle_between(d, a).min(angle_between(d, b))
        };
        if distance <= arm.angular_radius {
            return Err(JunctionError::BridgeCrossesArm {
                a: i,
                b: j,
                through: k,
            });
        }
    }
    Ok(())
}

/// Refuses a bridge where a vertex of ring `j` (from `run`) lies beyond ring
/// `i`'s plane, on the side away from the junction.
///
/// Strips between two runs stay outside both tubes only when each ring sees
/// the other on the junction side of its plane: the rings then bound a convex
/// region around the node, as the skeletal quad mesh construction assumes.
fn check_convex(
    arms: &[Arm],
    rings: &[Vec<[f64; 3]>],
    i: usize,
    j: usize,
    run: &[usize],
) -> Result<(), JunctionError> {
    let arm = &arms[i];
    let tolerance = 1e-9 * (norm(arm.centroid) + arm.radius);
    for &index in run {
        if dot(sub(rings[j][index], arm.centroid), arm.axis) > tolerance {
            return Err(JunctionError::RingBeyondNeighbour {
                ring: i,
                neighbour: j,
            });
        }
    }
    Ok(())
}

/// Parameters for [`add_junction`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JunctionParams {
    /// Branch node the arms radiate from.
    pub center: [f64; 3],
    /// One boundary half-edge per ring, either the OUTSIDE half-edge or its
    /// interior twin; each names its whole boundary loop.
    pub rings: Vec<HalfEdgeId>,
    /// Continue the tubes' corner UVs across the skin: each ring's UVs are
    /// read from the tube faces across its ring edges and passed as
    /// [`JunctionOptions::ring_uvs`].
    pub continue_uvs: bool,
    /// Optional smoothing. When its tangents are `None`, each ring vertex's
    /// tangent is measured from the tube: the mean direction from its
    /// neighbours off the ring into the vertex.
    pub smoothing: Option<JunctionSmoothing>,
    /// Optional `FACE_REGION` for the skin faces.
    pub region: Option<u32>,
}

/// Created skin topology.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JunctionOutput {
    /// Created faces, parallel to the plan's faces.
    pub faces: Vec<FaceId>,
    /// Skin part of each created face.
    pub origins: Vec<JunctionFaceOrigin>,
    /// Created skin vertices, parallel to [`JunctionPlan::skin_vertices`].
    pub skin_vertices: Vec<VertexId>,
    /// Arm graph edges as ascending ring pairs.
    pub arm_edges: Vec<[usize; 2]>,
    /// Counts.
    pub stats: JunctionStats,
}

/// Closes the space between boundary rings of a mesh with a junction skin.
///
/// Each ring is read in boundary-loop order from its seed half-edge. The skin
/// is planned with [`plan_junction`] and added with [`op::add_face`], so it
/// claims the rings' OUTSIDE half-edges and the result is closed where the
/// rings were open. Corner UVs are written when `params.continue_uvs` is set,
/// and `FACE_REGION` when `params.region` is.
///
/// Caller-defined layers follow their own
/// [`Propagation`](exedra_mesh::attributes::Propagation) rules: each new skin
/// vertex takes the ring vertices' values, and each of its corners the tube
/// corners at those ring vertices, weighted by the harmonic weights that also
/// extend the UVs. A skin corner at a ring vertex takes the tube corner at
/// that vertex across the ring edge its face shares with the tube (or across
/// the ring edge leaving the vertex). Skin faces have no source face, so
/// face layers start empty. Edge tags are stored per edge: ring edges keep
/// the tubes' tags and new skin edges start clear.
///
/// # Errors
///
/// Every refusal except [`JunctionError::AddFace`] happens before the mesh is
/// changed. Faces follow [`JunctionPlan::attach_order`], so the kernel accepts
/// them when the rings are open tube ends as planned; should it refuse one
/// anyway, the skin vertices and the faces added before it remain.
pub fn add_junction<S: ChangeSink>(
    edit: &mut EditSession<'_, S>,
    params: &JunctionParams,
) -> Result<JunctionOutput, JunctionError> {
    if params.rings.len() < 2 {
        return Err(JunctionError::TooFewRings);
    }
    if params.rings.len() > MAX_JUNCTION_RINGS {
        return Err(JunctionError::TooManyRings {
            count: params.rings.len(),
        });
    }
    let mesh = edit.mesh();
    let mut ring_vertices: Vec<Vec<VertexId>> = Vec::with_capacity(params.rings.len());
    let mut ring_points = Vec::with_capacity(params.rings.len());
    let mut ring_edges = Vec::with_capacity(params.rings.len());
    for (ring, &seed) in params.rings.iter().enumerate() {
        let loop_edges = mesh
            .boundary_loop(seed)
            .map_err(|error| JunctionError::Boundary { ring, error })?;
        let mut vertices = Vec::with_capacity(loop_edges.len());
        let mut points = Vec::with_capacity(loop_edges.len());
        for &edge in &loop_edges {
            let vertex = mesh
                .from_vertex(edge)
                .ok_or(JunctionError::DegenerateRing { ring })?;
            let position = mesh
                .vertex_position(vertex)
                .ok_or(JunctionError::DegenerateRing { ring })?;
            for (other, seen) in ring_vertices.iter().enumerate() {
                if seen.contains(&vertex) {
                    return Err(JunctionError::SharedVertex { ring, other });
                }
            }
            if vertices.contains(&vertex) {
                return Err(JunctionError::SharedVertex { ring, other: ring });
            }
            vertices.push(vertex);
            points.push(position.map(f64::from));
        }
        ring_vertices.push(vertices);
        ring_points.push(points);
        ring_edges.push(loop_edges);
    }

    let ring_uvs = if params.continue_uvs {
        Some(tube_ring_uvs(mesh, &ring_edges)?)
    } else {
        None
    };
    let smoothing = params
        .smoothing
        .as_ref()
        .map(|smoothing| JunctionSmoothing {
            rows: smoothing.rows,
            tangents: smoothing
                .tangents
                .clone()
                .or_else(|| Some(tube_tangents(mesh, &ring_vertices, &ring_points))),
        });
    let options = JunctionOptions {
        ring_uvs,
        smoothing,
    };
    let plan = plan_junction(&ring_points, params.center, &options)?;
    check_tube_walls(mesh, &plan, &ring_vertices)?;
    let layers = crate::layers::has_caller_layers(mesh)
        .then(|| SkinLayers::capture(mesh, &plan, &ring_vertices, &ring_edges));

    let skin_vertices = plan
        .skin_vertices
        .iter()
        .map(|position| op::add_vertex(edit, narrow(*position)))
        .collect::<Vec<_>>();
    if let Some(layers) = &layers {
        for (&vertex, captured) in skin_vertices.iter().zip(&layers.vertices) {
            let restored = op::restore_attributes(edit, vertex, captured);
            debug_assert!(restored.is_ok(), "the vertex was just added");
        }
    }
    let loops = plan
        .faces
        .iter()
        .map(|face| {
            face.vertices
                .iter()
                .map(|vertex| match *vertex {
                    JunctionVertex::Ring { ring, index } => ring_vertices[ring][index],
                    JunctionVertex::Skin(index) => skin_vertices[index],
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut ids = vec![None; loops.len()];
    for &index in &plan.attach_order {
        let id = op::add_face(edit, &loops[index]).map_err(JunctionError::AddFace)?;
        ids[index] = Some(id);
    }
    let mut faces = Vec::with_capacity(plan.faces.len());
    let mut origins = Vec::with_capacity(plan.faces.len());
    for ((face, loop_vertices), id) in plan.faces.iter().zip(&loops).zip(ids) {
        let id = id.expect("every planned face was added");
        if let Some(region) = params.region {
            let set = op::set_face_region(edit, id, region);
            debug_assert!(set.is_ok(), "the face was just added");
        }
        if let Some(layers) = &layers {
            let corners: Vec<CornerId> = edit.mesh().face_loop(id).collect();
            for corner in corners {
                let Some(slot) = edit
                    .mesh()
                    .to_vertex(corner)
                    .and_then(|vertex| loop_vertices.iter().position(|v| *v == vertex))
                else {
                    continue;
                };
                let captured = layers.corner(&face.vertices, slot);
                let restored = op::restore_attributes(edit, corner, captured);
                debug_assert!(restored.is_ok(), "the corner belongs to a face just added");
            }
        }
        if let Some(uvs) = &face.uvs {
            let corners: Vec<CornerId> = edit.mesh().face_loop(id).collect();
            for corner in corners {
                let Some(vertex) = edit.mesh().to_vertex(corner) else {
                    continue;
                };
                if let Some(slot) = loop_vertices.iter().position(|v| *v == vertex) {
                    let uv = uvs[slot];
                    let set = op::set_corner_uv(edit, corner, narrow_uv(uv));
                    debug_assert!(set.is_ok(), "the corner belongs to a face just added");
                }
            }
        }
        faces.push(id);
        origins.push(face.origin.clone());
    }
    Ok(JunctionOutput {
        faces,
        origins,
        skin_vertices,
        arm_edges: plan.arm_edges,
        stats: plan.stats,
    })
}

/// Caller-defined values for the skin, captured from the tubes before any
/// edit.
///
/// New skin vertices take the ring vertices' values, and their corners the
/// tube corners at those ring vertices, weighted by the harmonic weights
/// that also extend the UVs ([`chart::ring_weights`]). Each layer's own
/// `Propagation` rule decides what a weighted capture means (`Interpolate`
/// blends, `Copy` takes the heaviest). A skin corner at a ring vertex takes
/// the tube corner at that vertex across the ring edge the skin face shares
/// with the tube, or across the ring edge leaving the vertex when the face
/// has none. Skin faces have no source face, so face layers start empty.
struct SkinLayers {
    vertices: Vec<exedra_mesh::attributes::CapturedAttributes>,
    skin_corners: Vec<exedra_mesh::attributes::CapturedAttributes>,
    /// Per ring and ring edge `k -> k + 1`: the tube corners at `k` and at
    /// `k + 1` across that edge.
    ring_corners: Vec<Vec<[exedra_mesh::attributes::CapturedAttributes; 2]>>,
}

impl SkinLayers {
    fn capture(
        mesh: &exedra_mesh::Mesh,
        plan: &JunctionPlan,
        ring_vertices: &[Vec<VertexId>],
        ring_edges: &[Vec<HalfEdgeId>],
    ) -> Self {
        let lens = ring_vertices.iter().map(Vec::len).collect::<Vec<_>>();
        let weights = chart::ring_weights(&lens, plan.skin_vertices.len(), &plan.faces);
        // The tube corner at ring vertex k across edge k -> k + 1 is the
        // twin of the OUTSIDE half-edge leaving k; the corner at k + 1 is the
        // half-edge before it in the tube face.
        let tube_corner = |ring: usize, index: usize| mesh.twin(ring_edges[ring][index]);
        let vertices = weights
            .iter()
            .map(|list| {
                let sources = list
                    .iter()
                    .map(|&((ring, index), w)| (ring_vertices[ring][index], narrow_weight(w)))
                    .collect::<Vec<_>>();
                mesh.capture_attributes(&sources)
            })
            .collect();
        let skin_corners = weights
            .iter()
            .map(|list| {
                let sources = list
                    .iter()
                    .filter_map(|&((ring, index), w)| {
                        Some((tube_corner(ring, index)?, narrow_weight(w)))
                    })
                    .collect::<Vec<_>>();
                mesh.capture_attributes(&sources)
            })
            .collect();
        let ring_corners = ring_edges
            .iter()
            .enumerate()
            .map(|(ring, edges)| {
                (0..edges.len())
                    .map(|index| {
                        let at = tube_corner(ring, index);
                        let next = at.and_then(|corner| mesh.prev(corner));
                        [at, next].map(|corner| {
                            mesh.capture_attributes(corner.map(|c| (c, 1.0)).as_slice())
                        })
                    })
                    .collect()
            })
            .collect();
        Self {
            vertices,
            skin_corners,
            ring_corners,
        }
    }

    /// The capture for the corner at `face[slot]`.
    fn corner(
        &self,
        face: &[JunctionVertex],
        slot: usize,
    ) -> &exedra_mesh::attributes::CapturedAttributes {
        let n = face.len();
        match face[slot] {
            JunctionVertex::Skin(index) => &self.skin_corners[index],
            JunctionVertex::Ring { ring, index } => {
                let len = self.ring_corners[ring].len();
                let same_ring = |vertex: JunctionVertex, want: usize| {
                    vertex == JunctionVertex::Ring { ring, index: want }
                };
                if same_ring(face[(slot + 1) % n], (index + 1) % len) {
                    // The face runs along ring edge index -> index + 1.
                    &self.ring_corners[ring][index][0]
                } else if same_ring(face[(slot + n - 1) % n], (index + len - 1) % len) {
                    // The face runs along ring edge index - 1 -> index.
                    &self.ring_corners[ring][(index + len - 1) % len][1]
                } else {
                    &self.ring_corners[ring][index][0]
                }
            }
        }
    }
}

fn narrow_weight(weight: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "attribute capture weights are f32"
    )]
    let weight = weight as f32;
    weight
}

/// Reads each ring edge's UVs from the tube face across it.
///
/// A ring's OUTSIDE half-edge `k -> k + 1` has the tube's half-edge
/// `k + 1 -> k` as its twin; that half-edge is the corner at `k`, and the
/// one before it in the tube face is the corner at `k + 1`.
fn tube_ring_uvs(
    mesh: &exedra_mesh::Mesh,
    ring_edges: &[Vec<HalfEdgeId>],
) -> Result<Vec<Vec<[[f64; 2]; 2]>>, JunctionError> {
    let layer = mesh.attrs().sparse(exedra_mesh::attr::CORNER_UV);
    ring_edges
        .iter()
        .enumerate()
        .map(|(ring, edges)| {
            edges
                .iter()
                .map(|&edge| {
                    let uv = |corner: Option<HalfEdgeId>| {
                        corner
                            .and_then(|corner| layer?.get(corner.into()).copied())
                            .map(|uv| uv.map(f64::from))
                            .ok_or(JunctionError::MissingTubeUvs { ring })
                    };
                    let tube = mesh.twin(edge);
                    Ok([uv(tube)?, uv(tube.and_then(|t| mesh.prev(t)))?])
                })
                .collect()
        })
        .collect()
}

/// Measures each ring vertex's tube wall tangent: the mean unit direction
/// from its neighbours off the ring into the vertex.
///
/// A vertex with no such neighbour, or whose mean does not point to the
/// junction side of the ring's plane, keeps the ring's inward axis.
fn tube_tangents(
    mesh: &exedra_mesh::Mesh,
    ring_vertices: &[Vec<VertexId>],
    ring_points: &[Vec<[f64; 3]>],
) -> Vec<Vec<[f64; 3]>> {
    let limit = mesh.half_edges().count();
    ring_vertices
        .iter()
        .zip(ring_points)
        .map(|(vertices, points)| {
            let inward = inward_axis(points);
            vertices
                .iter()
                .zip(points)
                .map(|(&vertex, &point)| {
                    let mut sum = [0.0; 3];
                    if let Some(first) = mesh.vertex_out(vertex) {
                        let mut edge = first;
                        for _ in 0..limit {
                            if let Some(other) = mesh.to_vertex(edge)
                                && !vertices.contains(&other)
                                && let Some(p) = mesh.vertex_position(other)
                                && let Some(d) = normalize(sub(point, p.map(f64::from)))
                            {
                                sum = add(sum, d);
                            }
                            match mesh.twin(edge).and_then(|twin| mesh.next(twin)) {
                                Some(next) if next != first => edge = next,
                                _ => break,
                            }
                        }
                    }
                    normalize(sum)
                        .filter(|t| dot(*t, inward) > 0.0)
                        .unwrap_or(inward)
                })
                .collect()
        })
        .collect()
}

/// A ring's unit axis toward the junction: Newell's normal follows the
/// boundary order, which winds clockwise seen from outside, so it points back
/// toward the junction. Zero for a degenerate ring, which planning refuses.
fn inward_axis(points: &[[f64; 3]]) -> [f64; 3] {
    let mut newell = [0.0; 3];
    for (k, p) in points.iter().enumerate() {
        let q = points[(k + 1) % points.len()];
        newell[0] += (p[1] - q[1]) * (p[2] + q[2]);
        newell[1] += (p[2] - q[2]) * (p[0] + q[0]);
        newell[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    normalize(newell).unwrap_or([0.0; 3])
}

/// Orders faces so each one joins the existing faces around every one of its
/// vertices along an edge.
///
/// The kernel keeps vertices manifold while faces are added one at a time, so
/// a face may not touch an existing fan only at a vertex. Ring vertices start
/// with their tube faces, which own the ring edges in reverse boundary order.
/// The greedy order is deterministic; a skin without such an order is a
/// planning refusal.
fn attach_order(
    rings: &[Vec<[f64; 3]>],
    faces: &[JunctionFace],
) -> Result<Vec<usize>, JunctionError> {
    let mut used: HashSet<(JunctionVertex, JunctionVertex)> = HashSet::new();
    let mut covered: HashSet<JunctionVertex> = HashSet::new();
    for (ring, points) in rings.iter().enumerate() {
        let len = points.len();
        for index in 0..len {
            let vertex = JunctionVertex::Ring { ring, index };
            let next = JunctionVertex::Ring {
                ring,
                index: (index + 1) % len,
            };
            used.insert((next, vertex));
            covered.insert(vertex);
        }
    }
    let attachable = |face: &[JunctionVertex],
                      used: &HashSet<(JunctionVertex, JunctionVertex)>,
                      covered: &HashSet<JunctionVertex>| {
        let n = face.len();
        (0..n).all(|k| {
            let previous = face[(k + n - 1) % n];
            let vertex = face[k];
            let next = face[(k + 1) % n];
            !covered.contains(&vertex)
                || used.contains(&(vertex, previous))
                || used.contains(&(next, vertex))
        })
    };
    let mut pending = (0..faces.len()).collect::<Vec<_>>();
    let mut order = Vec::with_capacity(faces.len());
    while !pending.is_empty() {
        let pick = pending
            .iter()
            .position(|&face| attachable(&faces[face].vertices, &used, &covered))
            .ok_or(JunctionError::NoAttachOrder)?;
        let face = pending.remove(pick);
        let vertices = &faces[face].vertices;
        let n = vertices.len();
        for k in 0..n {
            used.insert((vertices[k], vertices[(k + 1) % n]));
            covered.insert(vertices[k]);
        }
        order.push(face);
    }
    Ok(order)
}

/// Per-ring geometry.
#[derive(Clone, Debug)]
struct Arm {
    ring: usize,
    len: usize,
    centroid: [f64; 3],
    /// Outward ring axis (away from the junction center).
    axis: [f64; 3],
    /// Unit direction from the junction center to the centroid.
    direction: [f64; 3],
    /// Largest distance from the centroid to a ring vertex.
    radius: f64,
    /// Largest angle between the arm direction and a ring vertex, seen from
    /// the junction center: the half-angle of the cone the ring occupies.
    angular_radius: f64,
    basis: [[f64; 3]; 2],
    /// Azimuth of each vertex around `axis`, counter-clockwise about it.
    azimuths: Vec<f64>,
}

impl Arm {
    fn new(ring: usize, points: &[[f64; 3]], center: [f64; 3]) -> Result<Self, JunctionError> {
        if points.len() < 3 {
            return Err(JunctionError::DegenerateRing { ring });
        }
        if !points.iter().flatten().all(|c| c.is_finite()) {
            return Err(JunctionError::NonFinite { ring: Some(ring) });
        }
        let len = points.len() as f64;
        let centroid = points
            .iter()
            .fold([0.0; 3], |sum, p| add(sum, *p))
            .map(|c| c / len);
        let axis = normalize(scale(inward_axis(points), -1.0))
            .ok_or(JunctionError::DegenerateRing { ring })?;
        let offset = sub(centroid, center);
        let height = dot(offset, axis);
        if height <= 0.0 {
            return Err(JunctionError::RingFacesCenter { ring });
        }
        let direction = normalize(offset).ok_or(JunctionError::RingFacesCenter { ring })?;
        let radius = points
            .iter()
            .map(|p| norm(sub(*p, centroid)))
            .fold(0.0, f64::max);
        if radius <= 0.0 {
            return Err(JunctionError::DegenerateRing { ring });
        }
        // Measured per vertex, so a ring tilted against its arm reports the
        // cone it actually occupies.
        let angular_radius = points
            .iter()
            .map(|p| angle_between(sub(*p, center), direction))
            .fold(0.0, f64::max);

        let helper = if axis[0].abs() <= axis[1].abs() && axis[0].abs() <= axis[2].abs() {
            [1.0, 0.0, 0.0]
        } else if axis[1].abs() <= axis[2].abs() {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let e1 = normalize(sub(helper, scale(axis, dot(helper, axis))))
            .ok_or(JunctionError::DegenerateRing { ring })?;
        let e2 = cross(axis, e1);
        let azimuths = points
            .iter()
            .map(|p| {
                let r = sub(*p, centroid);
                atan2(dot(r, e2), dot(r, e1))
            })
            .collect::<Vec<_>>();

        // Boundary order must turn once, clockwise about the axis.
        let mut total = 0.0;
        for k in 0..azimuths.len() {
            let step = wrap_signed(azimuths[(k + 1) % azimuths.len()] - azimuths[k]);
            if step >= 0.0 {
                return Err(JunctionError::RingNotStarShaped { ring });
            }
            total += step;
        }
        if (total + TAU).abs() > 1e-6 {
            return Err(JunctionError::RingNotStarShaped { ring });
        }

        Ok(Self {
            ring,
            len: points.len(),
            centroid,
            axis,
            direction,
            radius,
            angular_radius,
            basis: [e1, e2],
            azimuths,
        })
    }

    /// Azimuth of a direction projected onto this arm's ring plane.
    fn azimuth_of(&self, direction: [f64; 3]) -> f64 {
        atan2(dot(direction, self.basis[1]), dot(direction, self.basis[0]))
    }

    /// Divides the ring into one run per neighbour.
    ///
    /// `rotation` lists `(neighbour, face after it)` counter-clockwise about
    /// the arm. Neighbour `k`'s run spans the azimuths from the face before
    /// it to the face after it: crotch directions stay well defined even when
    /// a neighbour is antipodal, as in a T junction. Each run lists ring
    /// indices by increasing azimuth.
    ///
    /// A neighbour whose direction has a well-defined azimuth around this
    /// ring's axis must fall inside its own run's span; otherwise the bridge
    /// would leave the ring on the wrong side.
    fn runs(
        &self,
        rotation: &[(usize, usize)],
        faces: &[ArmFace],
        directions: &[[f64; 3]],
    ) -> Result<Vec<(usize, Vec<usize>)>, JunctionError> {
        let face_azimuths = rotation
            .iter()
            .map(|&(_, face)| self.azimuth_of(faces[face].direction))
            .collect::<Vec<_>>();
        // The combinatorial rotation must agree with the geometric one.
        let count = rotation.len();
        let mut turn = 0.0;
        for k in 0..count {
            let gap = wrap_positive(face_azimuths[(k + 1) % count] - face_azimuths[k]);
            if gap <= 0.0 {
                return Err(JunctionError::ArmsDegenerate);
            }
            turn += gap;
        }
        if (turn - TAU).abs() > 1e-6 {
            return Err(JunctionError::ArmsDegenerate);
        }

        // Each vertex joins exactly one sector: the first whose half-open
        // azimuth span contains it, or, when rounding at a boundary leaves it
        // in none, the nearest.
        let starts = (0..count)
            .map(|k| face_azimuths[(k + count - 1) % count])
            .collect::<Vec<_>>();
        let widths = (0..count)
            .map(|k| wrap_positive(face_azimuths[k] - starts[k]))
            .collect::<Vec<_>>();
        for (k, &(neighbour, _)) in rotation.iter().enumerate() {
            let direction = directions[neighbour];
            let planar = [dot(direction, self.basis[0]), dot(direction, self.basis[1])];
            if libm::hypot(planar[0], planar[1]) < AZIMUTH_CONDITION {
                continue;
            }
            // Runs are cut by crotch azimuths, and a neighbour's own azimuth
            // can fall past its run's span; the excess grows with ring tilt
            // and with arms crowding one side of a ring, and clean skins show
            // tens of degrees. The bound is therefore generous: the neighbour
            // must be nearer its own run than the middle of either adjacent
            // run. A neighbour deeper in another run means the bridge would
            // leave on the wrong side.
            let before = widths[(k + count - 1) % count] / 2.0;
            let after = widths[(k + 1) % count] / 2.0;
            let offset = wrap_positive(atan2(planar[1], planar[0]) - starts[k] + before);
            if offset >= before + widths[k] + after {
                return Err(JunctionError::RunMisaligned {
                    ring: self.ring,
                    neighbour,
                });
            }
        }
        let mut members: Vec<Vec<(f64, usize)>> = vec![Vec::new(); count];
        for index in 0..self.len {
            let offsets = (0..count)
                .map(|k| wrap_positive(self.azimuths[index] - starts[k]))
                .collect::<Vec<_>>();
            let sector = (0..count)
                .find(|&k| offsets[k] < widths[k])
                .or_else(|| {
                    (0..count).min_by(|&a, &b| {
                        (offsets[a] - widths[a]).total_cmp(&(offsets[b] - widths[b]))
                    })
                })
                .expect("a ring has neighbours");
            members[sector].push((offsets[sector], index));
        }
        let mut runs = Vec::with_capacity(count);
        for (k, mut members) in members.into_iter().enumerate() {
            if members.is_empty() {
                return Err(JunctionError::RingTooCoarse {
                    ring: self.ring,
                    neighbours: count,
                });
            }
            members.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            runs.push((
                rotation[k].0,
                members.into_iter().map(|(_, index)| index).collect(),
            ));
        }
        Ok(runs)
    }
}

fn run(runs: &[Vec<(usize, Vec<usize>)>], arm: usize, neighbour: usize) -> &[usize] {
    runs[arm]
        .iter()
        .find(|(n, _)| *n == neighbour)
        .map(|(_, run)| run.as_slice())
        .expect("every rotation neighbour has a run")
}

/// Fraction of the gap between a crotch polygon's centroid and its mean
/// radius by which the crotch center is raised along the crotch direction.
const CROTCH_DOME: f64 = 0.25;

/// Builds the coarse skin faces.
struct SkinBuilder<'a> {
    rings: &'a [Vec<[f64; 3]>],
    center: [f64; 3],
    skin_vertices: Vec<[f64; 3]>,
    faces: Vec<JunctionFace>,
    stats: JunctionStats,
}

impl<'a> SkinBuilder<'a> {
    fn new(rings: &'a [Vec<[f64; 3]>], center: [f64; 3]) -> Self {
        Self {
            rings,
            center,
            skin_vertices: Vec::new(),
            faces: Vec::new(),
            stats: JunctionStats::default(),
        }
    }

    fn position(&self, vertex: JunctionVertex) -> [f64; 3] {
        match vertex {
            JunctionVertex::Ring { ring, index } => self.rings[ring][index],
            JunctionVertex::Skin(index) => self.skin_vertices[index],
        }
    }

    fn push(&mut self, vertices: Vec<JunctionVertex>, origin: JunctionFaceOrigin) {
        self.faces.push(JunctionFace {
            vertices,
            uvs: None,
            origin,
        });
    }

    /// Two rings: one closed strip.
    ///
    /// The rings are aligned by the rotation that carries the first ring's
    /// inward axis onto the second ring's outward axis, as a bent tube's
    /// rotation-minimising frame would: each vertex pairs with the one whose
    /// offset from its centroid points the same way after that rotation. Any
    /// other alignment twists the strip, and rung lengths alone cannot tell the
    /// twist apart when one ring is much smaller than the other.
    fn closed_strip(&mut self, arms: &[Arm]) {
        let n0 = self.rings[0].len();
        let n1 = self.rings[1].len();
        let a = (0..=n0)
            .map(|s| JunctionVertex::Ring {
                ring: 0,
                index: s % n0,
            })
            .collect::<Vec<_>>();
        let aligned = |start: usize| {
            (0..=n1)
                .map(|t| JunctionVertex::Ring {
                    ring: 1,
                    index: (start + n1 * 2 - t) % n1,
                })
                .collect::<Vec<_>>()
        };
        let transport = Transport::new(scale(arms[0].axis, -1.0), arms[1].axis);
        let offset = |vertex: JunctionVertex, arm: &Arm| {
            normalize(sub(self.position(vertex), arm.centroid)).unwrap_or([0.0; 3])
        };
        let start = (0..n1)
            .map(|start| {
                let b = aligned(start);
                let agreement = self.strip_pairs(a.len(), b.len()).fold(0.0, |sum, (s, t)| {
                    sum + dot(
                        transport.apply(offset(a[s], &arms[0])),
                        offset(b[t], &arms[1]),
                    )
                });
                (agreement, start)
            })
            .max_by(|x, y| x.0.total_cmp(&y.0).then(y.1.cmp(&x.1)))
            .expect("rings have vertices")
            .1;
        let b = aligned(start);
        self.strip(&a, &b, [0, 1]);
        self.stats.bridges = 1;
    }

    /// The `(a, b)` index pairs joined by rungs in [`Self::strip`] for
    /// polylines of `len_a` and `len_b` vertices.
    fn strip_pairs(&self, len_a: usize, len_b: usize) -> impl Iterator<Item = (usize, usize)> {
        let p = len_a - 1;
        let q = len_b - 1;
        let (long, short) = (p.max(q), p.min(q));
        let (mut s, mut t) = (0, 0);
        core::iter::once((0, 0)).chain((0..long).map(move |step| {
            if (step + 1) * short / long > step * short / long {
                s += 1;
                t += 1;
            } else if p > q {
                s += 1;
            } else {
                t += 1;
            }
            (s.min(p), t.min(q))
        }))
    }

    /// Strip between polyline `a` (boundary order) and polyline `b` (reverse
    /// boundary order), starting at the pair `(a[0], b[0])`; see
    /// [`strip_faces`].
    fn strip(&mut self, a: &[JunctionVertex], b: &[JunctionVertex], rings: [usize; 2]) {
        let origin = JunctionFaceOrigin::Bridge { rings };
        for (face, quad) in strip_faces(a, b) {
            if quad {
                self.stats.bridge_quads += 1;
            } else {
                self.stats.bridge_triangles += 1;
            }
            self.push(face, origin.clone());
        }
    }

    /// Bridge between arm `i`'s run facing `j` and arm `j`'s run facing `i`.
    fn bridge(&mut self, i: usize, j: usize, run_i: &[usize], run_j: &[usize]) {
        // Arm i's run from high to low azimuth follows its boundary order;
        // arm j's run from low to high azimuth is its reverse.
        let a = run_i
            .iter()
            .rev()
            .map(|&index| JunctionVertex::Ring { ring: i, index })
            .collect::<Vec<_>>();
        let b = run_j
            .iter()
            .map(|&index| JunctionVertex::Ring { ring: j, index })
            .collect::<Vec<_>>();
        self.strip(&a, &b, [i, j]);
        self.stats.bridges += 1;
    }

    /// Crotch fan for one arm graph face, arms counter-clockwise from outside.
    fn crotch(
        &mut self,
        crotch: usize,
        face: &ArmFace,
        runs: &[Vec<(usize, Vec<usize>)>],
    ) -> Result<(), JunctionError> {
        let direction = face.direction;
        let face = face.arms.as_slice();
        let d = face.len();
        let mut polygon = Vec::with_capacity(2 * d);
        for m in 0..d {
            let arm = face[m];
            let previous = face[(m + d - 1) % d];
            let next = face[(m + 1) % d];
            let low = run(runs, arm, previous)[0];
            let high = *run(runs, arm, next).last().expect("runs are nonempty");
            polygon.push(JunctionVertex::Ring {
                ring: arm,
                index: low,
            });
            polygon.push(JunctionVertex::Ring {
                ring: arm,
                index: high,
            });
        }
        let points = polygon
            .iter()
            .map(|v| self.position(*v))
            .collect::<Vec<_>>();
        let count = points.len() as f64;
        let mean = points
            .iter()
            .fold([0.0; 3], |sum, p| add(sum, *p))
            .map(|c| c / count);
        let distance = points
            .iter()
            .map(|p| norm(sub(*p, self.center)))
            .sum::<f64>()
            / count;
        let direction = normalize(direction).ok_or(JunctionError::CenterDegenerate { crotch })?;
        // Dome the centroid part of the way toward the polygon's mean radius
        // along the crotch direction: flat crotches stay nearly flat, and no
        // crotch spikes out or folds inward.
        let height = dot(sub(mean, self.center), direction);
        let dome = CROTCH_DOME * (distance - height).max(0.0);
        let center_index = self.skin_vertices.len();
        // Stored at the mesh's f32 precision, so the exact checks see the
        // position the mesh will hold.
        self.skin_vertices
            .push(narrow(add(mean, scale(direction, dome))).map(f64::from));
        let center = JunctionVertex::Skin(center_index);
        let origin = JunctionFaceOrigin::Crotch {
            crotch,
            rings: face.to_vec(),
        };
        for m in 0..d {
            let quad = vec![
                polygon[2 * m],
                polygon[2 * m + 1],
                polygon[(2 * m + 2) % (2 * d)],
                center,
            ];
            self.push(quad, origin.clone());
            self.stats.crotch_quads += 1;
        }
        self.stats.crotches += 1;
        Ok(())
    }

    fn finish(
        self,
        rings: &[Vec<[f64; 3]>],
        arm_edges: Vec<[usize; 2]>,
        virtual_arm: bool,
        options: &JunctionOptions,
        tangents: Option<&[Vec<[f64; 3]>]>,
    ) -> Result<JunctionPlan, JunctionError> {
        let position = |vertices: &[[f64; 3]], vertex: JunctionVertex| match vertex {
            JunctionVertex::Ring { ring, index } => rings[ring][index],
            JunctionVertex::Skin(index) => vertices[index],
        };
        let coarse = |v| position(&self.skin_vertices, v);
        check_twists(&self.faces, coarse).map_err(|face| JunctionError::SkinTwisted { face })?;
        check_self_intersection(&self.faces, coarse)
            .map_err(|faces| JunctionError::SkinSelfIntersects { faces })?;

        let (skin_vertices, mut faces) = match (&options.smoothing, tangents) {
            (Some(smoothing), Some(tangents)) => {
                let smoothed =
                    smooth::smooth(rings, &self.skin_vertices, &self.faces, tangents, smoothing)?;
                let fine = |v| position(&smoothed.skin_vertices, v);
                check_twists(&smoothed.faces, fine)
                    .map_err(|face| JunctionError::SmoothedSkinTwisted { face })?;
                check_self_intersection(&smoothed.faces, fine)
                    .map_err(|faces| JunctionError::SmoothedSkinSelfIntersects { faces })?;
                (smoothed.skin_vertices, smoothed.faces)
            }
            _ => (self.skin_vertices, self.faces),
        };
        if let Some(ring_uvs) = &options.ring_uvs {
            chart::assign(rings, skin_vertices.len(), &mut faces, ring_uvs);
        }
        let attach_order = attach_order(rings, &faces)?;
        Ok(JunctionPlan {
            skin_vertices,
            faces,
            arm_edges,
            attach_order,
            stats: JunctionStats {
                virtual_arm,
                ..self.stats
            },
        })
    }
}

/// Faces of a strip between polylines `a` and `b` that run side by side, `a`
/// in the faces' orientation and `b` against it, starting at the pair
/// `(a[0], b[0])`. Each face comes with whether it is a quad.
///
/// The longer side advances every step and the shorter one advances on
/// evenly spread steps, so the strip has `min` quads and `|p - q|`
/// triangles for `p` and `q` segments. A side of one vertex is an apex: the
/// strip is then a fan of triangles.
fn strip_faces(a: &[JunctionVertex], b: &[JunctionVertex]) -> Vec<(Vec<JunctionVertex>, bool)> {
    let p = a.len() - 1;
    let q = b.len() - 1;
    let long = p.max(q);
    let short = p.min(q);
    let (mut s, mut t) = (0, 0);
    let mut faces = Vec::with_capacity(long);
    for step in 0..long {
        let short_advances = (step + 1) * short / long > step * short / long;
        let (advance_a, advance_b) = if p >= q {
            (true, short_advances)
        } else {
            (short_advances, true)
        };
        match (advance_a, advance_b) {
            (true, true) => {
                faces.push((vec![a[s], a[s + 1], b[t + 1], b[t]], true));
                s += 1;
                t += 1;
            }
            (true, false) => {
                faces.push((vec![a[s], a[s + 1], b[t]], false));
                s += 1;
            }
            (false, _) => {
                faces.push((vec![a[s], b[t + 1], b[t]], false));
                t += 1;
            }
        }
    }
    debug_assert_eq!((s, t), (p, q), "the strip reaches both ends");
    faces
}

/// Finds a quad whose two triangles fold against each other along both
/// diagonals: its rungs cross, which no face-to-face test sees.
fn check_twists(
    faces: &[JunctionFace],
    position: impl Fn(JunctionVertex) -> [f64; 3],
) -> Result<(), usize> {
    for (index, face) in faces.iter().enumerate() {
        let [a, b, c, d] = match face.vertices.as_slice() {
            &[a, b, c, d] => [a, b, c, d].map(&position),
            _ => continue,
        };
        let normal = |p: [f64; 3], q: [f64; 3], r: [f64; 3]| cross(sub(q, p), sub(r, p));
        let agree = |x: [f64; 3], y: [f64; 3]| dot(x, y) >= 0.0;
        let along_ac = agree(normal(a, b, c), normal(a, c, d));
        let along_bd = agree(normal(a, b, d), normal(b, c, d));
        if !along_ac && !along_bd {
            return Err(index);
        }
    }
    Ok(())
}

/// Finds two skin faces that cross each other.
///
/// Geometric preconditions catch the configurations known to fold; this
/// exact test is the backstop for the rest, such as very coarse rings far
/// from circular. Quads are split along both diagonals, so either
/// triangulation a consumer picks is covered. Its scope is the skin alone:
/// [`add_junction`] tests against the tube walls separately.
fn check_self_intersection(
    faces: &[JunctionFace],
    position: impl Fn(JunctionVertex) -> [f64; 3],
) -> Result<(), [usize; 2]> {
    let triangles = skin_triangles(faces, position);
    let bounds = triangles.iter().map(bounds_of).collect::<Vec<_>>();
    for f in 0..triangles.len() {
        for g in f + 1..triangles.len() {
            if bounds_overlap(bounds[f], bounds[g]) && any_intersect(&triangles[f], &triangles[g]) {
                return Err([f, g]);
            }
        }
    }
    Ok(())
}

/// The shortest rotation taking one unit vector onto another.
#[derive(Copy, Clone, Debug)]
struct Transport {
    axis: [f64; 3],
    cos: f64,
    sin: f64,
}

impl Transport {
    fn new(from: [f64; 3], to: [f64; 3]) -> Self {
        let raw = cross(from, to);
        let cos = dot(from, to).clamp(-1.0, 1.0);
        match normalize(raw) {
            Some(axis) => Self {
                axis,
                cos,
                sin: norm(raw),
            },
            // Parallel or antiparallel: turn about any perpendicular axis.
            None => {
                let helper = if from[0].abs() < 0.9 {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                Self {
                    axis: normalize(cross(from, helper)).unwrap_or([0.0, 0.0, 1.0]),
                    cos,
                    sin: 0.0,
                }
            }
        }
    }

    /// Rodrigues' rotation of `v`.
    fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        let k = self.axis;
        add(
            add(scale(v, self.cos), scale(cross(k, v), self.sin)),
            scale(k, dot(k, v) * (1.0 - self.cos)),
        )
    }
}

/// One arm graph face: arms counter-clockwise from outside, and the outward
/// direction of the crotch it becomes.
#[derive(Clone, Debug)]
struct ArmFace {
    arms: Vec<usize>,
    direction: [f64; 3],
}

/// Arm graph faces for three or more unit directions around the origin.
/// Returns whether a virtual arm was merged.
fn arm_graph_faces(directions: &[[f64; 3]]) -> Result<(Vec<ArmFace>, bool), JunctionError> {
    if let Some(faces) = sphere_faces(directions)? {
        return Ok((faces, false));
    }
    // Every arm lies in one open hemisphere: close the sphere with a virtual
    // arm opposite their mean, which no arm can coincide with.
    let sum = directions.iter().fold([0.0; 3], |sum, d| add(sum, *d));
    let virtual_direction = normalize(scale(sum, -1.0)).ok_or(JunctionError::ArmsDegenerate)?;
    let mut extended = directions.to_vec();
    extended.push(virtual_direction);
    let faces = sphere_faces(&extended)?.ok_or(JunctionError::ArmsDegenerate)?;
    Ok((
        merge_around(&faces, directions.len(), virtual_direction)?,
        true,
    ))
}

/// Faces of the arm graph over `points`, or `None` when the points are not
/// coplanar and the origin is not strictly inside their convex hull.
///
/// Coplanar points (always the case for three) lie on one circle of the
/// sphere, which splits it into two faces: the polygon seen from either side.
fn sphere_faces(points: &[[f64; 3]]) -> Result<Option<Vec<ArmFace>>, JunctionError> {
    const ORIGIN: [f64; 3] = [0.0; 3];
    let n = points.len();
    let Some((a, b, c)) = first_non_collinear(points) else {
        return Err(JunctionError::ArmsDegenerate);
    };
    let coplanar = (0..n)
        .all(|d| orient3d(points[a], points[b], points[c], points[d]) == Orientation3d::Coplanar);
    if coplanar {
        let normal = cross(sub(points[b], points[a]), sub(points[c], points[a]));
        let all = (0..n).collect::<Vec<_>>();
        let front = convex_order(points, &all, normal)?;
        let mut back = front.clone();
        back.reverse();
        let start = (0..back.len())
            .min_by_key(|&k| back[k])
            .expect("faces have vertices");
        back.rotate_left(start);
        let direction = normalize(normal).ok_or(JunctionError::ArmsDegenerate)?;
        return Ok(Some(vec![
            ArmFace {
                arms: front,
                direction,
            },
            ArmFace {
                arms: back,
                direction: scale(direction, -1.0),
            },
        ]));
    }

    let mut faces: Vec<ArmFace> = Vec::new();
    let mut seen: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            for k in j + 1..n {
                if collinear(points[i], points[j], points[k]) {
                    continue;
                }
                let mut above = false;
                let mut below = false;
                let mut on_plane = vec![i, j, k];
                for (d, point) in points.iter().enumerate() {
                    if d == i || d == j || d == k {
                        continue;
                    }
                    match orient3d(points[i], points[j], points[k], *point) {
                        Orientation3d::Above => above = true,
                        Orientation3d::Below => below = true,
                        Orientation3d::Coplanar => on_plane.push(d),
                    }
                }
                if above && below {
                    continue;
                }
                on_plane.sort_unstable();
                if seen.contains(&on_plane) {
                    continue;
                }
                seen.push(on_plane.clone());
                // Outward normal: away from the other points.
                let mut normal = cross(sub(points[j], points[i]), sub(points[k], points[i]));
                if above {
                    normal = scale(normal, -1.0);
                }
                faces.push(ArmFace {
                    arms: convex_order(points, &on_plane, normal)?,
                    direction: normalize(normal).ok_or(JunctionError::ArmsDegenerate)?,
                });
            }
        }
    }
    let inside = faces.iter().all(|face| {
        orient3d(
            points[face.arms[0]],
            points[face.arms[1]],
            points[face.arms[2]],
            ORIGIN,
        ) == Orientation3d::Below
    });
    if !inside {
        return Ok(None);
    }
    let mut faces = merge_near_coplanar(faces)?;
    faces.sort_by(|a, b| a.arms.cmp(&b.arms));
    Ok(Some(faces))
}

/// Angle below which adjacent hull facets merge into one crotch.
///
/// Arms that are nearly coplanar, such as children spread evenly around a
/// parent, give sliver facets whose choice of diagonal is arbitrary and
/// whose narrow azimuth sectors starve the rings of vertices. Merging keeps
/// the crotch polygonal instead.
const MERGE_ANGLE: f64 = 3.0 * PI / 180.0;

/// Merges edge-adjacent facets whose outward normals differ by less than
/// [`MERGE_ANGLE`].
///
/// Merging is bounded, not transitive: a facet joins a group only while every
/// member stays within [`MERGE_ANGLE`] of the group's mean normal, so a chain
/// of slightly bent facets cannot grow into one crotch spanning a wide angle.
fn merge_near_coplanar(faces: Vec<ArmFace>) -> Result<Vec<ArmFace>, JunctionError> {
    let count = faces.len();
    let mut group = (0..count).collect::<Vec<_>>();
    fn root(group: &mut [usize], mut x: usize) -> usize {
        while group[x] != x {
            group[x] = group[group[x]];
            x = group[x];
        }
        x
    }
    let cos_merge = libm::cos(MERGE_ANGLE);
    let edges = |face: &ArmFace| {
        let n = face.arms.len();
        (0..n)
            .map(|k| (face.arms[k], face.arms[(k + 1) % n]))
            .collect::<Vec<_>>()
    };
    for a in 0..count {
        for b in a + 1..count {
            let shares = edges(&faces[a])
                .iter()
                .any(|&(x, y)| edges(&faces[b]).contains(&(y, x)));
            if !shares || dot(faces[a].direction, faces[b].direction) <= cos_merge {
                continue;
            }
            let (ra, rb) = (root(&mut group, a), root(&mut group, b));
            if ra == rb {
                continue;
            }
            let members = (0..count)
                .filter(|&f| {
                    let r = root(&mut group, f);
                    r == ra || r == rb
                })
                .collect::<Vec<_>>();
            let bounded = normalize(
                members
                    .iter()
                    .fold([0.0; 3], |sum, &f| add(sum, faces[f].direction)),
            )
            .is_some_and(|mean| {
                members
                    .iter()
                    .all(|&f| dot(faces[f].direction, mean) > cos_merge)
            });
            if bounded {
                group[ra.max(rb)] = ra.min(rb);
            }
        }
    }
    let mut merged = Vec::new();
    for leader in 0..count {
        if root(&mut group, leader) != leader {
            continue;
        }
        let members = (0..count)
            .filter(|&f| root(&mut group, f) == leader)
            .collect::<Vec<_>>();
        if members.len() == 1 {
            merged.push(faces[leader].clone());
            continue;
        }
        // Boundary: member edges whose reverse is not in another member.
        let all = members
            .iter()
            .flat_map(|&f| edges(&faces[f]))
            .collect::<Vec<_>>();
        let boundary = all
            .iter()
            .copied()
            .filter(|&(x, y)| !all.contains(&(y, x)))
            .collect::<Vec<_>>();
        let start = boundary
            .iter()
            .copied()
            .min()
            .ok_or(JunctionError::ArmsDegenerate)?;
        let mut arms = vec![start.0];
        let mut current = start.1;
        while current != start.0 {
            if arms.contains(&current) || arms.len() > boundary.len() {
                return Err(JunctionError::ArmsDegenerate);
            }
            arms.push(current);
            let mut outgoing = boundary.iter().filter(|(x, _)| *x == current);
            let (Some(&(_, next)), None) = (outgoing.next(), outgoing.next()) else {
                return Err(JunctionError::ArmsDegenerate);
            };
            current = next;
        }
        if arms.len() != boundary.len() {
            return Err(JunctionError::ArmsDegenerate);
        }
        let direction = members
            .iter()
            .fold([0.0; 3], |sum, &f| add(sum, faces[f].direction));
        merged.push(ArmFace {
            arms,
            direction: normalize(direction).ok_or(JunctionError::ArmsDegenerate)?,
        });
    }
    Ok(merged)
}

/// Orders coplanar points counter-clockwise about `normal`, requiring every
/// point to be a strict convex vertex.
fn convex_order(
    points: &[[f64; 3]],
    members: &[usize],
    normal: [f64; 3],
) -> Result<Vec<usize>, JunctionError> {
    let axis = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .expect("three axes");
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let flip = normal[axis] < 0.0;
    let project = |p: [f64; 3]| {
        if flip { [p[v], p[u]] } else { [p[u], p[v]] }
    };
    let count = members.len() as f64;
    let mean = members.iter().fold([0.0; 2], |sum, &m| {
        let p = project(points[m]);
        [sum[0] + p[0], sum[1] + p[1]]
    });
    let mean = [mean[0] / count, mean[1] / count];
    let mut ordered = members.to_vec();
    ordered.sort_by(|&a, &b| {
        let pa = project(points[a]);
        let pb = project(points[b]);
        atan2(pa[1] - mean[1], pa[0] - mean[0])
            .total_cmp(&atan2(pb[1] - mean[1], pb[0] - mean[0]))
            .then(a.cmp(&b))
    });
    let len = ordered.len();
    for k in 0..len {
        let a = project(points[ordered[k]]);
        let b = project(points[ordered[(k + 1) % len]]);
        let c = project(points[ordered[(k + 2) % len]]);
        if orient2d(a, b, c) != Orientation::Ccw {
            return Err(JunctionError::ArmsDegenerate);
        }
    }
    // Start at the smallest index for determinism.
    let start = (0..len)
        .min_by_key(|&k| ordered[k])
        .expect("faces have vertices");
    ordered.rotate_left(start);
    Ok(ordered)
}

/// Merges every face around `vertex` into one face over its link.
fn merge_around(
    faces: &[ArmFace],
    vertex: usize,
    direction: [f64; 3],
) -> Result<Vec<ArmFace>, JunctionError> {
    let mut kept = Vec::new();
    let mut paths: Vec<Vec<usize>> = Vec::new();
    for face in faces {
        if let Some(at) = face.arms.iter().position(|&v| v == vertex) {
            let mut path = face.arms.clone();
            path.rotate_left(at);
            path.remove(0);
            paths.push(path);
        } else {
            kept.push(face.clone());
        }
    }
    let Some(first) = paths.first().cloned() else {
        return Err(JunctionError::ArmsDegenerate);
    };
    let mut merged = first.clone();
    let mut used = vec![false; paths.len()];
    used[0] = true;
    loop {
        let last = *merged.last().expect("paths are nonempty");
        if last == first[0] {
            merged.pop();
            break;
        }
        let Some(next) = (0..paths.len()).find(|&p| !used[p] && paths[p][0] == last) else {
            return Err(JunctionError::ArmsDegenerate);
        };
        used[next] = true;
        merged.extend_from_slice(&paths[next][1..]);
    }
    if used.iter().any(|u| !u) {
        return Err(JunctionError::ArmsDegenerate);
    }
    let start = (0..merged.len())
        .min_by_key(|&k| merged[k])
        .expect("merged face has vertices");
    merged.rotate_left(start);
    kept.push(ArmFace {
        arms: merged,
        direction,
    });
    kept.sort_by(|a, b| a.arms.cmp(&b.arms));
    Ok(kept)
}

/// Counter-clockwise `(neighbour, face after it)` order around each arm.
///
/// A face `(.., prev, i, next, ..)` lies between `next` and `prev` around `i`,
/// counter-clockwise from `next` to `prev`.
fn rotations(arms: usize, faces: &[ArmFace]) -> Result<Vec<Vec<(usize, usize)>>, JunctionError> {
    let mut steps: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); arms];
    for (index, face) in faces.iter().enumerate() {
        let d = face.arms.len();
        for m in 0..d {
            let arm = face.arms[m];
            let next = face.arms[(m + 1) % d];
            let previous = face.arms[(m + d - 1) % d];
            if steps[arm].iter().any(|(from, _, _)| *from == next) {
                return Err(JunctionError::ArmsDegenerate);
            }
            steps[arm].push((next, previous, index));
        }
    }
    steps
        .into_iter()
        .map(|steps| {
            let start = steps
                .iter()
                .map(|(from, _, _)| *from)
                .min()
                .ok_or(JunctionError::ArmsDegenerate)?;
            let mut order = Vec::with_capacity(steps.len());
            let mut current = start;
            loop {
                let (_, to, face) = *steps
                    .iter()
                    .find(|(from, _, _)| *from == current)
                    .ok_or(JunctionError::ArmsDegenerate)?;
                order.push((current, face));
                if to == start {
                    break;
                }
                if order.iter().any(|(n, _)| *n == to) {
                    return Err(JunctionError::ArmsDegenerate);
                }
                current = to;
            }
            if order.len() != steps.len() {
                return Err(JunctionError::ArmsDegenerate);
            }
            Ok(order)
        })
        .collect()
}

fn first_non_collinear(points: &[[f64; 3]]) -> Option<(usize, usize, usize)> {
    let n = points.len();
    for i in 0..n {
        for j in i + 1..n {
            for k in j + 1..n {
                if !collinear(points[i], points[j], points[k]) {
                    return Some((i, j, k));
                }
            }
        }
    }
    None
}

/// Exact collinearity: collinear in every axis-aligned projection.
fn collinear(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> bool {
    (0..3).all(|axis| {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        orient2d([a[u], a[v]], [b[u], b[v]], [c[u], c[v]]) == Orientation::Collinear
    })
}

/// Triangles of one face with their vertex ids: a quad split along both
/// diagonals, other faces as a fan.
type FaceTriangles<V> = Vec<([V; 3], [[f64; 3]; 3])>;

fn split_face<V: Copy>(vertices: &[V], position: impl Fn(V) -> [f64; 3]) -> FaceTriangles<V> {
    let v = vertices;
    let splits: Vec<[V; 3]> = if v.len() == 4 {
        vec![
            [v[0], v[1], v[2]],
            [v[0], v[2], v[3]],
            [v[0], v[1], v[3]],
            [v[1], v[2], v[3]],
        ]
    } else {
        (1..v.len().saturating_sub(1))
            .map(|k| [v[0], v[k], v[k + 1]])
            .collect()
    };
    splits
        .into_iter()
        .map(|ids| (ids, ids.map(&position)))
        .collect()
}

fn skin_triangles(
    faces: &[JunctionFace],
    position: impl Fn(JunctionVertex) -> [f64; 3],
) -> Vec<FaceTriangles<JunctionVertex>> {
    faces
        .iter()
        .map(|face| split_face(&face.vertices, &position))
        .collect()
}

type Bounds = ([f64; 3], [f64; 3]);

fn bounds_of<V>(triangles: &FaceTriangles<V>) -> Bounds {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for (_, points) in triangles {
        for p in points {
            for axis in 0..3 {
                low[axis] = low[axis].min(p[axis]);
                high[axis] = high[axis].max(p[axis]);
            }
        }
    }
    (low, high)
}

fn bounds_overlap((al, ah): Bounds, (bl, bh): Bounds) -> bool {
    (0..3).all(|axis| al[axis] <= bh[axis] && bl[axis] <= ah[axis])
}

fn any_intersect<V: Copy + Eq>(a: &FaceTriangles<V>, b: &FaceTriangles<V>) -> bool {
    a.iter().any(|(ia, pa)| {
        b.iter()
            .any(|(ib, pb)| intersect::triangles_intersect(*ia, pa, *ib, pb))
    })
}

/// Vertex identity shared by skin faces and existing mesh faces.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SkinOrMesh {
    Mesh(VertexId),
    Skin(usize),
}

/// Refuses a planned skin that crosses a tube face around one of its rings.
///
/// The tube faces are those incident to a ring vertex: the band of each tube
/// that the skin meets. Faces share ring vertices by identity, so contact
/// along the ring itself is not a crossing.
fn check_tube_walls(
    mesh: &exedra_mesh::Mesh,
    plan: &JunctionPlan,
    ring_vertices: &[Vec<VertexId>],
) -> Result<(), JunctionError> {
    let identity = |vertex: JunctionVertex| match vertex {
        JunctionVertex::Ring { ring, index } => SkinOrMesh::Mesh(ring_vertices[ring][index]),
        JunctionVertex::Skin(index) => SkinOrMesh::Skin(index),
    };
    let skin_position = |vertex: SkinOrMesh| match vertex {
        SkinOrMesh::Mesh(id) => mesh
            .vertex_position(id)
            .map_or([f64::NAN; 3], |p| p.map(f64::from)),
        SkinOrMesh::Skin(index) => plan.skin_vertices[index],
    };
    let skin = plan
        .faces
        .iter()
        .map(|face| {
            let ids = face
                .vertices
                .iter()
                .map(|v| identity(*v))
                .collect::<Vec<_>>();
            split_face(&ids, skin_position)
        })
        .collect::<Vec<_>>();
    let skin_bounds = skin.iter().map(bounds_of).collect::<Vec<_>>();

    let mut tube_faces = Vec::new();
    let limit = mesh.half_edges().count();
    for &vertex in ring_vertices.iter().flatten() {
        let Some(first) = mesh.vertex_out(vertex) else {
            continue;
        };
        let mut edge = first;
        for _ in 0..limit {
            if let Some(face) = mesh.face(edge)
                && face != FaceId::OUTSIDE
            {
                tube_faces.push(face);
            }
            match mesh.twin(edge).and_then(|twin| mesh.next(twin)) {
                Some(next) if next != first => edge = next,
                _ => break,
            }
        }
    }
    tube_faces.sort_unstable_by_key(|face| face.index());
    tube_faces.dedup();

    for tube_face in tube_faces {
        let ids = mesh
            .face_loop(tube_face)
            .filter_map(|corner| mesh.to_vertex(corner))
            .map(SkinOrMesh::Mesh)
            .collect::<Vec<_>>();
        let triangles = split_face(&ids, skin_position);
        let bounds = bounds_of(&triangles);
        for (face, skin_face) in skin.iter().enumerate() {
            if bounds_overlap(bounds, skin_bounds[face]) && any_intersect(skin_face, &triangles) {
                return Err(JunctionError::SkinCrossesTube { face, tube_face });
            }
        }
    }
    Ok(())
}

fn narrow(p: [f64; 3]) -> [f32; 3] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "skin positions are stored in the mesh's f32 coordinates"
    )]
    p.map(|c| c as f32)
}

fn narrow_uv(uv: [f64; 2]) -> [f32; 2] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "corner UVs are stored as f32"
    )]
    uv.map(|c| c as f32)
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    libm::sqrt(dot(a, a))
}

fn normalize(a: [f64; 3]) -> Option<[f64; 3]> {
    let length = norm(a);
    (length > 0.0 && length.is_finite()).then(|| scale(a, 1.0 / length))
}

fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// Angle between two vectors, accurate at every angle.
fn angle_between(a: [f64; 3], b: [f64; 3]) -> f64 {
    atan2(norm(cross(a, b)), dot(a, b))
}

/// Wraps an angle into `(-π, π]`.
fn wrap_signed(angle: f64) -> f64 {
    let wrapped = angle - TAU * libm::floor((angle + PI) / TAU);
    if wrapped <= -PI {
        wrapped + TAU
    } else {
        wrapped
    }
}

/// Wraps an angle into `[0, 2π)`.
fn wrap_positive(angle: f64) -> f64 {
    let wrapped = angle - TAU * libm::floor(angle / TAU);
    if wrapped >= TAU { 0.0 } else { wrapped }
}

mod chart;
mod intersect;
mod smooth;
#[cfg(test)]
mod tests;
