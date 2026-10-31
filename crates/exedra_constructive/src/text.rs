// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `constructive-ir-v1`: a deterministic, human-diffable text rendering of
//! a recipe, with a round-trip parser.
//!
//! The format follows the workspace golden-file conventions: one header
//! line, ordered sections, every `f64` as a 16-digit uppercase hex bit
//! pattern (bit-exact, no shortest-float ambiguity), and no maps anywhere.
//! Parsing rebuilds through [`RecipeBuilder`], so a parsed recipe re-runs
//! full validation and recomputes fingerprints — round-trip fingerprint
//! equality is the format's correctness oracle.
//!
//! This is the goldens/review/debugging format. The serde interchange
//! format (JSON, for external frontends) is a separate surface.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use kurbo::Point;

use crate::chart::{ChartTransform, SurfaceChart};
use crate::edge_finish::{EdgeSelection, OperandRegion, RoundKind, RoundPolicy};
use crate::ir::{
    CapMode, CsgOp, FramePolicy, Law, LoftPolicy, LoftSection, Node, NodeId, NodeKind, Path3,
    PathClosure, PathJoin, Placement3, Plane3, PrimitiveSpec, ProfileId, Recipe, RecipeBuilder,
    RecipeError, SectionLaw, VertexStretchStep,
};
use crate::profile::{Loop2, Profile2, ProfileError, Seg2, SegKind, SegTag};

/// Header line of the format.
pub const HEADER: &str = "constructive-ir-v1";

fn hex(v: f64) -> String {
    format!("{:016X}", v.to_bits())
}

fn put_point(out: &mut String, p: Point) {
    let _ = write!(out, "{} {}", hex(p.x), hex(p.y));
}

fn put_placement(out: &mut String, p: &Placement3) {
    for row in &p.rows {
        for &v in row {
            let _ = write!(out, " {}", hex(v));
        }
    }
}

fn put_caps(out: &mut String, caps: CapMode) {
    let name = match caps {
        CapMode::Both => "both",
        CapMode::Start => "start",
        CapMode::End => "end",
        CapMode::None => "none",
    };
    let _ = write!(out, "{name}");
}

/// Renders a recipe as `constructive-ir-v1` text.
#[must_use]
pub fn dump_recipe(recipe: &Recipe) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{HEADER}");
    let _ = writeln!(out, "schema {}", crate::EVAL_SCHEMA_VERSION);

    let _ = writeln!(out, "sources {}", recipe.sources().len());
    for (index, source) in recipe.sources().iter().enumerate() {
        let _ = writeln!(out, "  source {index} {source:?}");
    }
    let _ = writeln!(out, "slots {}", recipe.slots().len());
    for (index, slot) in recipe.slots().iter().enumerate() {
        let _ = writeln!(out, "  slot {index} {slot:?}");
    }
    let _ = writeln!(out, "policies {}", recipe.policies().len());
    for (index, policy) in recipe.policies().iter().enumerate() {
        let _ = writeln!(out, "  policy {index} {policy:?}");
    }
    let _ = writeln!(out, "imports {}", recipe.imports().len());
    for (index, mesh) in recipe.imports().iter().enumerate() {
        dump_import(&mut out, index, mesh);
    }

    let _ = writeln!(out, "profiles {}", recipe.profiles().len());
    for (index, profile) in recipe.profiles().iter().enumerate() {
        let _ = writeln!(out, "  profile {index} holes {}", profile.holes().len());
        dump_loop(&mut out, profile.outer());
        for hole in profile.holes() {
            dump_loop(&mut out, hole);
        }
    }

    let _ = writeln!(out, "nodes {}", recipe.nodes().len());
    for (index, node) in recipe.nodes().iter().enumerate() {
        let mut line = format!("  node {index} ");
        if let Some(chart) = node.surface_chart {
            line.push_str("charted ");
            match chart {
                SurfaceChart::Extrude { .. } => line.push_str("extrude "),
                SurfaceChart::Sweep { .. } => line.push_str("sweep "),
                SurfaceChart::Loft {
                    reference_section,
                    rest_length,
                    stations,
                    ..
                } => {
                    let _ = write!(
                        line,
                        "loft reference {reference_section} rest_length {} ",
                        hex(rest_length)
                    );
                    if stations == crate::chart::LoftStations::DatumDistance {
                        line.push_str("stations datum_distance ");
                    }
                }
                SurfaceChart::Revolve {
                    reference_radius, ..
                } => {
                    let _ = write!(line, "revolve radius {} ", hex(reference_radius));
                }
            }
            for (label, t) in [("wall", chart.wall()), ("caps", chart.caps())] {
                let _ = write!(line, "{label} ");
                for v in t.matrix.into_iter().flatten().chain(t.offset) {
                    let _ = write!(line, "{} ", hex(v));
                }
            }
            line.push_str("operation ");
        }
        dump_kind(&mut line, &node.kind);
        match node.source {
            None => line.push_str(" source -"),
            Some(id) => {
                let _ = write!(line, " source {}", id.0);
            }
        }
        match node.material {
            None => line.push_str(" material -"),
            Some(id) => {
                let _ = write!(line, " material {}", id.0);
            }
        }
        match node.issue {
            None => line.push_str(" issue -"),
            Some(id) => {
                let _ = write!(line, " issue {}", id.0);
            }
        }
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out, "root {}", recipe.root().0);
    out
}

fn dump_loop(out: &mut String, source: &Loop2) {
    let _ = writeln!(out, "    loop {}", source.segs().len());
    for seg in source.segs() {
        let mut line = String::from("      ");
        dump_seg_kind(&mut line, &seg.kind);
        put_point(&mut line, seg.to);
        match seg.tag {
            None => line.push_str(" tag -"),
            Some(SegTag(tag)) => {
                let _ = write!(line, " tag {tag}");
            }
        }
        let _ = writeln!(out, "{line}");
    }
}

fn dump_import(out: &mut String, index: usize, mesh: &exedra_mesh::Mesh) {
    let vertices: Vec<exedra_mesh::VertexId> = mesh.vertices().collect();
    let faces: Vec<exedra_mesh::FaceId> = mesh.faces().collect();
    let mut header = alloc::format!(
        "  import {index} vertices {} faces {}",
        vertices.len(),
        faces.len()
    );
    if let Some(rule) = mesh.attrs().propagation(
        exedra_mesh::attr::CORNER_UV1.domain(),
        exedra_mesh::attr::CORNER_UV1.name(),
    ) {
        let value = match rule {
            exedra_mesh::attributes::Propagation::Unspecified => 0,
            exedra_mesh::attributes::Propagation::Clear => 1,
            exedra_mesh::attributes::Propagation::Copy => 2,
            exedra_mesh::attributes::Propagation::Interpolate => 3,
        };
        let _ = write!(header, " uv1_rule {value}");
    }
    if let Some(layer) = mesh.attrs().dense(exedra_mesh::attr::CORNER_UV1) {
        let [u, v] = *layer.default();
        let _ = write!(header, " uv1_dense {:08X} {:08X}", u.to_bits(), v.to_bits());
    }
    let _ = writeln!(out, "{header}");
    for vertex in &vertices {
        if let Some(p) = mesh.vertex_position(*vertex) {
            let _ = writeln!(
                out,
                "    v {:08X} {:08X} {:08X}",
                p[0].to_bits(),
                p[1].to_bits(),
                p[2].to_bits()
            );
        }
    }
    for face in &faces {
        let mut line = String::from("    f");
        for index in crate::ir::canonical_face_loop_pub(mesh, *face) {
            let _ = write!(line, " {index}");
        }
        let corners = crate::ir::canonical_face_loop_half_edges(mesh, *face);
        dump_corner_channel(
            &mut line,
            "uv",
            corners
                .iter()
                .map(|&corner| {
                    mesh.attrs()
                        .sparse(exedra_mesh::attr::CORNER_UV)
                        .and_then(|layer| layer.get(corner.into()))
                        .copied()
                })
                .collect::<Vec<_>>(),
        );
        dump_corner_channel(
            &mut line,
            "uv1",
            corners
                .iter()
                .map(|&corner| {
                    exedra_mesh_ops::uv::corner_uv(mesh, crate::ir::UvSet::Secondary, corner)
                })
                .collect::<Vec<_>>(),
        );
        dump_corner_channel(
            &mut line,
            "normals",
            corners
                .iter()
                .map(|&corner| {
                    mesh.attrs()
                        .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                        .and_then(|layer| layer.get(corner.into()))
                        .copied()
                })
                .collect::<Vec<_>>(),
        );
        let _ = writeln!(out, "{line}");
    }
}

fn dump_corner_channel<const N: usize>(
    line: &mut String,
    label: &str,
    values: Vec<Option<[f32; N]>>,
) {
    if values.iter().all(Option::is_none) {
        return;
    }
    let _ = write!(line, " {label}");
    for value in values {
        match value {
            None => line.push_str(" -"),
            Some(value) => {
                for component in value {
                    let _ = write!(line, " {:08X}", component.to_bits());
                }
            }
        }
    }
}

fn parse_corner_channel<const N: usize>(
    text: Option<&str>,
    count: usize,
    line: usize,
) -> Result<Vec<Option<[f32; N]>>, TextError> {
    let Some(text) = text else {
        return Ok(alloc::vec![None; count]);
    };
    let mut tokens = text.split_whitespace();
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let first = tokens.next().ok_or(TextError::Malformed { line })?;
        if first == "-" {
            values.push(None);
            continue;
        }
        let mut value = [0.0; N];
        for (index, component) in value.iter_mut().enumerate() {
            let token = if index == 0 {
                first
            } else {
                tokens.next().ok_or(TextError::Malformed { line })?
            };
            *component = f32::from_bits(
                u32::from_str_radix(token, 16).map_err(|_| TextError::Malformed { line })?,
            );
        }
        values.push(Some(value));
    }
    if tokens.next().is_some() {
        return Err(TextError::Malformed { line });
    }
    Ok(values)
}

fn dump_seg_kind(line: &mut String, kind: &SegKind) {
    match kind {
        SegKind::Line => line.push_str("line to "),
        SegKind::Arc { bulge } => {
            let _ = write!(line, "arc {} to ", hex(*bulge));
        }
        SegKind::Cubic { c1, c2 } => {
            line.push_str("cubic ");
            put_point(line, *c1);
            line.push(' ');
            put_point(line, *c2);
            line.push_str(" to ");
        }
        SegKind::PolicyTo { policy, realized } => {
            let _ = write!(line, "policy {} ", policy.0);
            dump_seg_kind(line, realized);
        }
    }
}

