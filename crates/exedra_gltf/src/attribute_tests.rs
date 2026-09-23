// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tangents and mapped attribute streams in exported glTF.

use super::*;
use exedra_assembly::{CompilePolicy, PartCompiler};
use exedra_mesh::attributes::{AttrKey, Domain};
use exedra_mesh::{ExtractAttribute, MeshBuilder, TangentUv, attr, op};

const TAG: AttrKey<u32> = AttrKey::new(Domain::Face, "face.tag");
const PIVOT: AttrKey<[f32; 4]> = AttrKey::new(Domain::Vertex, "vertex.pivot");

/// One quad with UV0 = xy, UV1 = yx, a per-corner color, a vertex pivot and
/// a face tag.
fn painted_quad(tag: u32) -> exedra_mesh::Mesh {
    let mut builder = MeshBuilder::new();
    for p in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2, 3]).unwrap();
    let mut mesh = builder.build().unwrap().mesh;
    let face = mesh.faces().next().unwrap();
    let corners: Vec<_> = mesh
        .face_loop(face)
        .map(|c| {
            let v = mesh.to_vertex(c).unwrap();
            (c, v, *mesh.vertex_position(v).unwrap())
        })
        .collect();
    let mut edit = mesh.edit();
    for (corner, vertex, p) in corners {
        op::set_corner_uv(&mut edit, corner, [p[0], p[1]]).unwrap();
        op::set_attribute(&mut edit, attr::CORNER_UV1, corner, [p[1], p[0]]).unwrap();
        op::set_attribute(
            &mut edit,
            attr::CORNER_COLOR,
            corner,
            [p[0], p[1], 0.5, 1.0],
        )
        .unwrap();
        op::set_attribute(&mut edit, PIVOT, vertex, [p[0], p[1], 0.0, 2.0]).unwrap();
    }
    op::set_attribute(&mut edit, TAG, face, tag).unwrap();
    let _: () = edit.finish();
    mesh
}

fn policy() -> CompilePolicy {
    CompilePolicy {
        attributes: vec![
            ExtractAttribute::new(attr::CORNER_UV1, [0.0, 0.0]),
            ExtractAttribute::new(attr::CORNER_COLOR, [1.0; 4]),
            ExtractAttribute::new(PIVOT, [0.0; 4]),
            ExtractAttribute::new(TAG, 0),
        ],
        tangents: Some(TangentUv::Primary),
        ..CompilePolicy::default()
    }
}

fn scene(tag: u32) -> (Assembly, CompiledParts) {
    let mut assembly = Assembly::new();
    let part = assembly
        .add_baked_part("quad", painted_quad(tag), &[])
        .unwrap();
    assembly
        .add_instance(None, "a", part, Placement3::IDENTITY)
        .unwrap();
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &policy())
        .unwrap();
    (assembly, compiled)
}

fn mappings() -> Vec<GltfAttribute> {
    vec![
        GltfAttribute::tex_coord(attr::CORNER_UV1, 1),
        GltfAttribute::color(attr::CORNER_COLOR, 0),
        GltfAttribute::custom(PIVOT, "_WIND_PIVOT"),
        GltfAttribute::custom(TAG, "_TAG").with_integers(IntegerEncoding::UnsignedShort),
    ]
}

