// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Oracle operands: one solid in three witness forms.
//!
//! Every operand is a planar-faced polyhedron carried as:
//! - an `exedra_mesh` [`Mesh`] (the mesh-boolean witness input),
//! - a union of convex pieces, each a set of f64 half-space planes — for
//!   convex operands one piece derived from the mesh itself; for L/U
//!   prisms an analytic box decomposition mapped through the same rigid
//!   transform (exactly the solid the mesh witness sees, up to the
//!   documented f32 narrowing the mesh band covers),
//! - an `exedra_isosurface` scalar field for the analytic solid the mesh
//!   approximates, plus a Hausdorff bound between the two
//!   (`field_deviation`), which widens the field comparison band.

use exedra_isosurface::{
    ScalarField,
    analytic::{BoxField, CylinderField, SphereField, Union},
    transform::{RigidTransform3, Transform3},
};
use exedra_mesh::{FaceTriangulation, Mesh, op::set_vertex_position};
use exedra_primitives::{
    BoxParams, CylinderParams, UvSphereParams, box_primitive, cylinder, uv_sphere,
};

use crate::rng::SplitMix64;

/// A plane `normal . p - offset <= 0` (inside), normal outward, f64.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Plane {
    /// Outward unit-ish normal (Newell, normalized).
    pub(crate) normal: [f64; 3],
    /// Plane offset: `normal . point_on_plane`.
    pub(crate) offset: f64,
}

/// One operand solid in all three witness forms.
///
/// The referee is a union of convex pieces (`min` over pieces of `max`
/// over planes). Convex operands have exactly one piece; non-convex
/// operands (L/U prisms) decompose into overlapping-or-abutting convex
/// boxes. The value-space soundness argument is unchanged: each piece is
/// one more leaf in the min/max tree, every leaf pseudo-SDF is 1-Lipschitz
/// in space, and min/max composition preserves per-leaf perturbation
/// bounds.
pub(crate) struct Operand {
    /// The transformed mesh consumed by the boolean pipeline.
    pub(crate) mesh: Mesh,
    /// Convex pieces, each a set of outward half-space planes; membership
    /// is the union of the pieces.
    pub(crate) pieces: Vec<Vec<Plane>>,
    /// The analytic field for this solid, placed by the same rigid transform.
    pub(crate) field: Box<dyn ScalarField>,
    /// Hausdorff bound between the mesh solid and the analytic field solid.
    pub(crate) field_deviation: f64,
    /// Human-readable shape tag for reporting minimized reproducers.
    pub(crate) describe: String,
}

/// Rigid placement: rotation columns plus translation, f64.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Rigid {
    /// Rotation matrix columns (local axes in world space).
    pub(crate) cols: [[f64; 3]; 3],
    /// Translation.
    pub(crate) t: [f64; 3],
}

impl Rigid {
    fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        [
            self.cols[0][0] * p[0] + self.cols[1][0] * p[1] + self.cols[2][0] * p[2] + self.t[0],
            self.cols[0][1] * p[0] + self.cols[1][1] * p[1] + self.cols[2][1] * p[2] + self.t[1],
            self.cols[0][2] * p[0] + self.cols[1][2] * p[1] + self.cols[2][2] * p[2] + self.t[2],
        ]
    }
}

/// Samples a random rigid placement. A quarter of placements keep the
/// identity rotation so axis-aligned/near-touching configurations stay
/// common in the case mix.
pub(crate) fn random_rigid(rng: &mut SplitMix64) -> Rigid {
    let t = [
        rng.range_f64(-0.9, 0.9),
        rng.range_f64(-0.9, 0.9),
        rng.range_f64(-0.9, 0.9),
    ];
    if rng.index(4) == 0 {
        return Rigid {
            cols: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            t,
        };
    }
    // Axis-angle rotation from a random unit axis.
    let axis = random_unit(rng);
    let angle = rng.range_f64(0.0, core::f64::consts::TAU);
    let (s, c) = angle.sin_cos();
    let one_c = 1.0 - c;
    let [x, y, z] = axis;
    let cols = [
        [
            c + x * x * one_c,
            x * y * one_c + z * s,
            x * z * one_c - y * s,
        ],
        [
            x * y * one_c - z * s,
            c + y * y * one_c,
            y * z * one_c + x * s,
        ],
        [
            x * z * one_c + y * s,
            y * z * one_c - x * s,
            c + z * z * one_c,
        ],
    ];
    Rigid { cols, t }
}

