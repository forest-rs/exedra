// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Composition probes: model varied things through the public constructive
//! surface, check every invariant the evaluator promises, and export GLB for
//! visual review.
//!
//! Usage: `constructive_probe <out_dir>` writes `<name>.glb` per scenario,
//! `all.glb` with every scenario laid out on a grid, and `report.txt`.

use std::fmt::Write as _;
use std::path::Path;

use exedra_assembly::{Assembly, PartCompiler, flatten};
use exedra_constructive::builders;
use exedra_constructive::cache::EvalCache;
use exedra_constructive::evaluate::{Evaluation, Fidelity, evaluate, evaluate_with_cache};
use exedra_constructive::ir::{
    CapMode, CsgOp, FramePolicy, LoftPolicy, NodeId, NodeKind, Path3, Placement3, Plane3,
    PrimitiveSpec, ProfileId, Recipe, RecipeBuilder,
};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};
use exedra_constructive::tessellate::EvalPolicy;
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use exedra_mesh::{FaceTriangulation, Mesh};

const PI: f64 = std::f64::consts::PI;

/// The probe policy: a coarser chord tolerance than the default, since the
/// revolved spheres would otherwise dominate the run at tens of thousands
/// of faces each without exercising anything different.
fn probe_policy() -> EvalPolicy {
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.1;
    policy
}

/// One probe: a recipe plus what we expect of it.
struct Probe {
    name: &'static str,
    /// What the probe is stressing, for the report.
    intent: &'static str,
    recipe: Recipe,
    /// Analytic volume when one exists; checked within `volume_tol`.
    expected_volume: Option<f64>,
    volume_tol: f64,
    /// Expected Euler characteristic of the union of all bodies, if known.
    expected_euler: Option<i64>,
    /// Expected number of emitted bodies.
    expected_bodies: Option<usize>,
}

fn probes() -> Vec<Probe> {
    vec![
        mortise_and_tenon(),
        bracket_with_holes(),
        sphere_union(),
        sphere_minus_cylinder(),
        two_cylinder_intersection(),
        two_cylinder_unequal(),
        two_cylinder_skewed(),
        three_cylinder_intersection(),
        three_cylinder_chained(),
        bent_pipe_sweep(),
        tapered_loft(),
        loft_between_circles(),
        stretched_frame(),
        attributed_import_mirrored_instances(),
        thin_wall(),
        near_touching_and_flush(),
        hole_grid(),
        rotated_boxes(),
        nested_instances(),
        grid_shell_cut(),
        revolve_notched(),
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn add_box(b: &mut RecipeBuilder, size: [f64; 3], origin: [f64; 3]) -> NodeId {
    b.add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box { size },
        placement: Placement3::translate(origin[0], origin[1], origin[2]),
    })
    .expect("valid box")
}

fn add_cylinder(b: &mut RecipeBuilder, radius: f64, height: f64, placement: Placement3) -> NodeId {
    b.add(NodeKind::Primitive {
        spec: PrimitiveSpec::Cylinder {
            radius,
            height,
            segments: 32,
        },
        placement,
    })
    .expect("valid cylinder")
}

fn add_extrude(
    b: &mut RecipeBuilder,
    profile: ProfileId,
    placement: Placement3,
    height: f64,
) -> NodeId {
    b.add(NodeKind::Extrude {
        profile,
        placement,
        height,
        caps: CapMode::Both,
    })
    .expect("valid extrude")
}

fn csg(b: &mut RecipeBuilder, op: CsgOp, operands: Vec<NodeId>) -> NodeId {
    b.add(NodeKind::Csg { op, operands }).expect("valid csg")
}

fn plane(normal: [f64; 3], distance: f64) -> Plane3 {
    Plane3 { normal, distance }
}

/// Rotation about +Y by `radians`, then translation.
fn rotate_y_then_translate(radians: f64, t: [f64; 3]) -> Placement3 {
    let (s, c) = radians.sin_cos();
    Placement3 {
        rows: [[c, 0.0, s, t[0]], [0.0, 1.0, 0.0, t[1]], [-s, 0.0, c, t[2]]],
    }
}

/// A semicircle profile of radius `r` bowing toward +x, closed along the
/// axis: revolving it sweeps a sphere.
fn semicircle(r: f64, bulge: f64) -> Profile2 {
    Profile2::simple(
        Loop2::new(vec![Seg2::arc((0.0, r), bulge), Seg2::line((0.0, -r))])
            .expect("semicircle loop"),
    )
    .expect("semicircle profile")
}

fn sphere(b: &mut RecipeBuilder, r: f64, placement: Placement3) -> NodeId {
    let profile = b.add_profile(semicircle(r, 1.0));
    b.add(NodeKind::Revolve {
        profile,
        placement,
        sweep: 2.0 * PI,
        caps: CapMode::Both,
    })
    .expect("valid revolve")
}

// ---------------------------------------------------------------------------
// Probes
// ---------------------------------------------------------------------------