fn dump_kind(line: &mut String, kind: &NodeKind) {
    match kind {
        NodeKind::UvMap {
            child,
            mapping,
            options,
        } => {
            use crate::ir::UvProjection;
            let _ = write!(
                line,
                "uv_map child {} options {} {} {} {} ",
                child.0,
                u8::from(options.source == crate::ir::UvSet::Secondary),
                u8::from(options.target == crate::ir::UvSet::Secondary),
                u8::from(options.seams == crate::ir::UvSeamPolicy::Continuous),
                u8::from(options.poles == crate::ir::UvPolePolicy::FaceAverage)
            );
            match mapping.projection {
                UvProjection::Face => line.push_str("face"),
                UvProjection::Cylindrical(frame)
                | UvProjection::Spherical(frame)
                | UvProjection::Toroidal { frame, .. } => {
                    line.push_str(match mapping.projection {
                        UvProjection::Cylindrical(_) => "cylindrical",
                        UvProjection::Spherical(_) => "spherical",
                        _ => "toroidal",
                    });
                    for vector in [frame.origin, frame.u, frame.v] {
                        put_vector3(line, vector);
                    }
                    if let UvProjection::Toroidal { major_radius, .. } = mapping.projection {
                        let _ = write!(line, " {}", hex(major_radius));
                    }
                }
                UvProjection::Perspective(projector) => {
                    line.push_str("perspective");
                    for vector in [projector.frame.origin, projector.frame.u, projector.frame.v] {
                        put_vector3(line, vector);
                    }
                    for value in projector.focal.into_iter().chain([projector.near]) {
                        let _ = write!(line, " {}", hex(value));
                    }
                }
                UvProjection::Existing => line.push_str("existing"),
                UvProjection::Planar(frame) => {
                    line.push_str("planar");
                    for vector in [frame.origin, frame.u, frame.v] {
                        put_vector3(line, vector);
                    }
                }
                UvProjection::Box(layout) => {
                    line.push_str("box");
                    for flag in layout
                        .swap_axes
                        .into_iter()
                        .chain(layout.flip_u)
                        .chain(layout.flip_v)
                    {
                        let _ = write!(line, " {}", u8::from(flag));
                    }
                }
            }
            line.push_str(" transform");
            for value in mapping
                .transform
                .matrix
                .into_iter()
                .flatten()
                .chain(mapping.transform.offset)
            {
                let _ = write!(line, " {}", hex(value));
            }
        }
        NodeKind::OnWorkplane {
            support,
            child,
            attachment,
        } => {
            use crate::workplane::SurfaceSelector;
            let _ = write!(
                line,
                "on_workplane support {} child {} surface ",
                support.0, child.0
            );
            match &attachment.surface {
                SurfaceSelector::SourceStartCap(source) | SurfaceSelector::SourceEndCap(source) => {
                    line.push_str(
                        if matches!(attachment.surface, SurfaceSelector::SourceStartCap(_)) {
                            "source_start_cap "
                        } else {
                            "source_end_cap "
                        },
                    );
                    // Prefix keeps the empty UTF-8 label a nonempty token.
                    line.push('x');
                    for byte in source.as_bytes() {
                        let _ = write!(line, "{byte:02X}");
                    }
                }
                SurfaceSelector::StartCap => line.push_str("start_cap"),
                SurfaceSelector::EndCap => line.push_str("end_cap"),
                SurfaceSelector::Region(region) => {
                    let _ = write!(line, "region {region}");
                }
                SurfaceSelector::OperandRegion(value) => {
                    let _ = write!(line, "operand_region {} {}", value.operand, value.region);
                }
            }
            for (name, vector) in [
                ("anchor", attachment.anchor),
                ("projection", attachment.projection),
                ("x", attachment.x_direction),
            ] {
                let _ = write!(
                    line,
                    " {name} {} {} {}",
                    hex(vector[0]),
                    hex(vector[1]),
                    hex(vector[2])
                );
            }
        }
        NodeKind::PlaneCut {
            child,
            plane,
            side,
            cap,
        } => {
            let side = match side {
                crate::ir::PlaneSide::Negative => "negative",
                crate::ir::PlaneSide::Positive => "positive",
            };
            let material = cap
                .material
                .map_or_else(|| String::from("-"), |slot| format!("{}", slot.0));
            let _ = write!(
                line,
                "plane_cut child {} plane {} {} {} {} side {side} cap_region {} cap_material {material}",
                child.0,
                hex(plane.normal[0]),
                hex(plane.normal[1]),
                hex(plane.normal[2]),
                hex(plane.distance),
                cap.region
            );
        }
        NodeKind::ExtrudeToPlane {
            profile,
            placement,
            plane,
        } => {
            let _ = write!(
                line,
                "extrude_to_plane profile {} plane {} {} {} {} placement",
                profile.0,
                hex(plane.normal[0]),
                hex(plane.normal[1]),
                hex(plane.normal[2]),
                hex(plane.distance)
            );
            put_placement(line, placement);
        }
        NodeKind::Extrude {
            profile,
            placement,
            height,
            caps,
        } => {
            let _ = write!(
                line,
                "extrude profile {} height {} caps ",
                profile.0,
                hex(*height)
            );
            put_caps(line, *caps);
            line.push_str(" placement");
            put_placement(line, placement);
        }
        NodeKind::Revolve {
            profile,
            placement,
            sweep,
            caps,
        } => {
            let _ = write!(
                line,
                "revolve profile {} sweep {} caps ",
                profile.0,
                hex(*sweep)
            );
            put_caps(line, *caps);
            line.push_str(" placement");
            put_placement(line, placement);
        }
        NodeKind::Loft {
            sections,
            policy,
            caps,
        } => {
            let mode = match policy {
                LoftPolicy::Ruled => "ruled",
                LoftPolicy::Smooth => "smooth",
            };
            let _ = write!(line, "loft {mode} sections {} caps ", sections.len());
            put_caps(line, *caps);
            for section in sections {
                let _ = write!(line, " section {} placement", section.profile.0);
                put_placement(line, &section.placement);
                if let Some(source) = section.source {
                    let _ = write!(line, " section_source {}", source.0);
                }
            }
        }
        NodeKind::Sweep {
            profile,
            path,
            section,
            caps,
        } => {
            if !section.is_identity() {
                line.push_str("shaped scale ");
                put_law(line, &section.scale);
                line.push_str(" twist ");
                put_law(line, &section.twist);
                if section.turns != 0 {
                    let _ = write!(line, " turns {}", section.turns);
                }
                line.push(' ');
            }
            if let Path3::Curves {
                start,
                segments,
                section_x,
                section_origin,
                closure,
                joins,
            } = path
            {
                let _ = write!(line, "curved_path_sweep profile {} start", profile.0);
                put_vector3(line, *start);
                line.push_str(" section_x");
                put_vector3(line, *section_x);
                let _ = write!(
                    line,
                    " section_origin {} {} closure ",
                    hex(section_origin[0]),
                    hex(section_origin[1])
                );
                match closure {
                    PathClosure::Open => line.push_str("open"),
                    PathClosure::ClosedPlanar { normal } => {
                        line.push_str("closed_planar");
                        put_vector3(line, *normal);
                    }
                }
                match joins {
                    PathJoin::Smooth => line.push_str(" joins smooth"),
                    PathJoin::Miter { limit } => {
                        let _ = write!(line, " joins miter {}", hex(*limit));
                    }
                }
                let _ = write!(line, " segments {} caps ", segments.len());
                put_caps(line, *caps);
                for segment in segments {
                    match segment {
                        crate::path::PathSegment3::Line { to } => {
                            line.push_str(" line");
                            put_vector3(line, *to);
                        }
                        crate::path::PathSegment3::Arc {
                            axis_origin,
                            axis,
                            sweep,
                        } => {
                            line.push_str(" arc");
                            put_vector3(line, *axis_origin);
                            put_vector3(line, *axis);
                            let _ = write!(line, " {}", hex(*sweep));
                        }
                        crate::path::PathSegment3::Cubic {
                            control1,
                            control2,
                            to,
                        } => {
                            line.push_str(" cubic");
                            put_vector3(line, *control1);
                            put_vector3(line, *control2);
                            put_vector3(line, *to);
                        }
                    }
                }
                return;
            }
            let points = path.polyline_points().expect("polyline variant");
            match path {
                Path3::Polyline { frame, .. } => {
                    let FramePolicy::RotationMinimizing = frame;
                    let _ = write!(
                        line,
                        "sweep profile {} frame rmf points {} caps ",
                        profile.0,
                        points.len()
                    );
                }
                Path3::MiteredPolyline {
                    section_x,
                    section_origin,
                    closure,
                    miter_limit,
                    ..
                } => {
                    let _ = write!(
                        line,
                        "mitered_path_sweep profile {} section_origin {} {} closure ",
                        profile.0,
                        hex(section_origin[0]),
                        hex(section_origin[1])
                    );
                    match closure {
                        PathClosure::Open => line.push_str("open"),
                        PathClosure::ClosedPlanar { normal } => {
                            line.push_str("closed_planar");
                            put_vector3(line, *normal);
                        }
                    }
                    let _ = write!(
                        line,
                        " section_x {} {} {} miter_limit {} points {} caps ",
                        hex(section_x[0]),
                        hex(section_x[1]),
                        hex(section_x[2]),
                        hex(*miter_limit),
                        points.len()
                    );
                }
                Path3::Curves { .. } => unreachable!("handled above"),
            }
            put_caps(line, *caps);
            for p in points {
                let _ = write!(line, " {} {} {}", hex(p[0]), hex(p[1]), hex(p[2]));
            }
        }
        NodeKind::PlanarFace { profile, placement } => {
            let _ = write!(line, "planar_face profile {} placement", profile.0);
            put_placement(line, placement);
        }
        NodeKind::Primitive { spec, placement } => {
            match spec {
                PrimitiveSpec::Box { size } => {
                    let _ = write!(
                        line,
                        "primitive box {} {} {}",
                        hex(size[0]),
                        hex(size[1]),
                        hex(size[2])
                    );
                }
                PrimitiveSpec::Cylinder {
                    radius,
                    height,
                    segments,
                } => {
                    let _ = write!(
                        line,
                        "primitive cylinder {} {} {segments}",
                        hex(*radius),
                        hex(*height)
                    );
                }
            }
            line.push_str(" placement");
            put_placement(line, placement);
        }
        NodeKind::Csg { op, operands } => {
            let name = match op {
                CsgOp::Union => "union",
                CsgOp::Difference => "difference",
                CsgOp::Intersection => "intersection",
            };
            let _ = write!(line, "csg {name} operands {}", operands.len());
            for operand in operands {
                let _ = write!(line, " {}", operand.0);
            }
        }
        NodeKind::Transform { child, xf } => {
            let _ = write!(line, "transform child {} placement", child.0);
            put_placement(line, xf);
        }
        NodeKind::Mirror { child, plane } => {
            let _ = write!(
                line,
                "mirror child {} plane {} {} {} {}",
                child.0,
                hex(plane.normal[0]),
                hex(plane.normal[1]),
                hex(plane.normal[2]),
                hex(plane.distance)
            );
        }
        NodeKind::Instance { of, placement } => {
            let _ = write!(line, "instance of {} placement", of.0);
            put_placement(line, placement);
        }
        NodeKind::Group { children } => {
            let _ = write!(line, "group children {}", children.len());
            for child in children {
                let _ = write!(line, " {}", child.0);
            }
        }
        NodeKind::MeshImport { import, placement } => {
            let _ = write!(line, "mesh_import {} placement", import.0);
            put_placement(line, placement);
        }
        NodeKind::EdgeFinish {
            child,
            selection,
            policy,
        } => {
            let _ = write!(line, "edge_finish child {} ", child.0);
            match selection {
                EdgeSelection::SharpEdges => line.push_str("sharp"),
                EdgeSelection::RegionBoundaries(pairs) => {
                    let _ = write!(line, "boundaries {}", pairs.len());
                    for pair in pairs {
                        let _ = write!(line, " {} {}", pair[0], pair[1]);
                    }
                }
                EdgeSelection::OperandBoundaries(pairs) => {
                    let _ = write!(line, "operand_boundaries {}", pairs.len());
                    for pair in pairs {
                        for source in pair {
                            let _ = write!(line, " {} {}", source.operand, source.region);
                        }
                    }
                }
            }
            let (kind, offset) = match policy.kind {
                RoundKind::Fillet { radius } => ("fillet", radius),
                RoundKind::Chamfer { setback } => ("chamfer", setback),
            };
            let segments = policy
                .segments
                .map_or_else(|| String::from("-"), |n| format!("{n}"));
            let region = policy
                .region
                .map_or_else(|| String::from("-"), |n| format!("{n}"));
            let _ = write!(
                line,
                " {kind} {} segments {segments} tolerance {} sharpness {} region {region} planar {} turn {}",
                hex(offset),
                hex(policy.chord_tolerance),
                policy.sharpness_threshold.to_bits(),
                hex(policy.max_planar_deviation),
                hex(policy.max_tangent_turn)
            );
        }
        NodeKind::StretchVertices { child, steps } => {
            let _ = write!(
                line,
                "stretch_vertices child {} steps {}",
                child.0,
                steps.len()
            );
            for step in steps {
                let _ = write!(
                    line,
                    " plane {} {} {} {} length {}",
                    hex(step.plane.normal[0]),
                    hex(step.plane.normal[1]),
                    hex(step.plane.normal[2]),
                    hex(step.plane.distance),
                    hex(step.length)
                );
            }
        }
        NodeKind::Stretch {
            child,
            plane,
            length,
        } => {
            let _ = write!(
                line,
                "stretch child {} plane {} {} {} {} length {}",
                child.0,
                hex(plane.normal[0]),
                hex(plane.normal[1]),
                hex(plane.normal[2]),
                hex(plane.distance),
                hex(*length)
            );
        }
        NodeKind::GridSurface {
            points,
            rows,
            cols,
            close_u,
            close_w,
            thickness,
            placement,
        } => {
            let _ = write!(
                line,
                "grid_surface rows {} cols {} close_u {} close_w {} thickness {}",
                rows,
                cols,
                u8::from(*close_u),
                u8::from(*close_w),
                match thickness {
                    Some(t) => hex(*t),
                    None => String::from("-"),
                }
            );
            let _ = write!(line, " points {}", points.len());
            for p in points {
                let _ = write!(line, " {} {} {}", hex(p[0]), hex(p[1]), hex(p[2]));
            }
            let _ = write!(line, " placement");
            put_placement(line, placement);
        }
        NodeKind::Implicit {
            field,
            bounds,
            placement,
        } => {
            let _ = write!(line, "implicit bounds");
            match bounds {
                Some(b) => put_hexes(line, &[b.min, b.max].concat()),
                None => line.push_str(" -"),
            }
            let _ = write!(line, " placement");
            put_placement(line, placement);
            let _ = write!(line, " field");
            put_implicit(line, field);
        }
    }
}