#[test]
fn mapped_streams_and_tangents_round_trip() {
    let (assembly, compiled) = scene(7);
    let mappings = mappings();
    let options = GltfExportOptions::default().with_attributes(&mappings);
    let export = export_glb_with_options(&assembly, &compiled, options).unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        document.attribute_semantics(),
        [
            "COLOR_0",
            "NORMAL",
            "POSITION",
            "TANGENT",
            "TEXCOORD_0",
            "TEXCOORD_1",
            "_TAG",
            "_WIND_PIVOT"
        ]
    );
    let positions = document.attribute_components("POSITION").unwrap();
    let uv1 = document.attribute_components("TEXCOORD_1").unwrap();
    let color = document.attribute_components("COLOR_0").unwrap();
    let pivot = document.attribute_components("_WIND_PIVOT").unwrap();
    let tag = document.attribute_components("_TAG").unwrap();
    let tangents = document.attribute_components("TANGENT").unwrap();
    for (i, p) in positions.chunks(3).enumerate() {
        assert_eq!(&uv1[2 * i..2 * i + 2], [p[1], p[0]]);
        assert_eq!(&color[4 * i..4 * i + 4], [p[0], p[1], 0.5, 1.0]);
        assert_eq!(&pivot[4 * i..4 * i + 4], [p[0], p[1], 0.0, 2.0]);
        assert_eq!(tag[i], 7.0);
        assert_eq!(&tangents[4 * i..4 * i + 4], [1.0, 0.0, 0.0, 1.0]);
    }
    let json = document.json();
    let index = |value: &Value| usize::try_from(value.as_u64().unwrap()).unwrap();
    let tag_accessor =
        &json["accessors"][index(&json["meshes"][0]["primitives"][0]["attributes"]["_TAG"])];
    assert_eq!(tag_accessor["componentType"], 5123);
    assert_eq!(
        json["bufferViews"][index(&tag_accessor["bufferView"])]["byteStride"],
        4
    );
    assert_eq!(export.stats.attribute_accessors, 5);
    assert_eq!(
        export.stats.attribute_bytes,
        4 * (16 + 8 + 16 + 16 + 4),
        "tangent, uv1, color, pivot, padded tag for four vertices"
    );
    assert_eq!(export.stats.unmapped_attribute_streams, 0);
    assert_eq!(export.stats.missing_attribute_streams, 0);

    let again = export_glb_with_options(&assembly, &compiled, options).unwrap();
    assert_eq!(again.bytes, export.bytes, "export is deterministic");
}

#[test]
fn unmapped_and_missing_streams_are_counted_not_exported() {
    let (assembly, compiled) = scene(1);
    let mappings = [
        GltfAttribute::tex_coord(attr::CORNER_UV1, 1),
        GltfAttribute::custom(
            AttrKey::<f32>::new(Domain::HalfEdge, "corner.absent"),
            "_ABSENT",
        ),
    ];
    let export = export_glb_with_options(
        &assembly,
        &compiled,
        GltfExportOptions::default().with_attributes(&mappings),
    )
    .unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert!(!document.attribute_semantics().contains(&"_ABSENT"));
    assert!(!document.attribute_semantics().contains(&"COLOR_0"));
    assert_eq!(export.stats.unmapped_attribute_streams, 3);
    assert_eq!(export.stats.missing_attribute_streams, 1);

    // Without mappings the default export still carries tangents.
    let plain = export_glb(&assembly, &compiled).unwrap();
    let document = GlbDocument::parse(&plain.bytes).unwrap();
    assert_eq!(
        document.attribute_semantics(),
        ["NORMAL", "POSITION", "TANGENT", "TEXCOORD_0"]
    );
    assert_eq!(plain.stats.unmapped_attribute_streams, 4);
}

#[test]
fn invalid_mappings_are_rejected_before_export() {
    let (assembly, compiled) = scene(1);
    for (mapping, reason) in [
        (
            vec![GltfAttribute::tex_coord(attr::CORNER_UV1, 0)],
            "TEXCOORD_0 is the primary UV set",
        ),
        (
            vec![GltfAttribute::custom(PIVOT, "WIND")],
            "custom semantics must start with an underscore",
        ),
        (
            vec![
                GltfAttribute::color(attr::CORNER_COLOR, 0),
                GltfAttribute::color(PIVOT, 0),
            ],
            "semantic is mapped more than once",
        ),
        (
            vec![GltfAttribute::tex_coord(attr::CORNER_UV1, 2)],
            "indexed sets must be contiguous from the first set",
        ),
        (
            vec![GltfAttribute::color(attr::CORNER_COLOR, 1)],
            "indexed sets must be contiguous from the first set",
        ),
    ] {
        let error = export_glb_with_options(
            &assembly,
            &compiled,
            GltfExportOptions::default().with_attributes(&mapping),
        )
        .unwrap_err();
        let GltfError::InvalidAttributeMapping { reason: got, .. } = error else {
            panic!("unexpected error {error:?}");
        };
        assert_eq!(got, reason);
    }
}

