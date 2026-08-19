// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Oracle operands: one solid in three witness forms.
//!
//! Every operand is a convex, planar-faced polyhedron carried as:
//! - an exedra [`Mesh`] (the mesh-boolean witness input),
//! - f64 half-space planes derived from that mesh (the closed-form referee —
//!   by construction it describes *exactly* the solid the mesh witness sees),
//! - an `exedra_isosurface` scalar field for the analytic solid the mesh
//!   approximates, plus a Hausdorff bound between the two
//!   (`field_deviation`), which widens the field comparison band.

use exedra::{FaceTriangulation, Mesh, op::set_vertex_position};
use exedra_isosurface::{
    ScalarField,
    analytic::{BoxField, CylinderField},
    transform::{RigidTransform3, Transform3},
};
use exedra_primitives::{BoxParams, CylinderParams, box_primitive, cylinder};

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
pub(crate) struct Operand {
    /// The transformed mesh consumed by the boolean pipeline.
    pub(crate) mesh: Mesh,
    /// Half-space planes describing exactly the mesh solid (convex).
    pub(crate) planes: Vec<Plane>,
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
    let rigid = random_rigid(rng);
    if rng.index(2) == 0 {
        let size = [
            rng.range_f64(0.5, 1.8),
            rng.range_f64(0.5, 1.8),
            rng.range_f64(0.5, 1.8),
        ];
        box_operand(size, &rigid)
    } else {
        let radius = rng.range_f64(0.3, 0.9);
        let height = rng.range_f64(0.5, 1.8);
        let segments = [8_u32, 12, 16, 24][rng.index(4)];
        prism_operand(radius, height, segments, &rigid)
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
    let planes = convex_planes(&mesh);
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
        planes,
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
    let planes = convex_planes(&mesh);
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
        planes,
        field: Box::new(place_field(field, rigid)),
        field_deviation: sagitta + 1.0e-4,
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
    let vertices: Vec<exedra::VertexId> = mesh.vertices().collect();
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
        // Newell normal over the loop; outward because exedra faces wind CCW
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
    for vertex in mesh.vertices() {
        let Some(p) = mesh.vertex_position(vertex) else {
            continue;
        };
        let p = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
        for plane in &planes {
            let d = plane.normal[0] * p[0] + plane.normal[1] * p[1] + plane.normal[2] * p[2]
                - plane.offset;
            assert!(
                d <= 1.0e-5,
                "operand mesh is not convex: vertex {d} outside a face plane"
            );
        }
    }
    planes
}

impl Operand {
    /// Referee pseudo-SDF: exact sign; magnitude is a lower bound of the
    /// distance to the operand boundary (exact inside a convex solid).
    #[must_use]
    pub(crate) fn referee(&self, p: [f64; 3]) -> f64 {
        let mut value = f64::NEG_INFINITY;
        for plane in &self.planes {
            let d = plane.normal[0] * p[0] + plane.normal[1] * p[1] + plane.normal[2] * p[2]
                - plane.offset;
            value = value.max(d);
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