fn put_hexes(out: &mut String, values: &[f64]) {
    for &v in values {
        let _ = write!(out, " {}", hex(v));
    }
}

/// Writes `field` in prefix order: each variant's name, then its children
/// and parameters.
fn put_implicit(out: &mut String, field: &exedra_implicit::Expr) {
    use exedra_implicit::Expr;
    match field {
        Expr::Sphere(s) => {
            out.push_str(" sphere");
            put_hexes(out, &s.center());
            put_hexes(out, &[s.radius()]);
        }
        Expr::Cuboid(c) => {
            out.push_str(" cuboid");
            put_hexes(out, &c.center());
            put_hexes(out, &c.half_extents());
        }
        Expr::Cylinder(c) => {
            out.push_str(" cylinder");
            put_hexes(out, &c.center());
            put_hexes(out, &c.axis());
            put_hexes(out, &[c.radius(), c.half_height()]);
        }
        Expr::Torus(t) => {
            out.push_str(" torus");
            put_hexes(out, &t.center());
            put_hexes(out, &t.axis());
            put_hexes(out, &[t.major_radius(), t.minor_radius()]);
        }
        Expr::HalfSpace(h) => {
            out.push_str(" half_space");
            put_hexes(out, &h.point());
            put_hexes(out, &h.normal());
        }
        Expr::Capsule(c) => {
            out.push_str(" capsule");
            put_hexes(out, &c.a());
            put_hexes(out, &c.b());
            put_hexes(out, &[c.radius()]);
        }
        Expr::RoundCone(c) => {
            out.push_str(" round_cone");
            put_hexes(out, &c.a());
            put_hexes(out, &c.b());
            put_hexes(out, &[c.radius_a(), c.radius_b()]);
        }
        Expr::Extrude(e) => {
            out.push_str(" extrude");
            put_implicit_profile(out, e.profile());
            put_hexes(out, &[e.half_height()]);
        }
        Expr::Revolve(r) => {
            out.push_str(" revolve");
            put_implicit_profile(out, r.profile());
        }
        Expr::Union(u) => {
            out.push_str(" union");
            put_implicit(out, &u.left);
            put_implicit(out, &u.right);
        }
        Expr::Intersection(i) => {
            out.push_str(" intersection");
            put_implicit(out, &i.left);
            put_implicit(out, &i.right);
        }
        Expr::Difference(d) => {
            out.push_str(" difference");
            put_implicit(out, &d.left);
            put_implicit(out, &d.right);
        }
        Expr::SmoothUnion(u) => {
            out.push_str(" smooth_union");
            put_implicit(out, u.left());
            put_implicit(out, u.right());
            put_hexes(out, &[u.radius()]);
        }
        Expr::Blend(b) => {
            let _ = write!(out, " blend {}", b.parts().len());
            for part in b.parts() {
                put_implicit(out, part);
            }
            put_hexes(out, &[b.radius()]);
        }
        Expr::Translate(t) => {
            out.push_str(" translate");
            put_implicit(out, t.field());
            put_hexes(out, &t.offset());
        }
        Expr::Scale(t) => {
            out.push_str(" scale");
            put_implicit(out, t.field());
            put_hexes(out, &[t.factor()]);
        }
        Expr::Placed(t) => {
            out.push_str(" placed");
            put_implicit(out, t.field());
            put_placement(out, &t.placement());
        }
        Expr::Tagged(t) => {
            out.push_str(" tagged");
            put_implicit(out, &t.field);
            let _ = write!(out, " {}", t.region);
        }
    }
}

fn put_implicit_profile(out: &mut String, profile: &exedra_implicit::ProfileExpr) {
    use exedra_implicit::ProfileExpr;
    match profile {
        ProfileExpr::Circle(c) => {
            out.push_str(" circle");
            put_hexes(out, &c.center());
            put_hexes(out, &[c.radius()]);
        }
        ProfileExpr::Rectangle(r) => {
            out.push_str(" rectangle");
            put_hexes(out, &r.center());
            put_hexes(out, &r.half_extents());
        }
        ProfileExpr::HalfPlane(h) => {
            out.push_str(" half_plane");
            put_hexes(out, &h.point());
            put_hexes(out, &h.normal());
        }
    }
}

/// Typed parse failure with a 1-based line number.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TextError {
    /// The header or schema line is missing or wrong.
    BadHeader,
    /// A line failed to parse.
    Malformed {
        /// 1-based line number.
        line: usize,
    },
    /// The rebuilt profile failed validation.
    Profile(ProfileError),
    /// The rebuilt recipe failed validation.
    Recipe(RecipeError),
    /// An implicit field's parameters were refused on rebuild.
    Implicit {
        /// 1-based line number.
        line: usize,
        /// The refusal.
        error: exedra_implicit::FieldError,
    },
}

impl core::fmt::Display for TextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadHeader => write!(f, "missing or unsupported header"),
            Self::Malformed { line } => write!(f, "malformed line {line}"),
            Self::Profile(e) => write!(f, "profile validation failed: {e}"),
            Self::Recipe(e) => write!(f, "recipe validation failed: {e}"),
            Self::Implicit { line, error } => {
                write!(f, "implicit field on line {line} refused: {error}")
            }
        }
    }
}

impl core::error::Error for TextError {}

struct Lines<'a> {
    iter: core::iter::Enumerate<core::str::Lines<'a>>,
}

impl<'a> Lines<'a> {
    fn next(&mut self) -> Result<(usize, &'a str), TextError> {
        self.iter
            .next()
            .map(|(index, line)| (index + 1, line.trim()))
            .ok_or(TextError::Malformed { line: usize::MAX })
    }
}

fn put_law(out: &mut String, law: &Law) {
    match law {
        Law::Constant(value) => {
            let _ = write!(out, "constant {}", hex(*value));
        }
        Law::Linear(keys) => {
            let _ = write!(out, "linear {}", keys.len());
            for [t, value] in keys {
                let _ = write!(out, " {} {}", hex(*t), hex(*value));
            }
        }
    }
}

fn parse_law(tokens: &mut core::str::SplitWhitespace<'_>, line: usize) -> Result<Law, TextError> {
    match tokens.next() {
        Some("constant") => Ok(Law::Constant(next_f64(tokens, line)?)),
        Some("linear") => {
            let count = next_u32(tokens, line)? as usize;
            if count > MAX_LAW_KEYS {
                return Err(TextError::Malformed { line });
            }
            let mut keys = Vec::with_capacity(count);
            for _ in 0..count {
                keys.push([next_f64(tokens, line)?, next_f64(tokens, line)?]);
            }
            Ok(Law::Linear(keys))
        }
        _ => Err(TextError::Malformed { line }),
    }
}