/// A 100 x 100 beam with a through mortise, and a second beam whose end is
/// cut to a tenon by two side cuts and a shoulder cut, placed so the tenon
/// sits in the mortise. Chained differences on both parts.
fn mortise_and_tenon() -> Probe {
    let mut b = RecipeBuilder::new();
    let post = add_box(&mut b, [100.0, 100.0, 600.0], [0.0, 0.0, 0.0]);
    let mortise = add_box(&mut b, [40.0, 120.0, 80.0], [30.0, -10.0, 300.0]);
    let post_cut = csg(&mut b, CsgOp::Difference, vec![post, mortise]);

    let rail = add_box(&mut b, [400.0, 100.0, 80.0], [0.0, 0.0, 0.0]);
    let side_a = add_box(&mut b, [120.0, 40.0, 100.0], [-10.0, -10.0, -10.0]);
    let side_b = add_box(&mut b, [120.0, 40.0, 100.0], [-10.0, 70.0, -10.0]);
    let tenon_rail = csg(&mut b, CsgOp::Difference, vec![rail, side_a]);
    let tenon_rail = csg(&mut b, CsgOp::Difference, vec![tenon_rail, side_b]);
    // Rail runs along +X from the post; its tenon (x in [0, 100]) enters the
    // mortise. Place so tenon occupies post x in [30, 70]... approximately.
    let placed_rail = b
        .add(NodeKind::Transform {
            child: tenon_rail,
            xf: Placement3::translate(30.0, 0.0, 300.0),
        })
        .expect("valid transform");
    let root = b
        .add(NodeKind::Group {
            children: vec![post_cut, placed_rail],
        })
        .expect("valid group");
    let post_volume = 100.0 * 100.0 * 600.0 - 40.0 * 100.0 * 80.0;
    let rail_volume = 400.0 * 100.0 * 80.0 - 2.0 * (110.0 * 30.0 * 80.0);
    Probe {
        name: "mortise_and_tenon",
        intent: "chained differences on two parts, grouped",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(post_volume + rail_volume),
        volume_tol: 1e-3,
        // The mortise goes right through the post (genus 1), the rail is a ball.
        expected_euler: Some(2),
        expected_bodies: Some(2),
    }
}

/// An L bracket with three drilled holes (n-ary difference), mirrored.
fn bracket_with_holes() -> Probe {
    let mut b = RecipeBuilder::new();
    let l = b.add_profile(builders::l_profile(200.0, 200.0, 40.0, 40.0).expect("L"));
    let bracket = add_extrude(&mut b, l, Placement3::IDENTITY, 60.0);
    let hole = b.add_profile(builders::circle(8.0).expect("circle"));
    let holes: Vec<NodeId> = [(20.0, 60.0), (20.0, 120.0), (20.0, 170.0)]
        .into_iter()
        .map(|(x, y)| add_extrude(&mut b, hole, Placement3::translate(x, y, -10.0), 80.0))
        .collect();
    let mut operands = vec![bracket];
    operands.extend(holes);
    let drilled = csg(&mut b, CsgOp::Difference, operands);
    let mirrored = b
        .add(NodeKind::Mirror {
            child: drilled,
            plane: plane([1.0, 0.0, 0.0], -20.0),
        })
        .expect("valid mirror");
    let root = b
        .add(NodeKind::Group {
            children: vec![drilled, mirrored],
        })
        .expect("valid group");
    // `l_profile(w, h, notch_w, notch_h)` is the w x h rectangle minus one
    // notch_w x notch_h corner.
    let l_area = 200.0 * 200.0 - 40.0 * 40.0;
    let one = l_area * 60.0 - 3.0 * PI * 8.0 * 8.0 * 60.0;
    Probe {
        name: "bracket_with_holes",
        intent: "n-ary difference with 3 cutters, then Mirror of a CSG result",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(2.0 * one),
        volume_tol: 0.01,
        expected_euler: Some(2 * (2 - 2 * 3)),
        expected_bodies: Some(2),
    }
}