fn random_unit(rng: &mut SplitMix64) -> [f64; 3] {
    loop {
        let v = [
            rng.range_f64(-1.0, 1.0),
            rng.range_f64(-1.0, 1.0),
            rng.range_f64(-1.0, 1.0),
        ];
        let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
        if len2 > 1.0e-4 && len2 <= 1.0 {
            let inv = 1.0 / len2.sqrt();
            return [v[0] * inv, v[1] * inv, v[2] * inv];
        }
    }
}

/// Builds a random operand: box or regular prism under a random placement.
pub(crate) fn random_operand(rng: &mut SplitMix64) -> Operand {
    random_operand_scaled(rng, 1.0)
}

/// Builds a random operand at a coordinate scale (sizes and translation
/// both scale, so the shape family is identical up to similarity).
pub(crate) fn random_operand_scaled(rng: &mut SplitMix64, scale: f64) -> Operand {
    let mut rigid = random_rigid(rng);
    rigid.t = [rigid.t[0] * scale, rigid.t[1] * scale, rigid.t[2] * scale];
    if rng.index(2) == 0 {
        let size = [
            rng.range_f64(0.5, 1.8) * scale,
            rng.range_f64(0.5, 1.8) * scale,
            rng.range_f64(0.5, 1.8) * scale,
        ];
        box_operand(size, &rigid)
    } else {
        let radius = rng.range_f64(0.3, 0.9) * scale;
        let height = rng.range_f64(0.5, 1.8) * scale;
        let segments = [8_u32, 12, 16, 24][rng.index(4)];
        prism_operand(radius, height, segments, &rigid)
    }
}

/// Builds a random curved-wall operand: a cylindrical prism at a random
/// resolution from 8 to 96 segments — high resolutions generate the
/// collinear cut runs and sliver cascades that stress splitting.
pub(crate) fn random_curved_operand(rng: &mut SplitMix64) -> Operand {
    let rigid = random_rigid(rng);
    let radius = rng.range_f64(0.3, 0.9);
    let height = rng.range_f64(0.5, 1.8);
    let segments = [8_u32, 16, 32, 48, 64, 96][rng.index(6)];
    prism_operand(radius, height, segments, &rigid)
}

/// Builds a random non-convex operand: an L- or U-shaped prism carried as
/// one watertight mesh with a union-of-boxes referee.
pub(crate) fn random_nonconvex_operand(rng: &mut SplitMix64) -> Operand {
    let rigid = random_rigid(rng);
    let width = rng.range_f64(0.9, 1.8);
    let depth = rng.range_f64(0.9, 1.8);
    let height = rng.range_f64(0.5, 1.5);
    if rng.index(2) == 0 {
        let notch_w = width * rng.range_f64(0.3, 0.6);
        let notch_d = depth * rng.range_f64(0.3, 0.6);
        l_prism_operand([width, depth, height], [notch_w, notch_d], &rigid)
    } else {
        let leg = width * rng.range_f64(0.2, 0.35);
        let floor = depth * rng.range_f64(0.25, 0.5);
        u_prism_operand([width, depth, height], [leg, floor], &rigid)
    }
}

