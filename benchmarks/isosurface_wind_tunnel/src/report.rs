// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic signatures, topology audits, and artifact serialization.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use exedra::{ExtractParams, Mesh, VertexId, attr};
use exedra_math::{dot, sub};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TopologyReport {
    pub(crate) validation_errors: usize,
    pub(crate) boundary_loops: usize,
    pub(crate) incidence_violations: usize,
    pub(crate) degenerate_triangles: usize,
    pub(crate) isolated_vertices: usize,
}

impl TopologyReport {
    pub(crate) fn is_closed_clean(&self) -> bool {
        self.validation_errors == 0
            && self.boundary_loops == 0
            && self.incidence_violations == 0
            && self.degenerate_triangles == 0
            && self.isolated_vertices == 0
    }
}

pub(crate) fn extraction_signature(mesh: &Mesh) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for vertex in mesh.vertices() {
        hash = fnv_bytes(hash, &vertex.index().to_le_bytes());
        for component in mesh.vertex_position(vertex).expect("live vertex position") {
            hash = fnv_bytes(hash, &component.to_bits().to_le_bytes());
        }
    }

    let regions = mesh.attrs().dense(attr::FACE_REGION);
    let sharpness = mesh.attrs().sparse(attr::EDGE_SHARPNESS);
    let seams = mesh.attrs().sparse(attr::EDGE_SEAM);
    let normals = mesh.attrs().sparse(attr::CORNER_NORMAL_OVERRIDE);
    for face in mesh.faces() {
        hash = fnv_bytes(hash, &face.index().to_le_bytes());
        hash = fnv_bytes(
            hash,
            &regions
                .and_then(|layer| layer.get(face.as_id()))
                .copied()
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).expect("face-loop vertex");
            hash = fnv_bytes(hash, &vertex.index().to_le_bytes());
            hash = hash_optional_f32(
                hash,
                sharpness
                    .and_then(|layer| layer.get(corner.as_id()))
                    .copied(),
            );
            hash = fnv_bytes(
                hash,
                &[u8::from(
                    seams
                        .and_then(|layer| layer.get(corner.as_id()))
                        .copied()
                        .unwrap_or(false),
                )],
            );
            if let Some(normal) = normals.and_then(|layer| layer.get(corner.as_id())).copied() {
                hash = fnv_bytes(hash, &[1]);
                for component in normal {
                    hash = fnv_bytes(hash, &component.to_bits().to_le_bytes());
                }
            } else {
                hash = fnv_bytes(hash, &[0]);
            }
        }
        hash = fnv_bytes(hash, &[0xff]);
    }

    let (triangles, _) = mesh.to_trimesh(&ExtractParams::default());
    for position in triangles.positions {
        for component in position {
            hash = fnv_bytes(hash, &component.to_bits().to_le_bytes());
        }
    }
    for index in triangles.indices {
        hash = fnv_bytes(hash, &index.to_le_bytes());
    }
    hash
}

pub(crate) fn region_histogram(mesh: &Mesh) -> Vec<(u32, usize)> {
    let regions = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .expect("FACE_REGION layer");
    let mut histogram = BTreeMap::new();
    for face in mesh.faces() {
        let region = regions
            .get(face.as_id())
            .copied()
            .expect("every face has region");
        *histogram.entry(region).or_insert(0) += 1;
    }
    histogram.into_iter().collect()
}

pub(crate) fn topology(mesh: &Mesh) -> TopologyReport {
    let validation_errors = mesh.validate_deep().len();
    let boundary_loops = mesh
        .boundary_loops()
        .map_or(usize::MAX, |loops| loops.len());
    let mut incidence = BTreeMap::<(VertexId, VertexId), usize>::new();
    let mut referenced = BTreeSet::new();
    let mut degenerate_triangles = 0;
    for face in mesh.faces() {
        let vertices = mesh
            .face_loop(face)
            .map(|corner| mesh.to_vertex(corner).expect("face-loop vertex"))
            .collect::<Vec<_>>();
        referenced.extend(vertices.iter().copied());
        for index in 0..vertices.len() {
            let first = vertices[index];
            let second = vertices[(index + 1) % vertices.len()];
            let (a, b) = if first < second {
                (first, second)
            } else {
                (second, first)
            };
            *incidence.entry((a, b)).or_insert(0) += 1;
        }
        let nondegenerate = vertices.len() == 3
            && triangle_is_nondegenerate(core::array::from_fn(|corner| {
                *mesh
                    .vertex_position(vertices[corner])
                    .expect("face vertex position")
            }));
        if !nondegenerate {
            degenerate_triangles += 1;
        }
    }
    TopologyReport {
        validation_errors,
        boundary_loops,
        incidence_violations: incidence.values().filter(|&&count| count != 2).count(),
        degenerate_triangles,
        isolated_vertices: mesh.vertices().count() - referenced.len(),
    }
}