/// Upper bound on section-law keys read from text, before allocation.
const MAX_LAW_KEYS: usize = 1 << 16;

fn parse_f64(token: &str, line: usize) -> Result<f64, TextError> {
    u64::from_str_radix(token, 16)
        .map(f64::from_bits)
        .map_err(|_| TextError::Malformed { line })
}

fn parse_usize(token: &str, line: usize) -> Result<usize, TextError> {
    token.parse().map_err(|_| TextError::Malformed { line })
}

fn parse_u32(token: &str, line: usize) -> Result<u32, TextError> {
    token.parse().map_err(|_| TextError::Malformed { line })
}

fn next_i32(tokens: &mut core::str::SplitWhitespace<'_>, line: usize) -> Result<i32, TextError> {
    tokens
        .next()
        .ok_or(TextError::Malformed { line })?
        .parse()
        .map_err(|_| TextError::Malformed { line })
}

fn parse_placement(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<Placement3, TextError> {
    let mut rows = [[0.0; 4]; 3];
    for row in &mut rows {
        for cell in row.iter_mut() {
            let token = tokens.next().ok_or(TextError::Malformed { line })?;
            *cell = parse_f64(token, line)?;
        }
    }
    Ok(Placement3 { rows })
}

fn parse_caps(token: &str, line: usize) -> Result<CapMode, TextError> {
    match token {
        "both" => Ok(CapMode::Both),
        "start" => Ok(CapMode::Start),
        "end" => Ok(CapMode::End),
        "none" => Ok(CapMode::None),
        _ => Err(TextError::Malformed { line }),
    }
}

/// Parses `constructive-ir-v1` text back into a validated recipe.
///
/// # Errors
///
/// Returns a typed [`TextError`]; the rebuilt recipe re-runs full builder
/// validation, so malformed geometry is caught the same way direct
/// construction catches it.
pub fn parse_recipe(text: &str) -> Result<Recipe, TextError> {
    let mut lines = Lines {
        iter: text.lines().enumerate(),
    };
    let (_, header) = lines.next()?;
    if header != HEADER {
        return Err(TextError::BadHeader);
    }
    let (line, schema) = lines.next()?;
    let schema_value = schema
        .strip_prefix("schema ")
        .ok_or(TextError::Malformed { line })?;
    if parse_u32(schema_value, line)? != crate::EVAL_SCHEMA_VERSION {
        return Err(TextError::BadHeader);
    }

    let mut builder = RecipeBuilder::new();

    // Sources and slots intern in file order, so ids are stable.
    let (line, header) = lines.next()?;
    let count = section_count(header, "sources", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let rest = entry
            .strip_prefix("source ")
            .ok_or(TextError::Malformed { line })?;
        let (_, quoted) = rest.split_once(' ').ok_or(TextError::Malformed { line })?;
        let value = unquote(quoted).ok_or(TextError::Malformed { line })?;
        builder.source_ref(&value);
    }
    let (line, header) = lines.next()?;
    let count = section_count(header, "slots", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let rest = entry
            .strip_prefix("slot ")
            .ok_or(TextError::Malformed { line })?;
        let (_, quoted) = rest.split_once(' ').ok_or(TextError::Malformed { line })?;
        let value = unquote(quoted).ok_or(TextError::Malformed { line })?;
        builder.material_slot(&value);
    }

    let (line, header) = lines.next()?;
    let count = section_count(header, "policies", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let rest = entry
            .strip_prefix("policy ")
            .ok_or(TextError::Malformed { line })?;
        let (_, quoted) = rest.split_once(' ').ok_or(TextError::Malformed { line })?;
        let value = unquote(quoted).ok_or(TextError::Malformed { line })?;
        builder.curve_policy(&value);
    }

    let (line, header) = lines.next()?;
    let count = section_count(header, "imports", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let rest = entry
            .strip_prefix("import ")
            .ok_or(TextError::Malformed { line })?;
        let mut tokens = rest.split_whitespace();
        let _index = tokens.next().ok_or(TextError::Malformed { line })?;
        expect(&mut tokens, "vertices", line)?;
        let vertex_count = next_u32(&mut tokens, line)? as usize;
        expect(&mut tokens, "faces", line)?;
        let face_count = next_u32(&mut tokens, line)? as usize;
        let mut uv1_rule = None;
        let mut uv1_default = None;
        while let Some(field) = tokens.next() {
            match field {
                "uv1_rule" if uv1_rule.is_none() => {
                    uv1_rule = Some(match next_u32(&mut tokens, line)? {
                        0 => exedra_mesh::attributes::Propagation::Unspecified,
                        1 => exedra_mesh::attributes::Propagation::Clear,
                        2 => exedra_mesh::attributes::Propagation::Copy,
                        3 => exedra_mesh::attributes::Propagation::Interpolate,
                        _ => return Err(TextError::Malformed { line }),
                    });
                }
                "uv1_dense" if uv1_default.is_none() => {
                    let mut default = [0.0; 2];
                    for value in &mut default {
                        *value = f32::from_bits(
                            u32::from_str_radix(
                                tokens.next().ok_or(TextError::Malformed { line })?,
                                16,
                            )
                            .map_err(|_| TextError::Malformed { line })?,
                        );
                    }
                    uv1_default = Some(default);
                }
                _ => return Err(TextError::Malformed { line }),
            }
        }
        let mut mesh_builder = exedra_mesh::MeshBuilder::new();
        for _ in 0..vertex_count {
            let (line, entry) = lines.next()?;
            let rest = entry
                .strip_prefix("v ")
                .ok_or(TextError::Malformed { line })?;
            let mut tokens = rest.split_whitespace();
            let mut position = [0.0_f32; 3];
            for c in &mut position {
                let token = tokens.next().ok_or(TextError::Malformed { line })?;
                *c = f32::from_bits(
                    u32::from_str_radix(token, 16).map_err(|_| TextError::Malformed { line })?,
                );
            }
            mesh_builder.push_vertex(position);
        }
        let mut loops = Vec::with_capacity(face_count);
        let mut uvs = Vec::with_capacity(face_count);
        let mut uvs1 = Vec::with_capacity(face_count);
        let mut normals = Vec::with_capacity(face_count);
        for _ in 0..face_count {
            let (line, entry) = lines.next()?;
            let rest = entry
                .strip_prefix("f ")
                .ok_or(TextError::Malformed { line })?;
            let (rest, normal_text) = rest
                .split_once(" normals ")
                .map_or((rest, None), |(rest, text)| (rest, Some(text)));
            let (rest, uv1_text) = rest
                .split_once(" uv1 ")
                .map_or((rest, None), |(rest, text)| (rest, Some(text)));
            let (rest, uv_text) = rest
                .split_once(" uv ")
                .map_or((rest, None), |(rest, text)| (rest, Some(text)));
            let indices = rest
                .split_whitespace()
                .map(|token| parse_u32(token, line))
                .collect::<Result<Vec<_>, _>>()?;
            mesh_builder
                .add_face(&indices)
                .map_err(|_| TextError::Malformed { line })?;
            uvs.push(parse_corner_channel(uv_text, indices.len(), line)?);
            uvs1.push(parse_corner_channel(uv1_text, indices.len(), line)?);
            normals.push(parse_corner_channel(normal_text, indices.len(), line)?);
            loops.push(indices);
        }
        let built = mesh_builder
            .build()
            .map_err(|_| TextError::Malformed { line })?;
        let mut mesh = built.mesh;
        crate::import_mesh::restore_corner_channels(
            &mut mesh,
            &loops,
            &uvs,
            &uvs1,
            &normals,
            uv1_rule,
            uv1_default,
        )
        .map_err(|_| TextError::Malformed { line })?;
        builder.add_import(mesh).map_err(TextError::Recipe)?;
    }

    let (line, header) = lines.next()?;
    let count = section_count(header, "profiles", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let holes: usize = entry
            .strip_prefix("profile ")
            .and_then(|rest| rest.split_once(" holes "))
            .map(|(_, holes)| holes)
            .ok_or(TextError::Malformed { line })
            .and_then(|token| parse_usize(token, line))?;
        let outer = parse_loop(&mut lines)?;
        let hole_loops = (0..holes)
            .map(|_| parse_loop(&mut lines))
            .collect::<Result<Vec<_>, _>>()?;
        let profile = Profile2::new(outer, hole_loops).map_err(TextError::Profile)?;
        builder.add_profile(profile);
    }

    let (line, header) = lines.next()?;
    let count = section_count(header, "nodes", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let rest = entry
            .strip_prefix("node ")
            .ok_or(TextError::Malformed { line })?;
        let (_, body) = rest.split_once(' ').ok_or(TextError::Malformed { line })?;
        parse_node(&mut builder, body, line)?;
    }

    let (line, root_line) = lines.next()?;
    let root = root_line
        .strip_prefix("root ")
        .ok_or(TextError::Malformed { line })
        .and_then(|token| parse_u32(token, line))?;
    builder.finish(NodeId(root)).map_err(TextError::Recipe)
}

fn unquote(token: &str) -> Option<String> {
    // Values are Rust debug-quoted; parse the escape-free common case and
    // reject anything else (source refs are opaque identifiers by
    // convention).
    let inner = token.strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains('\\') {
        return None;
    }
    Some(String::from(inner))
}

fn section_count(header: &str, name: &str, line: usize) -> Result<usize, TextError> {
    header
        .strip_prefix(name)
        .map(str::trim)
        .ok_or(TextError::Malformed { line })
        .and_then(|token| parse_usize(token, line))
}

fn parse_loop(lines: &mut Lines<'_>) -> Result<Loop2, TextError> {
    let (line, header) = lines.next()?;
    let count = header
        .strip_prefix("loop ")
        .ok_or(TextError::Malformed { line })
        .and_then(|token| parse_usize(token, line))?;
    let mut segs = Vec::with_capacity(count);
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let mut tokens = entry.split_whitespace();
        let kind = tokens.next().ok_or(TextError::Malformed { line })?;
        let seg = parse_seg(kind, &mut tokens, line)?;
        expect(&mut tokens, "tag", line)?;
        let tag = tokens.next().ok_or(TextError::Malformed { line })?;
        let seg = if tag == "-" {
            seg
        } else {
            seg.tagged(SegTag(parse_u32(tag, line)?))
        };
        segs.push(seg);
    }
    Loop2::new(segs).map_err(TextError::Profile)
}