#[test]
fn type_mismatches_and_unrepresentable_integers_fail() {
    let (assembly, compiled) = scene(70_000);
    let mismatch = [GltfAttribute::color(TAG, 0)];
    assert!(matches!(
        export_glb_with_options(
            &assembly,
            &compiled,
            GltfExportOptions::default().with_attributes(&mismatch)
        ),
        Err(GltfError::AttributeTypeMismatch {
            stream: "face.tag",
            ..
        })
    ));
    let short = [GltfAttribute::custom(TAG, "_TAG").with_integers(IntegerEncoding::UnsignedShort)];
    assert!(matches!(
        export_glb_with_options(
            &assembly,
            &compiled,
            GltfExportOptions::default().with_attributes(&short)
        ),
        Err(GltfError::UnrepresentableAttribute { value: 70_000, .. })
    ));
    // The float encoding represents the same value exactly.
    let float = [GltfAttribute::custom(TAG, "_TAG")];
    let export = export_glb_with_options(
        &assembly,
        &compiled,
        GltfExportOptions::default().with_attributes(&float),
    )
    .unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert!(
        document
            .attribute_components("_TAG")
            .unwrap()
            .iter()
            .all(|&v| v == 70_000.0)
    );
}

#[test]
fn mappings_select_streams_by_domain_and_name() {
    let vertex_tag = AttrKey::<u32>::new(Domain::Vertex, "tag");
    let face_tag = AttrKey::<u32>::new(Domain::Face, "tag");
    let stream = |domain| exedra_mesh::AttributeStream {
        domain,
        name: "tag",
        values: exedra_mesh::AttributeBuffer::U32(Vec::new()),
    };
    let mapping = GltfAttribute::custom(face_tag, "_TAG");
    assert!(mapping.selects(&stream(Domain::Face)));
    assert!(!mapping.selects(&stream(Domain::Vertex)));
    assert!(GltfAttribute::custom(vertex_tag, "_TAG").selects(&stream(Domain::Vertex)));
}

#[test]
fn indexed_sets_stay_contiguous_per_body() {
    let (assembly, compiled) = scene(1);
    let absent = AttrKey::<[f32; 2]>::new(Domain::HalfEdge, "corner.absent_uv");

    // A missing highest set shortens the run and is counted.
    let top_missing = [
        GltfAttribute::tex_coord(attr::CORNER_UV1, 1),
        GltfAttribute::tex_coord(absent, 2),
    ];
    let export = export_glb_with_options(
        &assembly,
        &compiled,
        GltfExportOptions::default().with_attributes(&top_missing),
    )
    .unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert!(document.attribute_semantics().contains(&"TEXCOORD_1"));
    assert!(!document.attribute_semantics().contains(&"TEXCOORD_2"));
    assert_eq!(export.stats.missing_attribute_streams, 1);

    // A missing lower set under a present higher one would leave a gap.
    let gap = [
        GltfAttribute::tex_coord(absent, 1),
        GltfAttribute::tex_coord(attr::CORNER_UV1, 2),
    ];
    let error = export_glb_with_options(
        &assembly,
        &compiled,
        GltfExportOptions::default().with_attributes(&gap),
    )
    .unwrap_err();
    assert!(
        matches!(
            &error,
            GltfError::AttributeSetGap { stream: "corner.absent_uv", semantic, .. }
                if semantic == "TEXCOORD_1"
        ),
        "unexpected error {error:?}"
    );
}