/// Two revolved spheres overlapping: curved on curved union.
fn sphere_union() -> Probe {
    let mut b = RecipeBuilder::new();
    let a = sphere(&mut b, 50.0, Placement3::IDENTITY);
    let c = sphere(&mut b, 50.0, Placement3::translate(60.0, 0.0, 0.0));
    let root = csg(&mut b, CsgOp::Union, vec![a, c]);
    // Union volume = 2V - lens; lens of two equal spheres at distance d.
    let (r, d) = (50.0_f64, 60.0_f64);
    let lens = PI * (4.0 * r + d) * (2.0 * r - d).powi(2) / 12.0;
    Probe {
        name: "sphere_union",
        intent: "curved-on-curved union of two revolved spheres",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(2.0 * (4.0 / 3.0) * PI * r.powi(3) - lens),
        volume_tol: 0.03,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// A sphere with a cylinder bored through it.
fn sphere_minus_cylinder() -> Probe {
    let mut b = RecipeBuilder::new();
    let s = sphere(&mut b, 50.0, Placement3::IDENTITY);
    let bore = add_cylinder(&mut b, 20.0, 140.0, Placement3::translate(0.0, 0.0, -70.0));
    let root = csg(&mut b, CsgOp::Difference, vec![s, bore]);
    // Sphere minus a bore of radius a: (4/3) pi (R^2 - a^2)^(3/2).
    let (r, a) = (50.0_f64, 20.0_f64);
    Probe {
        name: "sphere_minus_cylinder",
        intent: "revolved sphere minus cylinder primitive (bead)",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some((4.0 / 3.0) * PI * (r * r - a * a).powf(1.5)),
        volume_tol: 0.03,
        expected_euler: Some(0),
        expected_bodies: Some(1),
    }
}

/// Three mutually perpendicular cylinders intersected: the tricylinder.
fn three_cylinder_intersection() -> Probe {
    let mut b = RecipeBuilder::new();
    let r = 40.0;
    let z = add_cylinder(&mut b, r, 200.0, Placement3::translate(0.0, 0.0, -100.0));
    let x = add_cylinder(
        &mut b,
        r,
        200.0,
        rotate_y_then_translate(PI / 2.0, [-100.0, 0.0, 0.0]),
    );
    let y = add_cylinder(
        &mut b,
        r,
        200.0,
        Placement3::rotate_x_then_translate(-PI / 2.0, 0.0, -100.0, 0.0),
    );
    let root = csg(&mut b, CsgOp::Intersection, vec![z, x, y]);
    Probe {
        name: "three_cylinder_intersection",
        intent: "n-ary intersection of three rotated cylinders (tricylinder)",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(8.0 * (2.0 - 2.0_f64.sqrt()) * r * r * r),
        volume_tol: 0.03,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// Two perpendicular cylinders intersected: the Steinmetz solid.
fn two_cylinder_intersection() -> Probe {
    let mut b = RecipeBuilder::new();
    let r = 40.0;
    let z = add_cylinder(&mut b, r, 200.0, Placement3::translate(0.0, 0.0, -100.0));
    let x = add_cylinder(
        &mut b,
        r,
        200.0,
        rotate_y_then_translate(PI / 2.0, [-100.0, 0.0, 0.0]),
    );
    let root = csg(&mut b, CsgOp::Intersection, vec![z, x]);
    Probe {
        name: "two_cylinder_intersection",
        intent: "intersection of two perpendicular cylinders (Steinmetz)",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(16.0 / 3.0 * r * r * r),
        volume_tol: 0.03,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// Two perpendicular cylinders of different radius: the intersection curve
/// no longer passes through the other cylinder's edges.
fn two_cylinder_unequal() -> Probe {
    let mut b = RecipeBuilder::new();
    let z = add_cylinder(&mut b, 40.0, 200.0, Placement3::translate(0.0, 0.0, -100.0));
    let x = add_cylinder(
        &mut b,
        30.0,
        200.0,
        rotate_y_then_translate(PI / 2.0, [-100.0, 0.0, 0.0]),
    );
    let root = csg(&mut b, CsgOp::Intersection, vec![z, x]);
    Probe {
        name: "two_cylinder_unequal",
        intent: "perpendicular cylinders, radii 40 and 30",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: None,
        volume_tol: 0.0,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// Equal radii, but the second cylinder rotated 5 degrees off perpendicular
/// and shifted a little, so no edge of one lies on a face of the other.
fn two_cylinder_skewed() -> Probe {
    let mut b = RecipeBuilder::new();
    let r = 40.0;
    let z = add_cylinder(&mut b, r, 200.0, Placement3::translate(0.0, 0.0, -100.0));
    let x = b
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Cylinder {
                radius: r,
                height: 200.0,
                segments: 32,
            },
            placement: Placement3::euler_extrinsic_xyz_then_translate(
                0.0,
                PI / 2.0 + 5.0_f64.to_radians(),
                0.2,
                [-100.0, 3.0, 2.0],
            ),
        })
        .expect("valid cylinder");
    let root = csg(&mut b, CsgOp::Intersection, vec![z, x]);
    Probe {
        name: "two_cylinder_skewed",
        intent: "equal radii, second cylinder skewed and shifted",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: None,
        volume_tol: 0.0,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// The tricylinder again, but as two chained binary intersections.
fn three_cylinder_chained() -> Probe {
    let mut b = RecipeBuilder::new();
    let r = 40.0;
    let z = add_cylinder(&mut b, r, 200.0, Placement3::translate(0.0, 0.0, -100.0));
    let x = add_cylinder(
        &mut b,
        r,
        200.0,
        rotate_y_then_translate(PI / 2.0, [-100.0, 0.0, 0.0]),
    );
    let y = add_cylinder(
        &mut b,
        r,
        200.0,
        Placement3::rotate_x_then_translate(-PI / 2.0, 0.0, -100.0, 0.0),
    );
    let zx = csg(&mut b, CsgOp::Intersection, vec![z, x]);
    let root = csg(&mut b, CsgOp::Intersection, vec![zx, y]);
    Probe {
        name: "three_cylinder_chained",
        intent: "tricylinder as chained binary intersections",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(8.0 * (2.0 - 2.0_f64.sqrt()) * r * r * r),
        volume_tol: 0.03,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// A ring profile swept along a bent polyline: a pipe elbow.
fn bent_pipe_sweep() -> Probe {
    let mut b = RecipeBuilder::new();
    let ring = b.add_profile(builders::ring(20.0, 14.0).expect("ring"));
    let root = b
        .add(NodeKind::Sweep {
            profile: ring,
            path: Path3::Polyline {
                points: vec![
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 100.0],
                    [80.0, 0.0, 160.0],
                    [200.0, 40.0, 160.0],
                ],
                frame: FramePolicy::RotationMinimizing,
            },
            caps: CapMode::Both,
        })
        .expect("valid sweep");
    Probe {
        name: "bent_pipe_sweep",
        intent: "hollow ring profile swept along a 3-segment polyline",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: None,
        volume_tol: 0.0,
        expected_euler: Some(0),
        expected_bodies: Some(1),
    }
}

/// A rectangle lofted to a smaller, raised rectangle: a truncated pyramid.
fn tapered_loft() -> Probe {
    let mut b = RecipeBuilder::new();
    let base = b.add_profile(builders::rect(200.0, 120.0).expect("rect"));
    let top = b.add_profile(builders::rect(100.0, 60.0).expect("rect"));
    let root = b
        .add(NodeKind::Loft {
            sections: vec![
                (Placement3::IDENTITY, base),
                (Placement3::translate(50.0, 30.0, 150.0), top),
            ],
            policy: LoftPolicy::Ruled,
            caps: CapMode::Both,
        })
        .expect("valid loft");
    // Frustum: h/3 (A1 + A2 + sqrt(A1 A2)).
    let (a1, a2) = (200.0 * 120.0, 100.0 * 60.0_f64);
    Probe {
        name: "tapered_loft",
        intent: "ruled loft between two rectangles (frustum)",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(150.0 / 3.0 * (a1 + a2 + (a1 * a2).sqrt())),
        volume_tol: 1e-6,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// Two circles of different radius: the loft discretizes both with the
/// larger edge count so the rings correspond.
fn loft_between_circles() -> Probe {
    let mut b = RecipeBuilder::new();
    let big = b.add_profile(builders::circle(60.0).expect("circle"));
    let small = b.add_profile(builders::circle(25.0).expect("circle"));
    let root = b
        .add(NodeKind::Loft {
            sections: vec![
                (Placement3::IDENTITY, big),
                (Placement3::translate(0.0, 0.0, 120.0), small),
            ],
            policy: LoftPolicy::Ruled,
            caps: CapMode::Both,
        })
        .expect("valid loft");
    let (r1, r2, h) = (60.0_f64, 25.0_f64, 120.0);
    Probe {
        name: "loft_between_circles",
        intent: "loft between circles of different radius (cone frustum)",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(PI * h / 3.0 * (r1 * r1 + r1 * r2 + r2 * r2)),
        volume_tol: 0.03,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// A drilled frame member stretched along its length.
fn stretched_frame() -> Probe {
    let mut b = RecipeBuilder::new();
    let member = add_box(&mut b, [300.0, 40.0, 40.0], [0.0, 0.0, 0.0]);
    let hole = add_cylinder(&mut b, 6.0, 60.0, Placement3::translate(40.0, 20.0, -10.0));
    let drilled = csg(&mut b, CsgOp::Difference, vec![member, hole]);
    let root = b
        .add(NodeKind::Stretch {
            child: drilled,
            plane: plane([1.0, 0.0, 0.0], 150.0),
            length: 200.0,
        })
        .expect("valid stretch");
    Probe {
        name: "stretched_frame",
        intent: "Stretch over a CSG result (mesh path), hole must not distort",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(500.0 * 40.0 * 40.0 - PI * 36.0 * 40.0),
        volume_tol: 0.01,
        expected_euler: Some(0),
        expected_bodies: Some(1),
    }
}

/// An attributed imported box (UVs, normals, regions, seams) placed under a
/// rotation, instanced, and mirrored.
fn attributed_import_mirrored_instances() -> Probe {
    let mut mesh = Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [80.0, 0.0, 0.0],
            [80.0, 50.0, 0.0],
            [0.0, 50.0, 0.0],
            [0.0, 0.0, 30.0],
            [80.0, 0.0, 30.0],
            [80.0, 50.0, 30.0],
            [0.0, 50.0, 30.0],
        ],
        &[
            &[0, 3, 2, 1],
            &[4, 5, 6, 7],
            &[0, 1, 5, 4],
            &[1, 2, 6, 5],
            &[2, 3, 7, 6],
            &[3, 0, 4, 7],
        ],
    )
    .expect("box");
    {
        let faces: Vec<_> = mesh.faces().collect();
        let mut edit = mesh.edit();
        for (i, face) in faces.iter().enumerate() {
            let region = u32::try_from(i).expect("few faces");
            exedra_mesh::op::set_face_region(&mut edit, *face, region).expect("live");
            let corners: Vec<_> = edit.mesh().face_loop(*face).collect();
            for (j, corner) in corners.iter().enumerate() {
                let uv = [j as f32 * 0.25, i as f32 * 0.1];
                exedra_mesh::op::set_corner_uv(&mut edit, *corner, uv).expect("live");
                exedra_mesh::op::set_corner_normal_override(
                    &mut edit,
                    *corner,
                    Some([0.0, 0.0, 1.0]),
                )
                .expect("live");
            }
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            edit.finish();
        }
    }
    let mut b = RecipeBuilder::new();
    let import = b.add_import(mesh).expect("valid import");
    let placed = b
        .add(NodeKind::MeshImport {
            import,
            placement: Placement3::rotate_x_then_translate(PI / 6.0, 0.0, 0.0, 0.0),
        })
        .expect("valid import node");
    let inst = b
        .add(NodeKind::Instance {
            of: placed,
            placement: Placement3::rotate_z_then_translate(PI / 3.0, 150.0, 0.0, 0.0),
        })
        .expect("valid instance");
    let mirrored = b
        .add(NodeKind::Mirror {
            child: inst,
            plane: plane([0.0, 1.0, 0.0], -40.0),
        })
        .expect("valid mirror");
    let root = b
        .add(NodeKind::Group {
            children: vec![placed, inst, mirrored],
        })
        .expect("valid group");
    Probe {
        name: "attributed_import_mirrored_instances",
        intent: "attributed import under rotation, instance, and mirror",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(3.0 * 80.0 * 50.0 * 30.0),
        volume_tol: 1e-4,
        expected_euler: Some(6),
        expected_bodies: Some(3),
    }
}

/// A box hollowed to a 0.5 mm wall at a 100 mm scale.
fn thin_wall() -> Probe {
    let mut b = RecipeBuilder::new();
    let outer = add_box(&mut b, [100.0, 100.0, 100.0], [0.0, 0.0, 0.0]);
    let inner = add_box(&mut b, [99.0, 99.0, 120.0], [0.5, 0.5, 0.5]);
    let root = csg(&mut b, CsgOp::Difference, vec![outer, inner]);
    Probe {
        name: "thin_wall",
        intent: "open-topped box with 0.5 mm walls at 100 mm scale",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(100.0_f64.powi(3) - 99.0 * 99.0 * 99.5),
        volume_tol: 1e-6,
        expected_euler: Some(2),
        expected_bodies: Some(1),
    }
}

/// Three-body union: two boxes flush along a shared face, and a third
/// separated from them by a hair.
fn near_touching_and_flush() -> Probe {
    let mut b = RecipeBuilder::new();
    let a = add_box(&mut b, [50.0, 50.0, 50.0], [0.0, 0.0, 0.0]);
    let flush = add_box(&mut b, [50.0, 50.0, 50.0], [50.0, 0.0, 0.0]);
    let near = add_box(&mut b, [50.0, 50.0, 50.0], [100.001, 0.0, 0.0]);
    let root = csg(&mut b, CsgOp::Union, vec![a, flush, near]);
    Probe {
        name: "near_touching_and_flush",
        intent: "union of flush (coplanar contact) and near-touching boxes",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(3.0 * 50.0_f64.powi(3)),
        volume_tol: 1e-6,
        expected_euler: None,
        expected_bodies: Some(1),
    }
}

/// A plate with a 6 x 6 grid of drilled holes as one n-ary difference.
fn hole_grid() -> Probe {
    let mut b = RecipeBuilder::new();
    let plate = add_box(&mut b, [300.0, 300.0, 12.0], [0.0, 0.0, 0.0]);
    let mut operands = vec![plate];
    for i in 0..6 {
        for j in 0..6 {
            let x = 25.0 + 50.0 * f64::from(i);
            let y = 25.0 + 50.0 * f64::from(j);
            operands.push(add_cylinder(
                &mut b,
                10.0,
                30.0,
                Placement3::translate(x, y, -10.0),
            ));
        }
    }
    let root = csg(&mut b, CsgOp::Difference, operands);
    Probe {
        name: "hole_grid",
        intent: "36-cutter n-ary difference",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(300.0 * 300.0 * 12.0 - 36.0 * PI * 100.0 * 12.0),
        volume_tol: 0.01,
        expected_euler: Some(2 - 2 * 36),
        expected_bodies: Some(1),
    }
}

/// Two boxes at arbitrary rotations, intersected and differenced.
fn rotated_boxes() -> Probe {
    let mut b = RecipeBuilder::new();
    let a = add_box(&mut b, [100.0, 60.0, 40.0], [0.0, 0.0, 0.0]);
    let c = b
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [90.0, 90.0, 90.0],
            },
            placement: Placement3::euler_extrinsic_xyz_then_translate(
                0.4,
                0.7,
                1.1,
                [20.0, 10.0, -20.0],
            ),
        })
        .expect("valid box");
    let inter = csg(&mut b, CsgOp::Intersection, vec![a, c]);
    let diff = csg(&mut b, CsgOp::Difference, vec![a, c]);
    let moved = b
        .add(NodeKind::Transform {
            child: diff,
            xf: Placement3::translate(0.0, 120.0, 0.0),
        })
        .expect("valid transform");
    let root = b
        .add(NodeKind::Group {
            children: vec![inter, moved],
        })
        .expect("valid group");
    Probe {
        name: "rotated_boxes",
        intent: "non-axis-aligned intersection and difference; volumes must sum to A",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(100.0 * 60.0 * 40.0),
        volume_tol: 1e-4,
        expected_euler: None,
        expected_bodies: Some(2),
    }
}

/// A leg definition instanced four times under a table top, the whole
/// table instanced twice, one of them mirrored.
fn nested_instances() -> Probe {
    let mut b = RecipeBuilder::new();
    let leg_profile = b.add_profile(builders::rounded_rect(30.0, 30.0, 6.0).expect("rounded"));
    let leg = add_extrude(&mut b, leg_profile, Placement3::IDENTITY, 400.0);
    let legs: Vec<NodeId> = [(10.0, 10.0), (360.0, 10.0), (10.0, 210.0), (360.0, 210.0)]
        .into_iter()
        .map(|(x, y)| {
            b.add(NodeKind::Instance {
                of: leg,
                placement: Placement3::translate(x, y, 0.0),
            })
            .expect("valid instance")
        })
        .collect();
    let top = add_box(&mut b, [400.0, 250.0, 20.0], [0.0, 0.0, 400.0]);
    let mut children = legs;
    children.push(top);
    let table = b.add(NodeKind::Group { children }).expect("valid group");
    let table_b = b
        .add(NodeKind::Instance {
            of: table,
            placement: Placement3::rotate_z_then_translate(0.3, 600.0, 0.0, 0.0),
        })
        .expect("valid instance");
    let table_c = b
        .add(NodeKind::Mirror {
            child: table_b,
            plane: plane([0.0, 1.0, 0.0], -50.0),
        })
        .expect("valid mirror");
    let root = b
        .add(NodeKind::Group {
            children: vec![table, table_b, table_c],
        })
        .expect("valid group");
    let leg_area = builders::profile_area(&builders::rounded_rect(30.0, 30.0, 6.0).expect("r"));
    Probe {
        name: "nested_instances",
        intent: "instances of a group of instances, one rotated, one mirrored",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: Some(3.0 * (4.0 * leg_area * 400.0 + 400.0 * 250.0 * 20.0)),
        volume_tol: 0.01,
        expected_euler: Some(3 * 5 * 2),
        expected_bodies: Some(15),
    }
}

/// A thickened grid shell with a box cut out of its crown.
fn grid_shell_cut() -> Probe {
    let mut b = RecipeBuilder::new();
    let (rows, cols) = (5_u32, 7_u32);
    let points: Vec<[f64; 3]> = (0..rows)
        .flat_map(|r| {
            (0..cols).map(move |c| {
                let (rf, cf) = (f64::from(r), f64::from(c));
                let bump = rf * (4.0 - rf) * cf * (6.0 - cf);
                [cf * 50.0, rf * 50.0, bump * 1.6]
            })
        })
        .collect();
    let shell = b
        .add(NodeKind::GridSurface {
            points,
            rows,
            cols,
            close_u: false,
            close_w: false,
            thickness: Some(12.0),
            placement: Placement3::IDENTITY,
        })
        .expect("valid grid");
    let cutter = add_box(&mut b, [60.0, 60.0, 100.0], [120.0, 70.0, 40.0]);
    let root = csg(&mut b, CsgOp::Difference, vec![shell, cutter]);
    Probe {
        name: "grid_shell_cut",
        intent: "doubly curved thickened grid minus a box",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: None,
        volume_tol: 0.0,
        expected_euler: None,
        expected_bodies: Some(1),
    }
}

/// A revolved goblet-like profile with a notch cut by a rotated box.
fn revolve_notched() -> Probe {
    let mut b = RecipeBuilder::new();
    let profile = Profile2::simple(
        Loop2::new(vec![
            Seg2::line((40.0, 0.0)),
            Seg2::arc((60.0, 60.0), 0.3),
            Seg2::line((60.0, 120.0)),
            Seg2::line((50.0, 120.0)),
            Seg2::line((50.0, 70.0)),
            Seg2::arc((30.0, 10.0), -0.25),
            Seg2::line((0.0, 10.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("goblet loop"),
    )
    .expect("goblet profile");
    let p = b.add_profile(profile);
    let goblet = b
        .add(NodeKind::Revolve {
            profile: p,
            placement: Placement3::IDENTITY,
            sweep: 2.0 * PI,
            caps: CapMode::Both,
        })
        .expect("valid revolve");
    // The revolve axis is local Y: the rim sits at y = 120, radius 50..60.
    let notch = b
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [40.0, 40.0, 200.0],
            },
            placement: Placement3::euler_extrinsic_xyz_then_translate(
                0.0,
                0.0,
                0.5,
                [45.0, 95.0, -100.0],
            ),
        })
        .expect("valid box");
    let root = csg(&mut b, CsgOp::Difference, vec![goblet, notch]);
    Probe {
        name: "revolve_notched",
        intent: "full revolve with arcs and an axis segment, minus a tilted box",
        recipe: b.finish(root).expect("valid recipe"),
        expected_volume: None,
        volume_tol: 0.0,
        expected_euler: None,
        expected_bodies: Some(1),
    }
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

fn mesh_volume(mesh: &Mesh) -> f64 {
    let mut six = 0.0;
    for face in mesh.faces() {
        for tri in mesh.face_triangles(face, FaceTriangulation::Robust) {
            let p = tri.map(|corner| {
                let v = mesh.to_vertex(corner).expect("corner vertex");
                mesh.vertex_position(v).expect("position").map(f64::from)
            });
            six += p[0][0] * (p[1][1] * p[2][2] - p[1][2] * p[2][1])
                - p[0][1] * (p[1][0] * p[2][2] - p[1][2] * p[2][0])
                + p[0][2] * (p[1][0] * p[2][1] - p[1][1] * p[2][0]);
        }
    }
    six / 6.0
}

fn euler(mesh: &Mesh) -> i64 {
    let count = |n: usize| i64::try_from(n).expect("mesh sizes fit i64");
    let v = count(mesh.vertices().count());
    let f = count(mesh.faces().count());
    let he = count(mesh.faces().map(|face| mesh.face_loop(face).count()).sum());
    v - he / 2 + f
}

fn boundary_edges(mesh: &Mesh) -> usize {
    mesh.boundary_loops()
        .map(|loops| loops.iter().map(Vec::len).sum())
        .unwrap_or(usize::MAX)
}

struct Outcome {
    lines: Vec<String>,
    problems: Vec<String>,
}

fn check(probe: &Probe, policy: &EvalPolicy) -> (Option<Evaluation>, Outcome) {
    let mut lines = Vec::new();
    let mut problems = Vec::new();
    let result = match evaluate(&probe.recipe, policy) {
        Ok(result) => result,
        Err(error) => {
            problems.push(format!("hard evaluation failure: {error}"));
            return (None, Outcome { lines, problems });
        }
    };
    let report = &result.report;
    let fidelities: Vec<String> = report
        .fidelity
        .iter()
        .filter(|(_, f)| *f != Fidelity::Exact)
        .map(|(n, f)| format!("{n:?}={f:?}"))
        .collect();
    lines.push(format!(
        "bodies={} tessellations={} faces={} vertices={} fan_unsafe={} envelope_only={}",
        report.counters.bodies,
        report.counters.tessellations,
        report.counters.faces,
        report.counters.vertices,
        report.counters.csg_fan_unsafe_faces,
        report.counters.envelope_only
    ));
    if !fidelities.is_empty() {
        lines.push(format!("non-exact fidelity: {}", fidelities.join(", ")));
    }
    for d in &report.diagnostics {
        lines.push(format!("  [{:?}] {} {}", d.severity, d.code, d.message));
    }
    if let Some(expected) = probe.expected_bodies
        && result.bodies.len() != expected
    {
        problems.push(format!(
            "expected {expected} bodies, got {}",
            result.bodies.len()
        ));
    }
    let mut total_volume = 0.0;
    let mut total_euler = 0;
    for (i, placed) in result.bodies.iter().enumerate() {
        let mesh = &placed.body.mesh;
        let errors = mesh.validate_deep();
        if !errors.is_empty() {
            problems.push(format!("body {i}: validate_deep: {errors:?}"));
        }
        if let Err(error) = placed.body.source_map.check(mesh) {
            problems.push(format!("body {i}: stale source map: {error:?}"));
        }
        let volume = mesh_volume(mesh);
        let chi = euler(mesh);
        let open = boundary_edges(mesh);
        lines.push(format!(
            "  body {i}: node={:?} faces={} volume={volume:.3} euler={chi} boundary_edges={open}",
            placed.node,
            mesh.faces().count()
        ));
        if volume < 0.0 {
            problems.push(format!(
                "body {i}: negative volume {volume:.3} (inside out)"
            ));
        }
        if open != 0 {
            problems.push(format!("body {i}: {open} boundary edges (not closed)"));
        }
        total_volume += volume;
        total_euler += chi;
    }
    if let Some(expected) = probe.expected_volume {
        let rel = (total_volume - expected).abs() / expected.abs().max(1e-9);
        lines.push(format!(
            "volume total={total_volume:.3} expected={expected:.3} rel_err={rel:.2e}"
        ));
        if rel > probe.volume_tol {
            problems.push(format!(
                "volume off by {rel:.2e} (tolerance {:.0e})",
                probe.volume_tol
            ));
        }
    }
    if let Some(expected) = probe.expected_euler
        && !result.bodies.is_empty()
        && total_euler != expected
    {
        problems.push(format!(
            "euler characteristic {total_euler}, expected {expected}"
        ));
    }
    // Cold versus warm: bit identity of geometry, attributes, and report.
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&probe.recipe, policy, &mut cache);
    let warm = evaluate_with_cache(&probe.recipe, policy, &mut cache);
    match (cold, warm) {
        (Ok(cold), Ok(warm)) => {
            if cold.report.fidelity != warm.report.fidelity
                || cold.report.diagnostics != warm.report.diagnostics
            {
                problems.push("warm report differs from cold".into());
            }
            if cold.bodies.len() != warm.bodies.len() {
                problems.push("warm body count differs from cold".into());
            }
            for (a, b) in cold.bodies.iter().zip(&warm.bodies) {
                if exedra_testkit::dump_mesh_topology(&a.body.mesh)
                    != exedra_testkit::dump_mesh_topology(&b.body.mesh)
                    || exedra_testkit::dump_attributes(&a.body.mesh)
                        != exedra_testkit::dump_attributes(&b.body.mesh)
                {
                    problems.push("warm mesh differs from cold".into());
                }
            }
            lines.push(format!(
                "warm: hits={} misses={} tessellations={}",
                warm.report.counters.cache_hits,
                warm.report.counters.cache_misses,
                warm.report.counters.tessellations
            ));
            if warm.report.counters.tessellations != 0 && cold.report.counters.envelope_only == 0 {
                lines.push("  note: warm run still tessellated".into());
            }
        }
        _ => problems.push("cached evaluation failed".into()),
    }
    (Some(result), Outcome { lines, problems })
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

fn export(assembly: &Assembly, policy: &EvalPolicy, path: &Path) -> Result<String, String> {
    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(assembly, &(*policy).into())
        .map_err(|e| format!("compile: {e}"))?;
    let list = flatten(assembly, &compiled);
    let glb = export_glb_with_options(
        assembly,
        &compiled,
        &list,
        GltfExportOptions::z_up_to_y_up(),
    )
    .map_err(|e| format!("glb: {e}"))?;
    std::fs::write(path, &glb.bytes).map_err(|e| format!("write: {e}"))?;
    Ok(format!(
        "glb nodes={} meshes={} primitives={} bytes={}",
        glb.stats.nodes, glb.stats.meshes, glb.stats.primitives, glb.stats.buffer_bytes
    ))
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .expect("usage: constructive_probe <out_dir>");
    std::fs::create_dir_all(&out).expect("out dir");
    let policy = probe_policy();
    let mut report = String::new();
    let mut all = Assembly::new();
    let mut column = 0.0;
    let mut summary: Vec<(String, usize)> = Vec::new();
    for probe in probes() {
        let (result, outcome) = check(&probe, &policy);
        writeln!(report, "== {} — {}", probe.name, probe.intent).unwrap();
        for line in &outcome.lines {
            writeln!(report, "{line}").unwrap();
        }
        for problem in &outcome.problems {
            writeln!(report, "PROBLEM: {problem}").unwrap();
        }
        if result.as_ref().is_some_and(|r| !r.bodies.is_empty()) {
            let mut single = Assembly::new();
            let part = single
                .add_recipe_part(probe.name, probe.recipe.clone())
                .expect("part");
            single
                .add_instance(None, probe.name, part, Placement3::IDENTITY)
                .expect("instance");
            match export(&single, &policy, &out.join(format!("{}.glb", probe.name))) {
                Ok(stats) => writeln!(report, "{stats}").unwrap(),
                Err(error) => writeln!(report, "PROBLEM: export {error}").unwrap(),
            }
            let part = all.add_recipe_part(probe.name, probe.recipe).expect("part");
            all.add_instance(
                None,
                probe.name,
                part,
                Placement3::translate(column, 0.0, 0.0),
            )
            .expect("instance");
            column += 900.0;
        }
        writeln!(report).unwrap();
        summary.push((probe.name.to_string(), outcome.problems.len()));
    }
    match export(&all, &policy, &out.join("all.glb")) {
        Ok(stats) => writeln!(report, "all: {stats}").unwrap(),
        Err(error) => writeln!(report, "PROBLEM: all export {error}").unwrap(),
    }
    writeln!(report, "\n== summary").unwrap();
    for (name, problems) in &summary {
        writeln!(report, "{name}: {problems} problem(s)").unwrap();
    }
    std::fs::write(out.join("report.txt"), &report).expect("write report");
    print!("{report}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Probes the kernel refuses today, with the diagnostic naming why.
    /// Equal-radius perpendicular cylinders put one cylinder's edges exactly
    /// on the other's surface; the Boolean pipeline defers those cuts.
    const KNOWN_REFUSALS: &[(&str, &str)] = &[
        ("two_cylinder_intersection", "eval.csg.unsupported"),
        ("three_cylinder_intersection", "eval.csg.unsupported"),
        ("three_cylinder_chained", "eval.csg.unsupported"),
    ];

    /// Probes that fail the whole evaluation today rather than reporting
    /// a typed refusal.
    const KNOWN_HARD_FAILURES: &[&str] = &[];

    #[test]
    fn every_probe_matches_its_pinned_outcome() {
        let policy = probe_policy();
        for probe in probes() {
            let (result, outcome) = check(&probe, &policy);
            if KNOWN_HARD_FAILURES.contains(&probe.name) {
                assert!(
                    result.is_none(),
                    "{}: now evaluates; update the pinned outcome",
                    probe.name
                );
                continue;
            }
            let result = result
                .unwrap_or_else(|| panic!("{}: hard failure: {:?}", probe.name, outcome.problems));
            if let Some((_, code)) = KNOWN_REFUSALS.iter().find(|(name, _)| *name == probe.name) {
                assert!(
                    result.report.diagnostics.iter().any(|d| d.code == *code),
                    "{}: expected refusal {code}, got {:?}",
                    probe.name,
                    result.report.diagnostics
                );
                assert!(
                    result.bodies.is_empty(),
                    "{}: refused yet emitted",
                    probe.name
                );
                continue;
            }
            assert!(
                outcome.problems.is_empty(),
                "{}: {:?}",
                probe.name,
                outcome.problems
            );
        }
    }
}