fn parse_seg(
    kind: &str,
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<Seg2, TextError> {
    Ok(match kind {
        "line" => {
            expect(tokens, "to", line)?;
            let to = parse_point(tokens, line)?;
            Seg2::line(to)
        }
        "arc" => {
            let bulge = next_f64(tokens, line)?;
            expect(tokens, "to", line)?;
            let to = parse_point(tokens, line)?;
            Seg2::arc(to, bulge)
        }
        "cubic" => {
            let c1 = parse_point(tokens, line)?;
            let c2 = parse_point(tokens, line)?;
            expect(tokens, "to", line)?;
            let to = parse_point(tokens, line)?;
            Seg2::cubic(to, c1, c2)
        }
        "policy" => {
            let policy = crate::ir::PolicyId(next_u32(tokens, line)?);
            let inner_kind = tokens.next().ok_or(TextError::Malformed { line })?;
            let inner = parse_seg(inner_kind, tokens, line)?;
            Seg2::policy(inner.to, policy, inner.kind)
        }
        _ => return Err(TextError::Malformed { line }),
    })
}

fn expect(
    tokens: &mut core::str::SplitWhitespace<'_>,
    expected: &str,
    line: usize,
) -> Result<(), TextError> {
    (tokens.next() == Some(expected))
        .then_some(())
        .ok_or(TextError::Malformed { line })
}

fn next_f64(tokens: &mut core::str::SplitWhitespace<'_>, line: usize) -> Result<f64, TextError> {
    let token = tokens.next().ok_or(TextError::Malformed { line })?;
    parse_f64(token, line)
}

fn next_f64s<const N: usize>(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<[f64; N], TextError> {
    let mut values = [0.0; N];
    for value in &mut values {
        *value = next_f64(tokens, line)?;
    }
    Ok(values)
}

/// Reads a field written by [`put_implicit`], rebuilding it through its
/// constructors.
fn parse_implicit(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<exedra_implicit::Expr, TextError> {
    use exedra_implicit as im;
    use exedra_implicit::Expr;
    let refused = |error| TextError::Implicit { line, error };
    let name = tokens.next().ok_or(TextError::Malformed { line })?;
    Ok(match name {
        "sphere" => {
            let center = next_f64s::<3>(tokens, line)?;
            let [radius] = next_f64s::<1>(tokens, line)?;
            Expr::from(im::Sphere::new(center, radius).map_err(refused)?)
        }
        "cuboid" => {
            let center = next_f64s::<3>(tokens, line)?;
            let half = next_f64s::<3>(tokens, line)?;
            Expr::from(im::Cuboid::new(center, half).map_err(refused)?)
        }
        "cylinder" => {
            let center = next_f64s::<3>(tokens, line)?;
            let axis = next_f64s::<3>(tokens, line)?;
            let [radius, half] = next_f64s::<2>(tokens, line)?;
            Expr::from(im::Cylinder::new(center, axis, radius, half).map_err(refused)?)
        }
        "torus" => {
            let center = next_f64s::<3>(tokens, line)?;
            let axis = next_f64s::<3>(tokens, line)?;
            let [major, minor] = next_f64s::<2>(tokens, line)?;
            Expr::from(im::Torus::new(center, axis, major, minor).map_err(refused)?)
        }
        "half_space" => {
            let point = next_f64s::<3>(tokens, line)?;
            let normal = next_f64s::<3>(tokens, line)?;
            Expr::from(im::HalfSpace::new(point, normal).map_err(refused)?)
        }
        "capsule" => {
            let a = next_f64s::<3>(tokens, line)?;
            let b = next_f64s::<3>(tokens, line)?;
            let [radius] = next_f64s::<1>(tokens, line)?;
            Expr::from(im::Capsule::new(a, b, radius).map_err(refused)?)
        }
        "round_cone" => {
            let a = next_f64s::<3>(tokens, line)?;
            let b = next_f64s::<3>(tokens, line)?;
            let [ra, rb] = next_f64s::<2>(tokens, line)?;
            Expr::from(im::RoundCone::new(a, b, ra, rb).map_err(refused)?)
        }
        "extrude" => {
            let profile = parse_implicit_profile(tokens, line)?;
            let [half] = next_f64s::<1>(tokens, line)?;
            Expr::from(im::Extrude::new(profile, half).map_err(refused)?)
        }
        "revolve" => Expr::from(im::Revolve::new(parse_implicit_profile(tokens, line)?)),
        "union" | "intersection" | "difference" => {
            let left = parse_implicit(tokens, line)?;
            let right = parse_implicit(tokens, line)?;
            match name {
                "union" => Expr::from(im::Union::new(left, right)),
                "intersection" => Expr::from(im::Intersection::new(left, right)),
                _ => Expr::from(im::Difference::new(left, right)),
            }
        }
        "smooth_union" => {
            let left = parse_implicit(tokens, line)?;
            let right = parse_implicit(tokens, line)?;
            let [radius] = next_f64s::<1>(tokens, line)?;
            Expr::from(im::SmoothUnion::new(left, right, radius).map_err(refused)?)
        }
        "blend" => {
            let count = next_u32(tokens, line)?;
            let parts = (0..count)
                .map(|_| parse_implicit(tokens, line))
                .collect::<Result<Vec<_>, _>>()?;
            let [radius] = next_f64s::<1>(tokens, line)?;
            Expr::from(im::SmoothUnionN::new(parts, radius).map_err(refused)?)
        }
        "translate" => {
            let field = parse_implicit(tokens, line)?;
            let offset = next_f64s::<3>(tokens, line)?;
            Expr::from(im::Translate::new(field, offset).map_err(refused)?)
        }
        "scale" => {
            let field = parse_implicit(tokens, line)?;
            let [factor] = next_f64s::<1>(tokens, line)?;
            Expr::from(im::Scale::new(field, factor).map_err(refused)?)
        }
        "placed" => {
            let field = parse_implicit(tokens, line)?;
            let placement = parse_placement(tokens, line)?;
            Expr::from(im::Placed::new(field, placement).map_err(refused)?)
        }
        "tagged" => {
            let field = parse_implicit(tokens, line)?;
            let region = next_u32(tokens, line)?;
            Expr::from(im::Tagged::new(field, region))
        }
        _ => return Err(TextError::Malformed { line }),
    })
}

fn parse_implicit_profile(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<exedra_implicit::ProfileExpr, TextError> {
    use exedra_implicit as im;
    use exedra_implicit::ProfileExpr;
    let refused = |error| TextError::Implicit { line, error };
    Ok(match tokens.next().ok_or(TextError::Malformed { line })? {
        "circle" => {
            let center = next_f64s::<2>(tokens, line)?;
            let [radius] = next_f64s::<1>(tokens, line)?;
            ProfileExpr::from(im::Circle::new(center, radius).map_err(refused)?)
        }
        "rectangle" => {
            let center = next_f64s::<2>(tokens, line)?;
            let half = next_f64s::<2>(tokens, line)?;
            ProfileExpr::from(im::Rectangle::new(center, half).map_err(refused)?)
        }
        "half_plane" => {
            let point = next_f64s::<2>(tokens, line)?;
            let normal = next_f64s::<2>(tokens, line)?;
            ProfileExpr::from(im::HalfPlane::new(point, normal).map_err(refused)?)
        }
        _ => return Err(TextError::Malformed { line }),
    })
}

fn next_u32(tokens: &mut core::str::SplitWhitespace<'_>, line: usize) -> Result<u32, TextError> {
    let token = tokens.next().ok_or(TextError::Malformed { line })?;
    parse_u32(token, line)
}

fn parse_point(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<Point, TextError> {
    let x = next_f64(tokens, line)?;
    let y = next_f64(tokens, line)?;
    Ok(Point::new(x, y))
}

fn parse_plane(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<Plane3, TextError> {
    Ok(Plane3 {
        normal: [
            next_f64(tokens, line)?,
            next_f64(tokens, line)?,
            next_f64(tokens, line)?,
        ],
        distance: next_f64(tokens, line)?,
    })
}

fn parse_node(builder: &mut RecipeBuilder, body: &str, line: usize) -> Result<(), TextError> {
    // Split off the trailing `source X material Y issue Z` suffix.
    let (body, issue) = split_suffix(body, "issue", line)?;
    let (body, material) = split_suffix(&body, "material", line)?;
    let (body, source) = split_suffix(&body, "source", line)?;
    let source = parse_opt_index(&source, line)?.map(crate::ir::SourceId);
    let material = parse_opt_index(&material, line)?.map(crate::ir::SlotId);
    let issue = parse_opt_index(&issue, line)?.map(crate::ir::SourceId);
    let mut surface_chart = None;

    let mut tokens = body.split_whitespace();
    let mut kind_name = tokens.next().ok_or(TextError::Malformed { line })?;
    if kind_name == "charted" {
        let metric = tokens.next().ok_or(TextError::Malformed { line })?;
        let mut reference_radius = 1.0;
        let mut reference_section = 0;
        let mut rest_length = 1.0;
        let mut stations = crate::chart::LoftStations::SectionIndex;
        match metric {
            "revolve" => {
                expect(&mut tokens, "radius", line)?;
                reference_radius = next_f64(&mut tokens, line)?;
            }
            "loft" => {
                expect(&mut tokens, "reference", line)?;
                reference_section = next_u32(&mut tokens, line)?;
                expect(&mut tokens, "rest_length", line)?;
                rest_length = next_f64(&mut tokens, line)?;
                if tokens.clone().next() == Some("stations") {
                    tokens.next();
                    expect(&mut tokens, "datum_distance", line)?;
                    stations = crate::chart::LoftStations::DatumDistance;
                }
            }
            "extrude" | "sweep" => {}
            _ => return Err(TextError::Malformed { line }),
        }
        let mut transform = |label| -> Result<ChartTransform, TextError> {
            expect(&mut tokens, label, line)?;
            Ok(ChartTransform {
                matrix: [
                    [next_f64(&mut tokens, line)?, next_f64(&mut tokens, line)?],
                    [next_f64(&mut tokens, line)?, next_f64(&mut tokens, line)?],
                ],
                offset: [next_f64(&mut tokens, line)?, next_f64(&mut tokens, line)?],
            })
        };
        let wall = transform("wall")?;
        let caps = transform("caps")?;
        surface_chart = Some(match metric {
            "extrude" => SurfaceChart::Extrude { wall, caps },
            "revolve" => SurfaceChart::Revolve {
                reference_radius,
                wall,
                caps,
            },
            "loft" => SurfaceChart::Loft {
                reference_section,
                rest_length,
                stations,
                wall,
                caps,
            },
            "sweep" => SurfaceChart::Sweep { wall, caps },
            _ => unreachable!("metric checked above"),
        });
        expect(&mut tokens, "operation", line)?;
        kind_name = tokens.next().ok_or(TextError::Malformed { line })?;
    }
    let mut section = SectionLaw::IDENTITY;
    if kind_name == "shaped" {
        expect(&mut tokens, "scale", line)?;
        section.scale = parse_law(&mut tokens, line)?;
        expect(&mut tokens, "twist", line)?;
        section.twist = parse_law(&mut tokens, line)?;
        kind_name = tokens.next().ok_or(TextError::Malformed { line })?;
        if kind_name == "turns" {
            section.turns = next_i32(&mut tokens, line)?;
            kind_name = tokens.next().ok_or(TextError::Malformed { line })?;
        }
        if !matches!(
            kind_name,
            "sweep" | "mitered_sweep" | "mitered_path_sweep" | "curved_sweep" | "curved_path_sweep"
        ) {
            return Err(TextError::Malformed { line });
        }
    }
    let kind = match kind_name {
        "on_workplane" => {
            use crate::workplane::{SurfaceSelector, WorkplaneAttachment};
            expect(&mut tokens, "support", line)?;
            let support = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "surface", line)?;
            let surface = match tokens.next() {
                Some(kind @ ("source_start_cap" | "source_end_cap")) => {
                    let token = tokens
                        .next()
                        .and_then(|s| s.strip_prefix('x'))
                        .ok_or(TextError::Malformed { line })?;
                    if token.len() % 2 != 0 || !token.is_ascii() {
                        return Err(TextError::Malformed { line });
                    }
                    let bytes = (0..token.len())
                        .step_by(2)
                        .map(|i| {
                            u8::from_str_radix(&token[i..i + 2], 16)
                                .map_err(|_| TextError::Malformed { line })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let source =
                        String::from_utf8(bytes).map_err(|_| TextError::Malformed { line })?;
                    if kind == "source_start_cap" {
                        SurfaceSelector::SourceStartCap(source)
                    } else {
                        SurfaceSelector::SourceEndCap(source)
                    }
                }
                Some("start_cap") => SurfaceSelector::StartCap,
                Some("end_cap") => SurfaceSelector::EndCap,
                Some("region") => SurfaceSelector::Region(next_u32(&mut tokens, line)?),
                Some("operand_region") => SurfaceSelector::OperandRegion(OperandRegion {
                    operand: u16::try_from(next_u32(&mut tokens, line)?)
                        .map_err(|_| TextError::Malformed { line })?,
                    region: next_u32(&mut tokens, line)?,
                }),
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "anchor", line)?;
            let anchor = [
                next_f64(&mut tokens, line)?,
                next_f64(&mut tokens, line)?,
                next_f64(&mut tokens, line)?,
            ];
            expect(&mut tokens, "projection", line)?;
            let projection = [
                next_f64(&mut tokens, line)?,
                next_f64(&mut tokens, line)?,
                next_f64(&mut tokens, line)?,
            ];
            expect(&mut tokens, "x", line)?;
            let x_direction = [
                next_f64(&mut tokens, line)?,
                next_f64(&mut tokens, line)?,
                next_f64(&mut tokens, line)?,
            ];
            NodeKind::OnWorkplane {
                support,
                child,
                attachment: WorkplaneAttachment {
                    surface,
                    anchor,
                    projection,
                    x_direction,
                },
            }
        }
        "plane_cut" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "plane", line)?;
            let plane = parse_plane(&mut tokens, line)?;
            expect(&mut tokens, "side", line)?;
            let side = match tokens.next() {
                Some("negative") => crate::ir::PlaneSide::Negative,
                Some("positive") => crate::ir::PlaneSide::Positive,
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "cap_region", line)?;
            let region = next_u32(&mut tokens, line)?;
            expect(&mut tokens, "cap_material", line)?;
            let material =
                parse_opt_index(tokens.next().ok_or(TextError::Malformed { line })?, line)?
                    .map(crate::ir::SlotId);
            NodeKind::PlaneCut {
                child,
                plane,
                side,
                cap: crate::section::CutCap { region, material },
            }
        }
        "extrude_to_plane" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "plane", line)?;
            let plane = parse_plane(&mut tokens, line)?;
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::ExtrudeToPlane {
                profile,
                placement,
                plane,
            }
        }
        "extrude" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "height", line)?;
            let height = next_f64(&mut tokens, line)?;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::Extrude {
                profile,
                placement,
                height,
                caps,
            }
        }
        "revolve" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "sweep", line)?;
            let sweep = next_f64(&mut tokens, line)?;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::Revolve {
                profile,
                placement,
                sweep,
                caps,
            }
        }
        "loft" => {
            let policy = match tokens.next() {
                Some("ruled") => LoftPolicy::Ruled,
                Some("smooth") => LoftPolicy::Smooth,
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "sections", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            let mut sections = Vec::with_capacity(count);
            for _ in 0..count {
                expect(&mut tokens, "section", line)?;
                let profile = ProfileId(next_u32(&mut tokens, line)?);
                expect(&mut tokens, "placement", line)?;
                let placement = parse_placement(&mut tokens, line)?;
                let mut section = LoftSection::new(placement, profile);
                if tokens.clone().next() == Some("section_source") {
                    tokens.next();
                    section.source = Some(crate::ir::SourceId(next_u32(&mut tokens, line)?));
                }
                sections.push(section);
            }
            NodeKind::Loft {
                sections,
                policy,
                caps,
            }
        }
        "curved_sweep" | "curved_path_sweep" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "start", line)?;
            let start = parse_vector3(&mut tokens, line)?;
            expect(&mut tokens, "section_x", line)?;
            let section_x = parse_vector3(&mut tokens, line)?;
            let (section_origin, closure, joins) = if kind_name == "curved_path_sweep" {
                expect(&mut tokens, "section_origin", line)?;
                let origin = [next_f64(&mut tokens, line)?, next_f64(&mut tokens, line)?];
                expect(&mut tokens, "closure", line)?;
                let closure = match tokens.next() {
                    Some("open") => PathClosure::Open,
                    Some("closed_planar") => PathClosure::ClosedPlanar {
                        normal: parse_vector3(&mut tokens, line)?,
                    },
                    _ => return Err(TextError::Malformed { line }),
                };
                expect(&mut tokens, "joins", line)?;
                let joins = match tokens.next() {
                    Some("smooth") => PathJoin::Smooth,
                    Some("miter") => PathJoin::Miter {
                        limit: next_f64(&mut tokens, line)?,
                    },
                    _ => return Err(TextError::Malformed { line }),
                };
                (origin, closure, joins)
            } else {
                ([0.0; 2], PathClosure::Open, PathJoin::Smooth)
            };
            expect(&mut tokens, "segments", line)?;
            let count = next_u32(&mut tokens, line)?;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            let mut segments = Vec::new();
            for _ in 0..count {
                let segment = match tokens.next() {
                    Some("line") => crate::path::PathSegment3::Line {
                        to: parse_vector3(&mut tokens, line)?,
                    },
                    Some("arc") => crate::path::PathSegment3::Arc {
                        axis_origin: parse_vector3(&mut tokens, line)?,
                        axis: parse_vector3(&mut tokens, line)?,
                        sweep: next_f64(&mut tokens, line)?,
                    },
                    Some("cubic") => crate::path::PathSegment3::Cubic {
                        control1: parse_vector3(&mut tokens, line)?,
                        control2: parse_vector3(&mut tokens, line)?,
                        to: parse_vector3(&mut tokens, line)?,
                    },
                    _ => return Err(TextError::Malformed { line }),
                };
                segments.push(segment);
            }
            NodeKind::Sweep {
                profile,
                section: core::mem::take(&mut section),
                path: Path3::Curves {
                    start,
                    segments,
                    section_x,
                    section_origin,
                    closure,
                    joins,
                },
                caps,
            }
        }
        "sweep" | "mitered_sweep" | "mitered_path_sweep" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            let (section_origin, closure) = if kind_name == "mitered_path_sweep" {
                expect(&mut tokens, "section_origin", line)?;
                let origin = [next_f64(&mut tokens, line)?, next_f64(&mut tokens, line)?];
                expect(&mut tokens, "closure", line)?;
                let closure = match tokens.next() {
                    Some("open") => PathClosure::Open,
                    Some("closed_planar") => PathClosure::ClosedPlanar {
                        normal: parse_vector3(&mut tokens, line)?,
                    },
                    _ => return Err(TextError::Malformed { line }),
                };
                (origin, closure)
            } else {
                ([0.0; 2], PathClosure::Open)
            };
            let orientation = match tokens.next() {
                Some("frame") if kind_name == "sweep" => {
                    expect(&mut tokens, "rmf", line)?;
                    None
                }
                Some("section_x") if kind_name != "sweep" => {
                    let x = [
                        next_f64(&mut tokens, line)?,
                        next_f64(&mut tokens, line)?,
                        next_f64(&mut tokens, line)?,
                    ];
                    expect(&mut tokens, "miter_limit", line)?;
                    Some((x, next_f64(&mut tokens, line)?))
                }
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "points", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            let mut points = Vec::with_capacity(count);
            for _ in 0..count {
                points.push([
                    next_f64(&mut tokens, line)?,
                    next_f64(&mut tokens, line)?,
                    next_f64(&mut tokens, line)?,
                ]);
            }
            NodeKind::Sweep {
                profile,
                section: core::mem::take(&mut section),
                path: match orientation {
                    None => Path3::Polyline {
                        points,
                        frame: FramePolicy::RotationMinimizing,
                    },
                    Some((section_x, miter_limit)) => Path3::MiteredPolyline {
                        points,
                        section_x,
                        section_origin,
                        closure,
                        miter_limit,
                    },
                },
                caps,
            }
        }
        "planar_face" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::PlanarFace { profile, placement }
        }
        "primitive" => {
            let which = tokens.next().ok_or(TextError::Malformed { line })?;
            let spec = match which {
                "box" => PrimitiveSpec::Box {
                    size: [
                        next_f64(&mut tokens, line)?,
                        next_f64(&mut tokens, line)?,
                        next_f64(&mut tokens, line)?,
                    ],
                },
                "cylinder" => PrimitiveSpec::Cylinder {
                    radius: next_f64(&mut tokens, line)?,
                    height: next_f64(&mut tokens, line)?,
                    segments: next_u32(&mut tokens, line)?,
                },
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::Primitive { spec, placement }
        }
        "csg" => {
            let op = match tokens.next().ok_or(TextError::Malformed { line })? {
                "union" => CsgOp::Union,
                "difference" => CsgOp::Difference,
                "intersection" => CsgOp::Intersection,
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "operands", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            let operands = (0..count)
                .map(|_| Ok(NodeId(next_u32(&mut tokens, line)?)))
                .collect::<Result<Vec<_>, TextError>>()?;
            NodeKind::Csg { op, operands }
        }
        "uv_map" => {
            use crate::ir::{BoxUv, UvFrame, UvMapping, UvProjection, UvTransform};
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            let options = if tokens.clone().next() == Some("options") {
                tokens.next();
                let mut flags = [false; 4];
                for flag in &mut flags {
                    *flag = match next_u32(&mut tokens, line)? {
                        0 => false,
                        1 => true,
                        _ => return Err(TextError::Malformed { line }),
                    };
                }
                crate::ir::UvMappingOptions {
                    source: if flags[0] {
                        crate::ir::UvSet::Secondary
                    } else {
                        crate::ir::UvSet::Primary
                    },
                    target: if flags[1] {
                        crate::ir::UvSet::Secondary
                    } else {
                        crate::ir::UvSet::Primary
                    },
                    seams: if flags[2] {
                        crate::ir::UvSeamPolicy::Continuous
                    } else {
                        crate::ir::UvSeamPolicy::Preserve
                    },
                    poles: if flags[3] {
                        crate::ir::UvPolePolicy::FaceAverage
                    } else {
                        crate::ir::UvPolePolicy::Refuse
                    },
                }
            } else {
                crate::ir::UvMappingOptions::default()
            };
            let mode = tokens.next().ok_or(TextError::Malformed { line })?;
            let projection = match mode {
                "face" => UvProjection::Face,
                "cylindrical" | "spherical" | "toroidal" | "perspective" => {
                    let frame = UvFrame {
                        origin: parse_vector3(&mut tokens, line)?,
                        u: parse_vector3(&mut tokens, line)?,
                        v: parse_vector3(&mut tokens, line)?,
                    };
                    match mode {
                        "cylindrical" => UvProjection::Cylindrical(frame),
                        "spherical" => UvProjection::Spherical(frame),
                        "toroidal" => UvProjection::Toroidal {
                            frame,
                            major_radius: next_f64(&mut tokens, line)?,
                        },
                        _ => UvProjection::Perspective(crate::ir::PerspectiveUv {
                            frame,
                            focal: [next_f64(&mut tokens, line)?, next_f64(&mut tokens, line)?],
                            near: next_f64(&mut tokens, line)?,
                        }),
                    }
                }
                "existing" => UvProjection::Existing,
                "planar" => UvProjection::Planar(UvFrame {
                    origin: parse_vector3(&mut tokens, line)?,
                    u: parse_vector3(&mut tokens, line)?,
                    v: parse_vector3(&mut tokens, line)?,
                }),
                "box" => {
                    let mut flags = [false; 9];
                    for flag in &mut flags {
                        *flag = match next_u32(&mut tokens, line)? {
                            0 => false,
                            1 => true,
                            _ => return Err(TextError::Malformed { line }),
                        };
                    }
                    UvProjection::Box(BoxUv {
                        swap_axes: [flags[0], flags[1], flags[2]],
                        flip_u: [flags[3], flags[4], flags[5]],
                        flip_v: [flags[6], flags[7], flags[8]],
                    })
                }
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "transform", line)?;
            let mut coefficients = [0.0; 6];
            for value in &mut coefficients {
                *value = next_f64(&mut tokens, line)?;
            }
            NodeKind::UvMap {
                options,
                child,
                mapping: UvMapping {
                    projection,
                    transform: UvTransform {
                        matrix: [
                            [coefficients[0], coefficients[1]],
                            [coefficients[2], coefficients[3]],
                        ],
                        offset: [coefficients[4], coefficients[5]],
                    },
                },
            }
        }
        "transform" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "placement", line)?;
            let xf = parse_placement(&mut tokens, line)?;
            NodeKind::Transform { child, xf }
        }
        "mirror" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "plane", line)?;
            let plane = parse_plane(&mut tokens, line)?;
            NodeKind::Mirror { child, plane }
        }
        "instance" => {
            expect(&mut tokens, "of", line)?;
            let of = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::Instance { of, placement }
        }
        "group" => {
            expect(&mut tokens, "children", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            let children = (0..count)
                .map(|_| Ok(NodeId(next_u32(&mut tokens, line)?)))
                .collect::<Result<Vec<_>, TextError>>()?;
            NodeKind::Group { children }
        }
        "mesh_import" => {
            let import = crate::ir::ImportId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::MeshImport { import, placement }
        }
        "edge_finish" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            let selection = match tokens.next() {
                Some("sharp") => EdgeSelection::SharpEdges,
                Some("boundaries") => {
                    let count = next_u32(&mut tokens, line)?;
                    let pairs = (0..count)
                        .map(|_| Ok([next_u32(&mut tokens, line)?, next_u32(&mut tokens, line)?]))
                        .collect::<Result<Vec<_>, TextError>>()?;
                    EdgeSelection::RegionBoundaries(pairs)
                }
                Some("operand_boundaries") => {
                    let count = next_u32(&mut tokens, line)?;
                    let mut source = || {
                        Ok(OperandRegion {
                            operand: u16::try_from(next_u32(&mut tokens, line)?)
                                .map_err(|_| TextError::Malformed { line })?,
                            region: next_u32(&mut tokens, line)?,
                        })
                    };
                    let pairs = (0..count)
                        .map(|_| Ok([source()?, source()?]))
                        .collect::<Result<Vec<_>, TextError>>()?;
                    EdgeSelection::OperandBoundaries(pairs)
                }
                _ => return Err(TextError::Malformed { line }),
            };
            let kind_name = tokens.next().ok_or(TextError::Malformed { line })?;
            let offset = next_f64(&mut tokens, line)?;
            let kind = match kind_name {
                "fillet" => RoundKind::Fillet { radius: offset },
                "chamfer" => RoundKind::Chamfer { setback: offset },
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "segments", line)?;
            let value = tokens.next().ok_or(TextError::Malformed { line })?;
            let segments = if value == "-" {
                None
            } else {
                Some(parse_u32(value, line)?)
            };
            expect(&mut tokens, "tolerance", line)?;
            let chord_tolerance = next_f64(&mut tokens, line)?;
            expect(&mut tokens, "sharpness", line)?;
            let sharpness_threshold = f32::from_bits(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "region", line)?;
            let value = tokens.next().ok_or(TextError::Malformed { line })?;
            let region = if value == "-" {
                None
            } else {
                Some(parse_u32(value, line)?)
            };
            expect(&mut tokens, "planar", line)?;
            let max_planar_deviation = next_f64(&mut tokens, line)?;
            expect(&mut tokens, "turn", line)?;
            let max_tangent_turn = next_f64(&mut tokens, line)?;
            NodeKind::EdgeFinish {
                child,
                selection,
                policy: RoundPolicy {
                    kind,
                    segments,
                    chord_tolerance,
                    sharpness_threshold,
                    region,
                    max_planar_deviation,
                    max_tangent_turn,
                },
            }
        }
        "stretch_vertices" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "steps", line)?;
            let count = next_u32(&mut tokens, line)?;
            let mut steps = Vec::new();
            for _ in 0..count {
                expect(&mut tokens, "plane", line)?;
                let plane = parse_plane(&mut tokens, line)?;
                expect(&mut tokens, "length", line)?;
                steps.push(VertexStretchStep {
                    plane,
                    length: next_f64(&mut tokens, line)?,
                });
            }
            NodeKind::StretchVertices { child, steps }
        }
        "stretch" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "plane", line)?;
            let plane = parse_plane(&mut tokens, line)?;
            expect(&mut tokens, "length", line)?;
            let length = next_f64(&mut tokens, line)?;
            NodeKind::Stretch {
                child,
                plane,
                length,
            }
        }
        "grid_surface" => {
            expect(&mut tokens, "rows", line)?;
            let rows = next_u32(&mut tokens, line)?;
            expect(&mut tokens, "cols", line)?;
            let cols = next_u32(&mut tokens, line)?;
            expect(&mut tokens, "close_u", line)?;
            let close_u = next_u32(&mut tokens, line)? != 0;
            expect(&mut tokens, "close_w", line)?;
            let close_w = next_u32(&mut tokens, line)? != 0;
            expect(&mut tokens, "thickness", line)?;
            let thickness_token = tokens.next().ok_or(TextError::Malformed { line })?;
            let thickness = if thickness_token == "-" {
                None
            } else {
                Some(parse_f64(thickness_token, line)?)
            };
            expect(&mut tokens, "points", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            let points = (0..count)
                .map(|_| {
                    Ok([
                        next_f64(&mut tokens, line)?,
                        next_f64(&mut tokens, line)?,
                        next_f64(&mut tokens, line)?,
                    ])
                })
                .collect::<Result<Vec<_>, TextError>>()?;
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::GridSurface {
                points,
                rows,
                cols,
                close_u,
                close_w,
                thickness,
                placement,
            }
        }
        "implicit" => {
            expect(&mut tokens, "bounds", line)?;
            let first = tokens.next().ok_or(TextError::Malformed { line })?;
            let bounds = if first == "-" {
                None
            } else {
                let mut corner = [parse_f64(first, line)?, 0.0, 0.0];
                corner[1] = next_f64(&mut tokens, line)?;
                corner[2] = next_f64(&mut tokens, line)?;
                let far = next_f64s::<3>(&mut tokens, line)?;
                Some(exedra_implicit::Aabb::new(corner, far).ok_or(TextError::Malformed { line })?)
            };
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            expect(&mut tokens, "field", line)?;
            let field = parse_implicit(&mut tokens, line)?;
            NodeKind::Implicit {
                field,
                bounds,
                placement,
            }
        }
        _ => return Err(TextError::Malformed { line }),
    };
    builder
        .add_node(Node {
            surface_chart,
            kind,
            source,
            material,
            issue,
        })
        .map_err(TextError::Recipe)?;
    Ok(())
}