pub(crate) fn grouped_obj(mesh: &Mesh, title: &str) -> String {
    let mut output = format!("# {title}\n# deterministic; faces grouped by primitive identity\n");
    let mut obj_indices = BTreeMap::<VertexId, usize>::new();
    for (index, vertex) in mesh.vertices().enumerate() {
        let position = mesh.vertex_position(vertex).expect("live vertex position");
        writeln!(
            output,
            "v {:.9} {:.9} {:.9}",
            position[0], position[1], position[2]
        )
        .expect("writing to String cannot fail");
        obj_indices.insert(vertex, index + 1);
    }
    let regions = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .expect("FACE_REGION layer");
    let mut groups = BTreeMap::<u32, Vec<Vec<usize>>>::new();
    for face in mesh.faces() {
        let region = regions.get(face.as_id()).copied().unwrap_or(0);
        let vertices = mesh
            .face_loop(face)
            .map(|corner| {
                let vertex = mesh.from_vertex(corner).expect("face-loop source vertex");
                obj_indices[&vertex]
            })
            .collect();
        groups.entry(region).or_default().push(vertices);
    }
    for (region, faces) in groups {
        writeln!(output, "g primitive_{region}").expect("writing to String cannot fail");
        for face in faces {
            output.push('f');
            for index in face {
                write!(output, " {index}").expect("writing to String cannot fail");
            }
            output.push('\n');
        }
    }
    output
}

fn triangle_is_nondegenerate(points: [[f32; 3]; 3]) -> bool {
    let points = points.map(|point| point.map(f64::from));
    let ab = sub(points[1], points[0]);
    let ac = sub(points[2], points[0]);
    let bc = sub(points[2], points[1]);
    let cross = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let area_squared = dot(cross, cross);
    let longest_edge_squared = dot(ab, ab).max(dot(ac, ac)).max(dot(bc, bc));
    let relative_epsilon = 16.0 * f64::from(f32::EPSILON);
    let minimum_area_squared =
        relative_epsilon * relative_epsilon * longest_edge_squared * longest_edge_squared;
    area_squared.is_finite()
        && longest_edge_squared.is_finite()
        && longest_edge_squared > 0.0
        && area_squared > minimum_area_squared
}

fn hash_optional_f32(hash: u64, value: Option<f32>) -> u64 {
    match value {
        Some(value) => fnv_bytes(fnv_bytes(hash, &[1]), &value.to_bits().to_le_bytes()),
        None => fnv_bytes(hash, &[0]),
    }
}

fn fnv_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use exedra::MeshBuilder;

    use super::{extraction_signature, topology};

    #[test]
    fn signature_and_topology_are_deterministic() {
        let mut builder = MeshBuilder::new();
        let vertices = [
            builder.push_vertex([0.0, 0.0, 0.0]),
            builder.push_vertex([1.0, 0.0, 0.0]),
            builder.push_vertex([0.0, 1.0, 0.0]),
        ];
        builder.add_face(&vertices).expect("triangle");
        let mesh = builder.build().expect("mesh").mesh;
        assert_eq!(extraction_signature(&mesh), extraction_signature(&mesh));
        let report = topology(&mesh);
        assert_eq!(report.validation_errors, 0);
        assert_eq!(report.boundary_loops, 1);
        assert_eq!(report.incidence_violations, 3);
        assert_eq!(report.degenerate_triangles, 0);
        assert_eq!(report.isolated_vertices, 0);
    }
}
