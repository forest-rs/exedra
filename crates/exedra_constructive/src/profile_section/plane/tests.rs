// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::ir::{CapMode, LoftPolicy};
use crate::section::{PlaneSection, SectionLoop, SectionRegion, SectionStats};
use crate::tessellate::{
    EvalPolicy, Feature, TessellateError, tessellate_extrude, tessellate_loft,
};
use alloc::vec;

fn section() -> PlaneSection {
    PlaneSection {
        frame: Placement3::IDENTITY,
        regions: vec![SectionRegion {
            outer: boundary(&[[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]]),
            holes: vec![boundary(&[[1.0, 1.0], [1.0, 2.0], [2.0, 2.0], [2.0, 1.0]])],
        }],
        stats: SectionStats::default(),
    }
}

fn boundary(points: &[[f64; 2]]) -> SectionLoop {
    SectionLoop {
        points: points.to_vec(),
        edge_features: vec![Feature::Imported; points.len()],
    }
}

#[test]
fn malformed_sources_report_the_region_and_boundary() {
    let mut section = section();
    section.regions.push(section.regions[0].clone());
    section.regions[1].holes[0].edge_features.pop();
    assert_eq!(
        section.to_profiles(),
        Err(SectionProfileError::SourceCount {
            region: 1,
            loop_index: 1,
            points: 4,
            sources: 3,
        })
    );
}

#[test]
fn invalid_boundary_geometry_is_refused() {
    for points in [
        vec![],
        vec![[0.0, 0.0], [1.0, 0.0]],
        vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]],
        vec![[0.0, 0.0], [f64::MAX, 0.0], [0.0, f64::MAX]],
    ] {
        let mut section = section();
        section.regions[0].outer = boundary(&points);
        assert_eq!(
            section.to_profiles(),
            Err(SectionProfileError::InvalidBoundary {
                region: 0,
                loop_index: 0,
            })
        );
    }
    for points in [
        vec![[0.0, 0.0], [f64::NAN, 0.0], [0.0, 1.0]],
        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.0]],
    ] {
        let mut section = section();
        section.regions[0].outer = boundary(&points);
        assert!(matches!(
            section.to_profiles(),
            Err(SectionProfileError::InvalidProfile { region: 0, .. })
        ));
    }
}

#[test]
fn invalid_winding_is_not_repaired() {
    let mut section = section();
    section.regions[0].holes[0].points.reverse();
    assert_eq!(
        section.to_profiles(),
        Err(SectionProfileError::InvalidProfile {
            region: 0,
            error: ProfileError::WrongWinding { hole: Some(0) },
        })
    );
}

#[test]
fn empty_sections_and_invalid_frames_are_explicit() {
    let mut section = section();
    section.regions.clear();
    assert!(section.to_profiles().unwrap().is_empty());
    let mut mesh_section = geometry::PlaneSection {
        frame: section.frame,
        regions: Vec::new(),
        stats: SectionStats::default(),
    };
    assert!(
        profiles_from_mesh_section(&mesh_section)
            .unwrap()
            .is_empty()
    );
    for value in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        section.frame.rows[2][2] = value;
        mesh_section.frame = section.frame;
        assert_eq!(
            section.to_profiles(),
            Err(SectionProfileError::InvalidFrame)
        );
        assert_eq!(
            profiles_from_mesh_section(&mesh_section),
            Err(SectionProfileError::InvalidFrame)
        );
    }
}

#[test]
fn tag_lookup_survives_an_authored_seam_change() {
    let mut section = section();
    section.regions[0].holes[0].edge_features = (0..4)
        .map(|seg| Feature::Wall { loop_index: 1, seg })
        .collect();
    let converted = section.to_profiles().unwrap().remove(0);
    let hole = converted.profile.holes()[0].with_seam(2).unwrap();
    for segment in hole.segs() {
        let tag = segment.tag.unwrap();
        assert!(tag.0 >= 4);
        assert_eq!(
            converted.source(tag),
            Some(&Feature::Wall {
                loop_index: 1,
                seg: tag.0 - 4
            })
        );
    }
    assert_eq!(converted.source(SegTag(8)), None);
}

#[test]
fn collinear_section_rims_remain_closed_in_extrusion_and_loft_caps() {
    let mut section = section();
    section.regions[0].outer =
        boundary(&[[0.0, 0.0], [2.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]]);
    section.regions[0].holes[0] =
        boundary(&[[1.0, 1.0], [1.0, 1.5], [1.0, 2.0], [2.0, 2.0], [2.0, 1.0]]);
    let converted = section.to_profiles().unwrap().remove(0);
    let policy = EvalPolicy::default();
    let extrusion = tessellate_extrude(
        &converted.profile,
        &converted.placement,
        0.5,
        CapMode::Both,
        &policy,
    )
    .unwrap();
    let loft = tessellate_loft(
        &[
            (converted.placement, &converted.profile),
            (Placement3::translate(0.0, 0.0, 0.5), &converted.profile),
        ],
        LoftPolicy::Ruled,
        CapMode::Both,
        &policy,
    )
    .unwrap();
    for body in [extrusion, loft] {
        assert_eq!(body.mesh.vertices().count(), 20);
        assert!(body.mesh.validate_deep().is_empty());
        assert!(body.mesh.boundary_loops().unwrap().is_empty());
        for face in body.mesh.faces() {
            assert!(
                !body
                    .mesh
                    .face_triangles_counted(face, exedra_mesh::FaceTriangulation::Robust)
                    .1
            );
        }
    }
}

#[test]
fn cap_realization_failure_is_typed() {
    let converted = section().to_profiles().unwrap().remove(0);
    let placement = Placement3::translate(1e9, 1e9, 0.0);
    let policy = EvalPolicy::default();
    assert!(matches!(
        tessellate_extrude(&converted.profile, &placement, 0.5, CapMode::Both, &policy,),
        Err(TessellateError::CollapsedGeometry)
    ));
    assert!(matches!(
        tessellate_loft(
            &[
                (placement, &converted.profile),
                (Placement3::translate(1e9, 1e9, 0.5), &converted.profile)
            ],
            LoftPolicy::Ruled,
            CapMode::Both,
            &policy,
        ),
        Err(TessellateError::CollapsedGeometry)
    ));
}