/// Box operand: mesh solid and analytic field agree exactly (both are the
/// same box), so the field deviation is pure float slop.
pub(crate) fn box_operand(size: [f64; 3], rigid: &Rigid) -> Operand {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "operand parameters narrow once at construction"
    )]
    let size_f32 = [size[0] as f32, size[1] as f32, size[2] as f32];
    let primitive = box_primitive(&BoxParams {
        size: size_f32,
        centered: true,
        segments: [1, 1, 1],
    });
    let mesh = transform_mesh(&primitive.mesh, rigid);
    let pieces = vec![convex_planes(&mesh)];
    let field = BoxField {
        center: [0.0; 3],
        half_extents: [size_f32[0] * 0.5, size_f32[1] * 0.5, size_f32[2] * 0.5],
    };
    Operand {
        describe: format!(
            "box size=({:.3},{:.3},{:.3}) t=({:.3},{:.3},{:.3})",
            size[0], size[1], size[2], rigid.t[0], rigid.t[1], rigid.t[2]
        ),
        mesh,
        pieces,
        field: Box::new(place_field(field, rigid)),
        field_deviation: 1.0e-4,
    }
}

/// Prism operand: the mesh is a regular n-gon prism inscribed in the round
/// cylinder the field describes; the Hausdorff gap is the chord sagitta.
pub(crate) fn prism_operand(radius: f64, height: f64, segments: u32, rigid: &Rigid) -> Operand {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "operand parameters narrow once at construction"
    )]
    let (radius_f32, height_f32) = (radius as f32, height as f32);
    let primitive = cylinder(&CylinderParams {
        radius: radius_f32,
        height: height_f32,
        segments,
        cap_fill: exedra_primitives::CapFill::Ngon,
        centered: true,
    });
    let mesh = transform_mesh(&primitive.mesh, rigid);
    let pieces = vec![convex_planes(&mesh)];
    let field = CylinderField {
        center: [0.0; 3],
        axis: [0.0, 1.0, 0.0],
        radius: radius_f32,
        half_height: height_f32 * 0.5,
    };
    let sagitta = radius * (1.0 - (core::f64::consts::PI / f64::from(segments)).cos());
    Operand {
        describe: format!(
            "prism r={radius:.3} h={height:.3} n={segments} t=({:.3},{:.3},{:.3})",
            rigid.t[0], rigid.t[1], rigid.t[2]
        ),
        mesh,
        pieces,
        field: Box::new(place_field(field, rigid)),
        field_deviation: sagitta + 1.0e-4,
    }
}

/// Faceted sphere operand with both an exact polyhedral referee and an
/// independent analytic sphere field.
///
/// The UV mesh is convex, so its face planes describe precisely the solid
/// consumed by the Boolean pipeline. The field deviation covers the largest
/// spherical patch represented by one latitude/longitude cell; the diagonal
/// angular bound is deliberately conservative near the poles.
pub(crate) fn sphere_operand(
    radius: f64,
    lat_segments: u32,
    lon_segments: u32,
    rigid: &Rigid,
) -> Operand {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "operand parameters narrow once at construction"
    )]
    let radius_f32 = radius as f32;
    let primitive = uv_sphere(&UvSphereParams {
        radius: radius_f32,
        lat_segments,
        lon_segments,
        centered: true,
    });
    let mesh = transform_mesh(&primitive.mesh, rigid);
    let pieces = vec![convex_planes(&mesh)];
    let field = SphereField {
        center: [0.0; 3],
        radius: radius_f32,
    };
    let latitude_step = core::f64::consts::PI / f64::from(lat_segments + 1);
    let longitude_step = core::f64::consts::TAU / f64::from(lon_segments);
    let angular_diameter = latitude_step.hypot(longitude_step);
    // Using the full cell diameter (rather than assuming its center is the
    // spherical circumcenter of every pole triangle and band quad) keeps the
    // field band conservative for every UV cell shape.
    let sagitta = radius * (1.0 - angular_diameter.cos());
    Operand {
        describe: format!(
            "sphere r={radius:.3} lat={lat_segments} lon={lon_segments} t=({:.3},{:.3},{:.3})",
            rigid.t[0], rigid.t[1], rigid.t[2]
        ),
        mesh,
        pieces,
        field: Box::new(place_field(field, rigid)),
        field_deviation: sagitta + 1.0e-4,
    }
}

