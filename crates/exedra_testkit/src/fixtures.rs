// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic mesh fixtures for tests.

use exedra::{BuildParams, Mesh, MeshBuilder};
use exedra_primitives::{
    BoxParams, CapFill, CylinderParams, QuadParams, UvSphereParams, box_primitive, cylinder, quad,
    uv_sphere,
};

/// Returns one-triangle fixture mesh.
#[must_use]
pub fn triangle_mesh() -> Mesh {
    Mesh::from_indexed_triangles(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        &[[0, 1, 2]],
        &BuildParams::default(),
    )
    .expect("triangle fixture must be valid")
}

/// Returns one-quad fixture mesh.
#[must_use]
pub fn quad_mesh() -> Mesh {
    Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        &[&[0, 1, 2, 3]],
    )
    .expect("quad fixture must be valid")
}

/// Returns the canonical unit quad fixture backed by `exedra_primitives`.
#[must_use]
pub fn unit_quad() -> Mesh {
    quad(&QuadParams::default()).mesh
}

/// Returns the canonical centered unit box fixture backed by `exedra_primitives`.
#[must_use]
pub fn unit_box() -> Mesh {
    box_primitive(&BoxParams::default()).mesh
}

/// Returns the canonical capped eight-segment cylinder fixture.
#[must_use]
pub fn capped_cylinder_8() -> Mesh {
    cylinder(&CylinderParams {
        segments: 8,
        cap_fill: CapFill::Ngon,
        ..CylinderParams::default()
    })
    .mesh
}

/// Returns the canonical low-resolution UV sphere fixture.
#[must_use]
pub fn uv_sphere_8() -> Mesh {
    uv_sphere(&UvSphereParams {
        lat_segments: 4,
        lon_segments: 8,
        ..UvSphereParams::default()
    })
    .mesh
}

/// Returns the canonical sphere fixture.
#[must_use]
pub fn sphere_mesh() -> Mesh {
    uv_sphere_8()
}

/// Returns one-tetrahedron fixture mesh.
#[must_use]
pub fn tetrahedron_mesh() -> Mesh {
    Mesh::from_indexed_triangles(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.5, 0.866_025_4, 0.0],
            [0.5, 0.288_675_13, 0.816_496_6],
        ],
        &[[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
        &BuildParams::default(),
    )
    .expect("tetrahedron fixture must be valid")
}

/// Returns a planar `width x height` quad grid fixture.
///
/// Cells are emitted in row-major order with deterministic winding.
#[must_use]
pub fn grid_mesh(width: u32, height: u32) -> Mesh {
    assert!(width > 0, "grid width must be > 0");
    assert!(height > 0, "grid height must be > 0");

    let mut builder = MeshBuilder::new();
    let columns = width + 1;
    let rows = height + 1;

    for y in 0..rows {
        for x in 0..columns {
            let _ = builder.push_vertex([x as f32, y as f32, 0.0]);
        }
    }

    let index = |x: u32, y: u32| y * columns + x;
    for y in 0..height {
        for x in 0..width {
            let face = [
                index(x, y),
                index(x + 1, y),
                index(x + 1, y + 1),
                index(x, y + 1),
            ];
            builder
                .add_face(&face)
                .expect("grid face must be valid and manifold");
        }
    }

    builder
        .build()
        .expect("grid fixture build must succeed")
        .mesh
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::{
        capped_cylinder_8, grid_mesh, quad_mesh, sphere_mesh, tetrahedron_mesh, triangle_mesh,
        unit_box, unit_quad,
    };

    #[test]
    fn core_fixtures_validate_deep() {
        let fixtures = [
            triangle_mesh(),
            quad_mesh(),
            unit_quad(),
            unit_box(),
            capped_cylinder_8(),
            sphere_mesh(),
            tetrahedron_mesh(),
            grid_mesh(2, 3),
        ];
        for mesh in fixtures {
            assert!(mesh.validate_fast().is_empty());
            assert!(mesh.validate_deep().is_empty());
        }
    }

    #[test]
    fn grid_fixture_counts_are_deterministic() {
        let mesh = grid_mesh(3, 2);
        let vertices = mesh.vertices().collect::<Vec<_>>();
        let faces = mesh.faces().collect::<Vec<_>>();
        assert_eq!(vertices.len(), 12);
        assert_eq!(faces.len(), 6);
    }
}
