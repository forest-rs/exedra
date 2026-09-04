// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Human-readable mesh dump helpers for debugging tests.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;

use exedra_mesh::{ExtractParams, Mesh};

/// Renders a deterministic topology dump for quick inspection.
#[must_use]
pub fn dump_mesh_topology(mesh: &Mesh) -> String {
    let mut out = String::new();

    let vertex_count = mesh.vertices().count();
    let face_count = mesh.faces().count();
    out.push_str(&format!("vertices {vertex_count}\n"));
    out.push_str(&format!("faces {face_count}\n"));

    for vertex in mesh.vertices() {
        match mesh.vertex_out(vertex) {
            Some(edge) => out.push_str(&format!("v {} out {}\n", vertex.index(), edge.index())),
            None => out.push_str(&format!("v {} out none\n", vertex.index())),
        }
    }

    for face in mesh.faces() {
        out.push_str(&format!("f {}\n", face.index()));
        for corner in mesh.face_loop(face) {
            let from = mesh
                .from_vertex(corner)
                .expect("face loop corner must have source vertex");
            let to = mesh
                .to_vertex(corner)
                .expect("face loop corner must have destination vertex");
            let twin = mesh.twin(corner).expect("face loop corner must have twin");
            let adjacent = mesh
                .face(twin)
                .expect("face loop corner twin must have face");
            out.push_str(&format!(
                "  he {} {}->{}, twin {}, twin_face {}\n",
                corner.index(),
                from.index(),
                to.index(),
                twin.index(),
                adjacent.index()
            ));
        }
    }

    out
}

/// Renders a deterministic dump of built-in attribute layers.
#[must_use]
pub fn dump_attributes(mesh: &Mesh) -> String {
    let mut out = String::new();

    out.push_str("vertex.position\n");
    for vertex in mesh.vertices() {
        let Some(position) = mesh.vertex_position(vertex) else {
            continue;
        };
        out.push_str(&format!(
            "  v {} [{:08x} {:08x} {:08x}]\n",
            vertex.index(),
            position[0].to_bits(),
            position[1].to_bits(),
            position[2].to_bits()
        ));
    }

    out.push_str("vertex.sharpness\n");
    if let Some(layer) = mesh.attrs().sparse(exedra_mesh::attr::VERTEX_SHARPNESS) {
        for vertex in mesh.vertices() {
            if let Some(sharpness) = layer.get(vertex.as_id()) {
                out.push_str(&format!(
                    "  v {} {:08x}\n",
                    vertex.index(),
                    sharpness.to_bits()
                ));
            }
        }
    }

    out.push_str("face.region\n");
    if let Some(layer) = mesh.attrs().dense(exedra_mesh::attr::FACE_REGION) {
        for face in mesh.faces() {
            if let Some(value) = layer.get(face.as_id()) {
                out.push_str(&format!("  f {} {}\n", face.index(), value));
            }
        }
    }

    out.push_str("corner.uv\n");
    if let Some(layer) = mesh.attrs().sparse(exedra_mesh::attr::CORNER_UV) {
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                if let Some(uv) = layer.get(corner.as_id()) {
                    out.push_str(&format!(
                        "  he {} [{:08x} {:08x}]\n",
                        corner.index(),
                        uv[0].to_bits(),
                        uv[1].to_bits()
                    ));
                }
            }
        }
    }

    out.push_str("corner.normal_override\n");
    if let Some(layer) = mesh
        .attrs()
        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
    {
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                if let Some(normal) = layer.get(corner.as_id()) {
                    out.push_str(&format!(
                        "  he {} [{:08x} {:08x} {:08x}]\n",
                        corner.index(),
                        normal[0].to_bits(),
                        normal[1].to_bits(),
                        normal[2].to_bits()
                    ));
                }
            }
        }
    }

    out.push_str("edge.seam\n");
    let mut seen_edges = BTreeSet::new();
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            let Some(canonical) = mesh.canonical_edge(corner) else {
                continue;
            };
            if !seen_edges.insert(canonical) {
                continue;
            }
            let seam = mesh
                .edge_seam(corner)
                .expect("canonical edge should be live for seam query");
            out.push_str(&format!("  e {} {}\n", canonical.index(), seam));
        }
    }

    out.push_str("edge.sharpness\n");
    for canonical in seen_edges {
        let sharp = mesh
            .edge_sharpness(canonical)
            .expect("canonical edge should be live for sharpness query");
        out.push_str(&format!(
            "  e {} {:08x}\n",
            canonical.index(),
            sharp.to_bits()
        ));
    }

    out
}

/// Exports the extracted render mesh as a simple OBJ string.
#[must_use]
pub fn mesh_to_obj(mesh: &Mesh) -> String {
    let (tri, _) = mesh.to_trimesh(&ExtractParams::default());
    let mut out = String::new();

    for position in &tri.positions {
        out.push_str(&format!(
            "v {} {} {}\n",
            position[0], position[1], position[2]
        ));
    }
    for uv in &tri.uvs {
        out.push_str(&format!("vt {} {}\n", uv[0], uv[1]));
    }
    for normal in &tri.normals {
        out.push_str(&format!("vn {} {} {}\n", normal[0], normal[1], normal[2]));
    }

    for triangle in tri.indices.chunks_exact(3) {
        let a = triangle[0] + 1;
        let b = triangle[1] + 1;
        let c = triangle[2] + 1;
        out.push_str(&format!("f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}\n"));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{dump_attributes, dump_mesh_topology, mesh_to_obj};
    use crate::fixtures::quad_mesh;

    #[test]
    fn dump_helpers_emit_stable_headers() {
        let mesh = quad_mesh();
        let topology = dump_mesh_topology(&mesh);
        assert!(topology.starts_with("vertices "));
        assert!(topology.contains("faces 1\n"));

        let attrs = dump_attributes(&mesh);
        assert!(attrs.starts_with("vertex.position\n"));
        assert!(attrs.contains("vertex.sharpness\n"));
        assert!(attrs.contains("face.region\n"));
        assert!(attrs.contains("corner.normal_override\n"));
        assert!(attrs.contains("edge.sharpness\n"));

        let obj = mesh_to_obj(&mesh);
        assert!(obj.contains("\nv ") || obj.starts_with("v "));
        assert!(obj.contains("\nf ") || obj.starts_with("f ") || obj.contains("\nf "));
    }
}