/// L-shaped prism: `size = [width, depth, height]`, `notch = [notch_w,
/// notch_d]` removed from the +x/+y corner. One watertight mesh; referee =
/// union of two overlapping boxes; field = union of the same boxes.
pub(crate) fn l_prism_operand(size: [f64; 3], notch: [f64; 2], rigid: &Rigid) -> Operand {
    let [a, b, h] = size;
    let [na, nb] = notch;
    let (cx, cy) = (a * 0.5, b * 0.5);
    let section = vec![
        [-cx, -cy],
        [a - cx, -cy],
        [a - cx, (b - nb) - cy],
        [(a - na) - cx, (b - nb) - cy],
        [(a - na) - cx, b - cy],
        [-cx, b - cy],
    ];
    let mesh = extruded_polygon_mesh(&section, h, rigid);
    // Overlapping pieces keep interior referee margins healthy at the seam.
    let overlap = 0.5 * nb.min(b - nb);
    let pieces = vec![
        box_piece(
            [-cx, -cy, -h * 0.5],
            [a - cx, (b - nb) - cy, h * 0.5],
            rigid,
        ),
        box_piece(
            [-cx, (b - nb) - cy - overlap, -h * 0.5],
            [(a - na) - cx, b - cy, h * 0.5],
            rigid,
        ),
    ];
    let field = Union::new(
        piece_box_field([-cx, -cy, -h * 0.5], [a - cx, (b - nb) - cy, h * 0.5]),
        piece_box_field(
            [-cx, (b - nb) - cy - overlap, -h * 0.5],
            [(a - na) - cx, b - cy, h * 0.5],
        ),
    );
    Operand {
        describe: format!(
            "l_prism a={a:.3} b={b:.3} h={h:.3} notch=({na:.3},{nb:.3}) t=({:.3},{:.3},{:.3})",
            rigid.t[0], rigid.t[1], rigid.t[2]
        ),
        mesh,
        pieces,
        field: Box::new(place_field(field, rigid)),
        field_deviation: 1.0e-4,
    }
}

/// U-shaped prism: `size = [width, depth, height]`, `cut = [leg_w,
/// floor_d]` — two legs of width `leg_w` around a top-opening channel above
/// a floor of depth `floor_d`. One watertight mesh; referee/field = union
/// of three overlapping boxes.
pub(crate) fn u_prism_operand(size: [f64; 3], cut: [f64; 2], rigid: &Rigid) -> Operand {
    let [a, b, h] = size;
    let [leg, floor] = cut;
    let (cx, cy) = (a * 0.5, b * 0.5);
    let section = vec![
        [-cx, -cy],
        [a - cx, -cy],
        [a - cx, b - cy],
        [(a - leg) - cx, b - cy],
        [(a - leg) - cx, floor - cy],
        [leg - cx, floor - cy],
        [leg - cx, b - cy],
        [-cx, b - cy],
    ];
    let mesh = extruded_polygon_mesh(&section, h, rigid);
    let boxes = [
        ([-cx, -cy, -h * 0.5], [a - cx, floor - cy, h * 0.5]),
        ([-cx, -cy, -h * 0.5], [leg - cx, b - cy, h * 0.5]),
        ([(a - leg) - cx, -cy, -h * 0.5], [a - cx, b - cy, h * 0.5]),
    ];
    let pieces = boxes
        .iter()
        .map(|(lo, hi)| box_piece(*lo, *hi, rigid))
        .collect();
    let field = Union::new(
        piece_box_field(boxes[0].0, boxes[0].1),
        Union::new(
            piece_box_field(boxes[1].0, boxes[1].1),
            piece_box_field(boxes[2].0, boxes[2].1),
        ),
    );
    Operand {
        describe: format!(
            "u_prism a={a:.3} b={b:.3} h={h:.3} cut=({leg:.3},{floor:.3}) t=({:.3},{:.3},{:.3})",
            rigid.t[0], rigid.t[1], rigid.t[2]
        ),
        mesh,
        pieces,
        field: Box::new(place_field(field, rigid)),
        field_deviation: 1.0e-4,
    }
}