/// Splits `... <keyword> <token>` off the end of a node body.
fn split_suffix(body: &str, keyword: &str, line: usize) -> Result<(String, String), TextError> {
    let position = body
        .rfind(&format!(" {keyword} "))
        .ok_or(TextError::Malformed { line })?;
    let value = body[position + keyword.len() + 2..].trim();
    Ok((String::from(&body[..position]), String::from(value)))
}

fn parse_opt_index(token: &str, line: usize) -> Result<Option<u32>, TextError> {
    if token == "-" {
        Ok(None)
    } else {
        parse_u32(token, line).map(Some)
    }
}

/// Test-only fixture shared with the interchange tests.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::builders;
    use alloc::vec;

    /// A tiny valid mesh for import fixtures: one triangle... actually a
    /// closed tetrahedron so deep validation passes.
    pub(crate) fn tetrahedron() -> exedra_mesh::Mesh {
        let mut mb = exedra_mesh::MeshBuilder::new();
        mb.push_vertex([0.0, 0.0, 0.0]);
        mb.push_vertex([1.0, 0.0, 0.0]);
        mb.push_vertex([0.0, 1.0, 0.0]);
        mb.push_vertex([0.0, 0.0, 1.0]);
        mb.add_face(&[0, 2, 1]).expect("base");
        mb.add_face(&[0, 1, 3]).expect("side");
        mb.add_face(&[1, 2, 3]).expect("side");
        mb.add_face(&[2, 0, 3]).expect("side");
        mb.build().expect("valid tetrahedron").mesh
    }

    /// A recipe exercising every node kind and segment kind.
    pub(crate) fn full_coverage_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let rect = b.add_profile(builders::rect_from_corner(2.0, 1.0).expect("rect"));
        let rounded =
            b.add_profile(builders::rounded_rect_from_corner(3.0, 2.0, 0.25).expect("rounded"));
        let ring = b.add_profile(builders::ring(1.0, 0.5).expect("ring"));
        let src = b.source_ref("text:demo");
        let slot = b.material_slot("front");

        let extrude = b
            .add_node(
                Node::new(NodeKind::Extrude {
                    profile: rect,
                    placement: Placement3::translate(0.5, 0.25, 0.0),
                    height: 2.0,
                    caps: CapMode::Both,
                })
                .with_source(src)
                .with_material(slot),
            )
            .expect("valid");
        let revolve = b
            .add_node(NodeKind::Revolve {
                profile: ring,
                placement: Placement3::translate(5.0, 0.0, 0.0),
                sweep: core::f64::consts::FRAC_PI_2,
                caps: CapMode::Start,
            })
            .expect("valid");
        let loft = b
            .add_node(NodeKind::Loft {
                sections: vec![
                    LoftSection::new(Placement3::IDENTITY, rect),
                    LoftSection::new(Placement3::translate(0.0, 0.0, 2.0), rect),
                ],
                policy: LoftPolicy::Ruled,
                caps: CapMode::None,
            })
            .expect("valid");
        let sweep = b
            .add_node(NodeKind::Sweep {
                section: SectionLaw::IDENTITY,
                profile: rounded,
                path: Path3::Polyline {
                    points: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [1.0, 0.0, 3.0]],
                    frame: FramePolicy::RotationMinimizing,
                },
                caps: CapMode::End,
            })
            .expect("valid");
        let face = b
            .add_node(NodeKind::PlanarFace {
                profile: rect,
                placement: Placement3::IDENTITY,
            })
            .expect("valid");
        let primitive = b
            .add_node(NodeKind::Primitive {
                spec: PrimitiveSpec::Cylinder {
                    radius: 0.5,
                    height: 2.0,
                    segments: 16,
                },
                placement: Placement3::IDENTITY,
            })
            .expect("valid");
        let csg = b
            .add_node(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![extrude, primitive],
            })
            .expect("valid");
        let transform = b
            .add_node(NodeKind::Transform {
                child: csg,
                xf: Placement3::rotate_z_then_translate(0.3, 1.0, 2.0, 3.0),
            })
            .expect("valid");
        let mirror = b
            .add_node(NodeKind::Mirror {
                child: revolve,
                plane: Plane3 {
                    normal: [0.0, 1.0, 0.0],
                    distance: 0.5,
                },
            })
            .expect("valid");
        let instance = b
            .add_node(NodeKind::Instance {
                of: loft,
                placement: Placement3::translate(10.0, 0.0, 0.0),
            })
            .expect("valid");
        let stretch = b
            .add_node(NodeKind::Stretch {
                child: sweep,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 1.0,
                },
                length: 0.5,
            })
            .expect("valid");
        let import = b.add_import(tetrahedron()).expect("valid import");
        let imported = b
            .add_node(NodeKind::MeshImport {
                import,
                placement: Placement3::translate(20.0, 0.0, 0.0),
            })
            .expect("valid");
        let grid = b
            .add_node(NodeKind::GridSurface {
                points: vec![
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.1],
                    [2.0, 0.0, 0.0],
                    [0.0, 1.0, 0.2],
                    [1.0, 1.0, 0.4],
                    [2.0, 1.0, 0.2],
                ],
                rows: 2,
                cols: 3,
                close_u: false,
                close_w: false,
                thickness: Some(0.125),
                placement: Placement3::translate(30.0, 0.0, 0.0),
            })
            .expect("valid");
        let implicit = b
            .add_node(NodeKind::Implicit {
                field: every_implicit_variant(),
                bounds: Some(exedra_implicit::Aabb::new([-2.0; 3], [2.0; 3]).expect("bounds")),
                placement: Placement3::rotate_z_then_translate(0.3, 40.0, 0.0, 0.0),
            })
            .expect("valid");
        let group = b
            .add_node(NodeKind::Group {
                children: vec![
                    transform, mirror, instance, stretch, face, imported, grid, implicit,
                ],
            })
            .expect("valid");
        b.finish(group).expect("valid recipe")
    }

    /// A field using every expression and profile variant once.
    pub(crate) fn every_implicit_variant() -> exedra_implicit::Expr {
        use exedra_implicit as im;
        use exedra_implicit::{Expr, ProfileExpr};
        let leaves = vec![
            Expr::from(im::Sphere::new([0.1, 0.2, 0.3], 0.4).unwrap()),
            Expr::from(im::Cuboid::new([0.0; 3], [0.5, 0.4, 0.3]).unwrap()),
            Expr::from(im::Cylinder::new([0.0; 3], [0.2, 0.3, 1.0], 0.25, 0.6).unwrap()),
            Expr::from(im::Torus::new([0.0; 3], [0.0, 1.0, 0.1], 0.7, 0.1).unwrap()),
            Expr::from(im::Capsule::new([-0.5, 0.0, 0.0], [0.5, 0.1, 0.0], 0.1).unwrap()),
            Expr::from(im::RoundCone::new([0.0; 3], [0.0, 0.0, 0.8], 0.3, 0.1).unwrap()),
            Expr::from(
                im::Extrude::new(
                    ProfileExpr::from(im::Rectangle::new([0.0, 0.1], [0.3, 0.2]).unwrap()),
                    0.4,
                )
                .unwrap(),
            ),
            Expr::from(im::Revolve::new(ProfileExpr::from(
                im::Circle::new([0.6, 0.0], 0.15).unwrap(),
            ))),
        ];
        let blend = Expr::from(im::SmoothUnionN::new(leaves, 0.05).unwrap());
        let cut = Expr::from(im::Difference::new(
            blend,
            Expr::from(im::Tagged::new(
                Expr::from(im::HalfSpace::new([0.0, 0.0, 0.5], [0.1, 0.0, 1.0]).unwrap()),
                3,
            )),
        ));
        let half = Expr::from(
            im::Extrude::new(
                ProfileExpr::from(im::HalfPlane::new([0.0, 0.0], [1.0, 0.2]).unwrap()),
                0.5,
            )
            .unwrap(),
        );
        let lens = Expr::from(im::Intersection::new(cut, half));
        let smooth = Expr::from(
            im::SmoothUnion::new(
                lens,
                Expr::from(
                    im::Translate::new(
                        Expr::from(im::Sphere::new([0.0; 3], 0.2).unwrap()),
                        [0.0, 0.0, -0.9],
                    )
                    .unwrap(),
                ),
                0.1,
            )
            .unwrap(),
        );
        let scaled = Expr::from(im::Scale::new(smooth, 1.5).unwrap());
        let placed = Expr::from(
            im::Placed::new(
                scaled,
                Placement3::rotate_z_then_translate(0.4, 0.1, 0.0, 0.0),
            )
            .unwrap(),
        );
        Expr::from(im::Union::new(
            placed,
            Expr::from(im::Tagged::new(
                Expr::from(im::Sphere::new([1.2, 0.0, 0.0], 0.2).unwrap()),
                9,
            )),
        ))
    }
}