/// Watertight extrusion of a CCW simple polygon along local z, transformed
/// like every other operand (build local f32, one f64 rigid apply, one
/// narrowing).
fn extruded_polygon_mesh(section: &[[f64; 2]], height: f64, rigid: &Rigid) -> Mesh {
    let mut builder = exedra_mesh::MeshBuilder::new();
    let n = section.len();
    let count = u32::try_from(n).expect("small section");
    for z in [-height * 0.5, height * 0.5] {
        for p in section {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "local build narrowing matches the primitive path"
            )]
            builder.push_vertex([p[0] as f32, p[1] as f32, z as f32]);
        }
    }
    let bottom: Vec<u32> = (0..count).rev().collect();
    builder.add_face(&bottom).expect("bottom cap");
    let top: Vec<u32> = (count..2 * count).collect();
    builder.add_face(&top).expect("top cap");
    for i in 0..count {
        let j = (i + 1) % count;
        builder
            .add_face(&[i, j, count + j, count + i])
            .expect("side wall");
    }
    let local = builder.build().expect("valid extrusion").mesh;
    transform_mesh(&local, rigid)
}

/// Exact f64 half-space planes of a local axis-aligned box `[lo, hi]`
/// mapped through the rigid transform: local plane `q . e_axis <= hi`
/// becomes world plane `p . col_axis <= col_axis . t + hi`.
fn box_piece(lo: [f64; 3], hi: [f64; 3], rigid: &Rigid) -> Vec<Plane> {
    let mut planes = Vec::with_capacity(6);
    for axis in 0..3 {
        let col = rigid.cols[axis];
        let col_dot_t = col[0] * rigid.t[0] + col[1] * rigid.t[1] + col[2] * rigid.t[2];
        planes.push(Plane {
            normal: col,
            offset: col_dot_t + hi[axis],
        });
        planes.push(Plane {
            normal: [-col[0], -col[1], -col[2]],
            offset: -col_dot_t - lo[axis],
        });
    }
    planes
}

/// Local-space box field for one convex piece of a non-convex operand.
fn piece_box_field(lo: [f64; 3], hi: [f64; 3]) -> BoxField {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "field placement narrows once; the comparison band covers it"
    )]
    let narrow = |v: f64| v as f32;
    BoxField {
        center: [
            narrow((lo[0] + hi[0]) * 0.5),
            narrow((lo[1] + hi[1]) * 0.5),
            narrow((lo[2] + hi[2]) * 0.5),
        ],
        half_extents: [
            narrow((hi[0] - lo[0]) * 0.5),
            narrow((hi[1] - lo[1]) * 0.5),
            narrow((hi[2] - lo[2]) * 0.5),
        ],
    }
}

fn place_field<F: ScalarField + 'static>(field: F, rigid: &Rigid) -> Transform3<F> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "field placement narrows once; the comparison band covers it"
    )]
    let narrow = |v: [f64; 3]| [v[0] as f32, v[1] as f32, v[2] as f32];
    let transform = RigidTransform3::new(
        narrow(rigid.t),
        narrow(rigid.cols[0]),
        narrow(rigid.cols[1]),
        narrow(rigid.cols[2]),
    )
    .expect("sampled rotations are orthonormal");
    Transform3::new(field, transform)
}

/// Clones a mesh with vertices rigid-transformed in f64 and narrowed once.
fn transform_mesh(source: &Mesh, rigid: &Rigid) -> Mesh {
    let mut mesh = source.clone();
    let vertices: Vec<exedra_mesh::VertexId> = mesh.vertices().collect();
    {
        let mut session = mesh.edit();
        for vertex in vertices {
            if let Some(p) = session.mesh().vertex_position(vertex) {
                let local = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
                let world = rigid.apply(local);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "single documented narrowing at placement"
                )]
                let narrowed = [world[0] as f32, world[1] as f32, world[2] as f32];
                let _ = set_vertex_position(&mut session, vertex, narrowed);
            }
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
    }
    mesh
}

/// Derives outward half-space planes from a convex planar-faced mesh and
/// asserts convexity (every vertex on or behind every plane).
fn convex_planes(mesh: &Mesh) -> Vec<Plane> {
    let mut planes = Vec::new();
    for face in mesh.faces() {
        let mut loop_points: Vec<[f64; 3]> = Vec::new();
        for half_edge in mesh.face_loop(face) {
            if let Some(p) = mesh
                .to_vertex(half_edge)
                .and_then(|v| mesh.vertex_position(v))
            {
                loop_points.push([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
            }
        }
        if loop_points.len() < 3 {
            continue;
        }
        // Newell normal over the loop; outward because exedra_mesh faces wind CCW
        // as seen from outside.
        let mut normal = [0.0_f64; 3];
        let mut centroid = [0.0_f64; 3];
        let n = loop_points.len();
        for i in 0..n {
            let a = loop_points[i];
            let b = loop_points[(i + 1) % n];
            normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
            normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
            normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
            centroid[0] += a[0];
            centroid[1] += a[1];
            centroid[2] += a[2];
        }
        let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        assert!(len > 1.0e-12, "operand face with degenerate normal");
        let inv = 1.0 / len;
        let normal = [normal[0] * inv, normal[1] * inv, normal[2] * inv];
        #[expect(clippy::cast_precision_loss, reason = "loop lengths are tiny")]
        let inv_n = 1.0 / n as f64;
        let centroid = [
            centroid[0] * inv_n,
            centroid[1] * inv_n,
            centroid[2] * inv_n,
        ];
        planes.push(Plane {
            normal,
            offset: normal[0] * centroid[0] + normal[1] * centroid[1] + normal[2] * centroid[2],
        });
    }

    // Convexity sanity: the referee is only sound for convex operands.
    // Tolerance scales with the mesh extent (f32 narrowing is relative).
    let mut extent = 1.0_f64;
    for vertex in mesh.vertices() {
        if let Some(p) = mesh.vertex_position(vertex) {
            for &coordinate in p {
                extent = extent.max(f64::from(coordinate).abs());
            }
        }
    }
    let tolerance = 1.0e-5 * extent;
    for vertex in mesh.vertices() {
        let Some(p) = mesh.vertex_position(vertex) else {
            continue;
        };
        let p = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
        for plane in &planes {
            let d = plane.normal[0] * p[0] + plane.normal[1] * p[1] + plane.normal[2] * p[2]
                - plane.offset;
            assert!(
                d <= tolerance,
                "operand mesh is not convex: vertex {d} outside a face plane"
            );
        }
    }
    planes
}

impl Operand {
    /// Referee pseudo-SDF: exact sign; per-piece magnitude is a 1-Lipschitz
    /// lower bound, and the union (`min` over pieces) keeps the value-space
    /// perturbation argument intact (each piece is one more min/max leaf).
    #[must_use]
    pub(crate) fn referee(&self, p: [f64; 3]) -> f64 {
        let mut value = f64::INFINITY;
        for piece in &self.pieces {
            let mut piece_value = f64::NEG_INFINITY;
            for plane in piece {
                let d = plane.normal[0] * p[0] + plane.normal[1] * p[1] + plane.normal[2] * p[2]
                    - plane.offset;
                piece_value = piece_value.max(d);
            }
            value = value.min(piece_value);
        }
        value
    }
}

/// Extracts all face triangles of a mesh as f64 corner positions,
/// dropping exactly-degenerate triangles.
#[must_use]
pub(crate) fn mesh_triangles_f64(mesh: &Mesh) -> Vec<[[f64; 3]; 3]> {
    let mut triangles = Vec::new();
    let mut buffer = Vec::new();
    for face in mesh.faces() {
        let _ = mesh.face_triangles_into(face, FaceTriangulation::Robust, &mut buffer);
        for triangle in &buffer {
            let mut corners = [[0.0_f64; 3]; 3];
            let mut live = true;
            for (slot, corner) in corners.iter_mut().zip(triangle) {
                match mesh
                    .to_vertex(*corner)
                    .and_then(|v| mesh.vertex_position(v))
                {
                    Some(p) => *slot = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])],
                    None => live = false,
                }
            }
            if live {
                triangles.push(corners);
            }
        }
    }
    triangles
}