fn put_vector3(out: &mut String, point: [f64; 3]) {
    let _ = write!(
        out,
        " {} {} {}",
        hex(point[0]),
        hex(point[1]),
        hex(point[2])
    );
}
fn parse_vector3(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<[f64; 3], TextError> {
    Ok([
        next_f64(tokens, line)?,
        next_f64(tokens, line)?,
        next_f64(tokens, line)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use tests_support::full_coverage_recipe;

    #[test]
    fn round_trip_preserves_fingerprints() {
        let recipe = full_coverage_recipe();
        let text = dump_recipe(&recipe);
        let parsed = parse_recipe(&text).expect("parses");
        assert_eq!(
            recipe.recipe_fingerprint(),
            parsed.recipe_fingerprint(),
            "round trip must preserve content identity exactly"
        );
        // Second dump is byte-identical: the format is canonical.
        assert_eq!(text, dump_recipe(&parsed));
    }

    #[test]
    fn dump_is_deterministic_and_headed() {
        let recipe = full_coverage_recipe();
        let a = dump_recipe(&recipe);
        let b = dump_recipe(&recipe);
        assert_eq!(a, b);
        assert!(a.starts_with("constructive-ir-v1\nschema 41\n"));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(matches!(parse_recipe("nope"), Err(TextError::BadHeader)));
        let mut text = String::from(
            "constructive-ir-v1\nschema 41\nsources 0\nslots 0\npolicies 0\nimports 0\n",
        );
        text.push_str("profiles 0\nnodes 1\n  node 0 fancy thing source - material -\nroot 0\n");
        assert!(matches!(
            parse_recipe(&text),
            Err(TextError::Malformed { .. })
        ));
    }

    #[test]
    fn parse_revalidates_geometry() {
        // A structurally valid file with an invalid extrude height fails
        // through the builder, not silently.
        let recipe = {
            let mut b = RecipeBuilder::new();
            let p = b.add_profile(builders::rect_from_corner(1.0, 1.0).expect("rect"));
            let n = b
                .add_node(NodeKind::Extrude {
                    profile: p,
                    placement: Placement3::IDENTITY,
                    height: 1.0,
                    caps: CapMode::Both,
                })
                .expect("valid");
            b.finish(n).expect("valid")
        };
        let text = dump_recipe(&recipe);
        let negative_height = hex(-1.0);
        let broken = text.replace(&hex(1.0), &negative_height);
        assert!(matches!(
            parse_recipe(&broken),
            Err(TextError::Recipe(_)) | Err(TextError::Profile(_))
        ));
    }
}
