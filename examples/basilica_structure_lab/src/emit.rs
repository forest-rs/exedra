// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use exedra_assembly::{Assembly, CompiledParts, PartCompiler, RenderList, flatten};
use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::tessellate::EvalPolicy;

use crate::model::{
    ElementRole, OrientedBox, StructuralModel, TransferKind, TransferTarget, Vec3, add, scale, sub,
};

const SELECTED_BEARING: &str = "bearing-principal-south-east-on-wall-plate";
#[cfg(test)]
const SELECTED_BEARING_DIAGNOSTICS: usize = 5;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum Layer {
    Full,
    Structure,
    LoadPath,
    Bearings,
    TransparentRoof,
}

impl Layer {
    pub(crate) const ALL: [Self; 5] = [
        Self::Full,
        Self::Structure,
        Self::LoadPath,
        Self::Bearings,
        Self::TransparentRoof,
    ];

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "full" => Some(Self::Full),
            "structure" => Some(Self::Structure),
            "load-path" => Some(Self::LoadPath),
            "bearings" => Some(Self::Bearings),
            "transparent-roof" => Some(Self::TransparentRoof),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Structure => "structure",
            Self::LoadPath => "load-path",
            Self::Bearings => "bearings",
            Self::TransparentRoof => "transparent-roof",
        }
    }

    fn includes(self, role: ElementRole) -> bool {
        match self {
            Self::Structure => !role.is_roof_skin(),
            Self::Full | Self::LoadPath | Self::Bearings | Self::TransparentRoof => true,
        }
    }

    fn element_material(self, role: ElementRole) -> &'static str {
        match self {
            Self::Bearings => "diagnostic-context",
            Self::TransparentRoof if role == ElementRole::RoofCovering => {
                "semantic-roof-transparent"
            }
            Self::TransparentRoof if role == ElementRole::Boarding => {
                "semantic-boarding-transparent"
            }
            _ => role.material(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct EmittedScene {
    pub(crate) assembly: Assembly,
    pub(crate) compiled: CompiledParts,
    pub(crate) render_list: RenderList,
}

impl EmittedScene {
    pub(crate) fn group_count(&self) -> usize {
        self.render_list.items.len()
    }

    pub(crate) fn write_grouped_obj(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("OBJ path has no parent: {}", path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("OBJ path has no UTF-8 stem: {}", path.display()))?;
        let mtl_name = format!("{stem}.mtl");
        let mtl_path = path.with_extension("mtl");
        let (obj, materials) = export_obj(&self.compiled, &self.render_list, &mtl_name);
        std::fs::write(path, obj).map_err(|error| format!("write {}: {error}", path.display()))?;
        std::fs::write(&mtl_path, export_mtl(&materials))
            .map_err(|error| format!("write {}: {error}", mtl_path.display()))?;
        Ok(())
    }
}

pub(crate) fn emit(model: &StructuralModel, layer: Layer) -> Result<EmittedScene, String> {
    let mut assembly = Assembly::new();

    for element in &model.elements {
        if !element.present || !layer.includes(element.role) {
            continue;
        }
        add_solid(
            &mut assembly,
            &element.key,
            &element.solid,
            layer.element_material(element.role),
            element.role.label(),
            &element.evidence.to_string(),
        )?;
    }

    match layer {
        Layer::LoadPath => add_transfer_diagnostics(&mut assembly, model)?,
        Layer::Bearings => {
            add_bearing_diagnostics(&mut assembly, model)?;
            add_support_diagnostics(&mut assembly, model)?;
        }
        Layer::Full | Layer::Structure | Layer::TransparentRoof => {}
    }

    let mut compiler = PartCompiler::new();
    let compiled = compiler
        .compile_parts(&assembly, &EvalPolicy::default())
        .map_err(|error| format!("compile structural assembly: {error}"))?;
    let render_list = flatten(&assembly, &compiled);
    Ok(EmittedScene {
        assembly,
        compiled,
        render_list,
    })
}

fn add_transfer_diagnostics(
    assembly: &mut Assembly,
    model: &StructuralModel,
) -> Result<(), String> {
    for transfer in &model.transfers {
        let Some(from) = model.elements.get(transfer.from.0) else {
            continue;
        };
        if !from.present {
            continue;
        }
        let from_point = from.solid.center();
        let to_point = match transfer.to {
            TransferTarget::Element(id) => {
                let Some(target) = model.elements.get(id.0) else {
                    continue;
                };
                if !target.present {
                    continue;
                }
                target.solid.center()
            }
            TransferTarget::Support(id) => {
                let Some(support) = model.supports.get(id.0) else {
                    continue;
                };
                let center = model.elements[support.element.0].solid.center();
                [center[0], center[1], 0.0]
            }
        };
        let material = match transfer.kind {
            TransferKind::Bearing => "diagnostic-transfer-bearing",
            TransferKind::Joint => "diagnostic-transfer-joint",
            TransferKind::Ground => "diagnostic-transfer-ground",
        };
        add_solid(
            assembly,
            &transfer.key,
            &beam_between(from_point, to_point, 0.045),
            material,
            "load-transfer",
            "graph-derived",
        )?;
    }
    Ok(())
}

fn add_bearing_diagnostics(assembly: &mut Assembly, model: &StructuralModel) -> Result<(), String> {
    for bearing in &model.bearings {
        let Some(carried) = model.elements.get(bearing.carried.element.0) else {
            continue;
        };
        let Some(carrier) = model.elements.get(bearing.carrier.element.0) else {
            continue;
        };
        if !carried.present || !carrier.present {
            continue;
        }
        let carried_point = carried.solid.anchor(bearing.carried.local);
        let carrier_point = carrier.solid.anchor(bearing.carrier.local);
        let center = scale(add(carried_point, carrier_point), 0.5);
        add_solid(
            assembly,
            &bearing.key,
            &marker(center, 0.09),
            "diagnostic-bearing",
            "bearing",
            &bearing.evidence.to_string(),
        )?;
        if bearing.key == SELECTED_BEARING {
            add_solid(
                assembly,
                &format!("{}-carried-anchor", bearing.key),
                &marker(carried_point, 0.14),
                "diagnostic-bearing-carried",
                "bearing-carried-anchor-marker",
                "graph-derived-nonphysical-marker",
            )?;
            add_solid(
                assembly,
                &format!("{}-carrier-anchor", bearing.key),
                &marker(carrier_point, 0.11),
                "diagnostic-bearing-carrier",
                "bearing-carrier-anchor-marker",
                "graph-derived-nonphysical-marker",
            )?;
            let frame_origin = scale(add(carried_point, carrier_point), 0.5);
            for (suffix, axis, material) in [
                ("normal", bearing.normal, "diagnostic-bearing-normal"),
                (
                    "tangent-0",
                    bearing.tangents[0],
                    "diagnostic-bearing-tangent-0",
                ),
                (
                    "tangent-1",
                    bearing.tangents[1],
                    "diagnostic-bearing-tangent-1",
                ),
            ] {
                add_solid(
                    assembly,
                    &format!("{}-{suffix}", bearing.key),
                    &beam_between(frame_origin, add(frame_origin, scale(axis, 0.72)), 0.025),
                    material,
                    "bearing-frame-marker",
                    "graph-derived-nonphysical-marker",
                )?;
            }
        }
    }
    Ok(())
}

fn add_support_diagnostics(assembly: &mut Assembly, model: &StructuralModel) -> Result<(), String> {
    for support in &model.supports {
        let center = model.elements[support.element.0].solid.center();
        add_solid(
            assembly,
            &support.key,
            &marker([center[0], center[1], 0.0], 0.24),
            "diagnostic-support",
            "ground-support",
            "graph-derived",
        )?;
    }
    Ok(())
}

fn add_solid(
    assembly: &mut Assembly,
    key: &str,
    solid: &OrientedBox,
    material: &str,
    role: &str,
    evidence: &str,
) -> Result<(), String> {
    let part_key = format!("part-{key}");
    let part = assembly
        .add_recipe_part(
            &part_key,
            box_recipe(solid.size, &format!("structure-lab:{key}")),
        )
        .map_err(|error| format!("register {part_key}: {error}"))?;
    assembly
        .set_default_slot(part, "surface")
        .map_err(|error| format!("set slot on {part_key}: {error}"))?;
    assembly
        .set_part_material(part, "surface", material)
        .map_err(|error| format!("set material on {part_key}: {error}"))?;
    let instance = assembly
        .add_instance(None, key, part, placement(solid))
        .map_err(|error| format!("place {key}: {error}"))?;
    assembly
        .set_metadata(instance, "structural_role", role)
        .map_err(|error| format!("set role metadata on {key}: {error}"))?;
    assembly
        .set_metadata(instance, "evidence_class", evidence)
        .map_err(|error| format!("set evidence metadata on {key}: {error}"))?;
    Ok(())
}

fn box_recipe(size: Vec3, source_name: &str) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref(source_name);
    let surface = builder.material_slot("surface");
    let profile = builder.add_profile(
        builders::rect(size[0], size[1]).expect("validated positive structural box footprint"),
    );
    let node = builder
        .with_source(source)
        .with_material(surface)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: size[2],
            caps: CapMode::Both,
        })
        .expect("validated structural box");
    builder.finish(node).expect("box recipe has one root")
}

fn placement(solid: &OrientedBox) -> Placement3 {
    Placement3 {
        rows: [
            [
                solid.axes[0][0],
                solid.axes[1][0],
                solid.axes[2][0],
                solid.origin[0],
            ],
            [
                solid.axes[0][1],
                solid.axes[1][1],
                solid.axes[2][1],
                solid.origin[1],
            ],
            [
                solid.axes[0][2],
                solid.axes[1][2],
                solid.axes[2][2],
                solid.origin[2],
            ],
        ],
    }
}

fn marker(center: Vec3, size: f64) -> OrientedBox {
    OrientedBox {
        origin: [
            center[0] - size * 0.5,
            center[1] - size * 0.5,
            center[2] - size * 0.5,
        ],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        size: [size, size, size],
    }
}

fn beam_between(from: Vec3, to: Vec3, width: f64) -> OrientedBox {
    let delta = sub(to, from);
    let length = dot(delta, delta).sqrt();
    if length < 1.0e-9 {
        return marker(from, width * 2.0);
    }
    let along = scale(delta, 1.0 / length);
    let reference = if along[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let across = normalize(cross(reference, along));
    let normal = normalize(cross(along, across));
    OrientedBox {
        origin: sub(
            sub(from, scale(across, width * 0.5)),
            scale(normal, width * 0.5),
        ),
        axes: [along, across, normal],
        size: [length, width, width],
    }
}

fn export_obj(
    compiled: &CompiledParts,
    list: &RenderList,
    mtl_name: &str,
) -> (String, BTreeSet<String>) {
    let mut out =
        format!("# Basilica structural lab; generated deterministically\nmtllib {mtl_name}\n");
    let mut vertex_base = 1_u32;
    let mut materials = BTreeSet::new();
    for item in &list.items {
        let body = &compiled
            .part(item.part)
            .expect("render item part is compiled")
            .bodies[item.body as usize];
        let name = item.path.to_string().replace('/', "_");
        let material = item
            .regions
            .first()
            .and_then(|region| region.material.as_deref())
            .unwrap_or("unbound");
        materials.insert(material.to_owned());
        writeln!(out, "o {name}.body{}", item.body).expect("write String");
        writeln!(out, "g {name}").expect("write String");
        writeln!(out, "usemtl {material}").expect("write String");
        for &position in &body.tri.positions {
            let point = transform_point(&item.world, position);
            writeln!(out, "v {:.6} {:.6} {:.6}", point[0], point[1], point[2])
                .expect("write String");
        }
        for &uv in &body.tri.uvs {
            writeln!(out, "vt {:.6} {:.6}", uv[0], uv[1]).expect("write String");
        }
        for &normal in &body.tri.normals {
            let normal = transform_vector(&item.world, normal);
            writeln!(out, "vn {:.6} {:.6} {:.6}", normal[0], normal[1], normal[2])
                .expect("write String");
        }
        for triangle in body.tri.indices.chunks_exact(3) {
            let a = vertex_base + triangle[0];
            let b = vertex_base + triangle[1];
            let c = vertex_base + triangle[2];
            writeln!(out, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}").expect("write String");
        }
        vertex_base += u32::try_from(body.tri.positions.len()).expect("lab output fits u32");
    }
    (out, materials)
}

fn export_mtl(materials: &BTreeSet<String>) -> String {
    let mut out = String::from("# Basilica structural lab semantic materials\n");
    for material in materials {
        let (rgb, alpha) = material_style(material);
        writeln!(out, "newmtl {material}").expect("write String");
        writeln!(out, "Kd {:.4} {:.4} {:.4}", rgb[0], rgb[1], rgb[2]).expect("write String");
        writeln!(
            out,
            "Ka {:.4} {:.4} {:.4}",
            rgb[0] * 0.18,
            rgb[1] * 0.18,
            rgb[2] * 0.18
        )
        .expect("write String");
        writeln!(out, "Ks 0.08 0.08 0.08").expect("write String");
        writeln!(out, "Ns 24.0").expect("write String");
        writeln!(out, "d {alpha:.3}\n").expect("write String");
    }
    out
}

fn material_style(material: &str) -> (Vec3, f64) {
    match material {
        "semantic-roof-covering" => ([0.40, 0.10, 0.07], 1.0),
        "semantic-roof-transparent" => ([0.55, 0.16, 0.10], 0.26),
        "semantic-boarding" => ([0.46, 0.28, 0.12], 1.0),
        "semantic-boarding-transparent" => ([0.55, 0.34, 0.15], 0.34),
        "semantic-common-rafter" => ([0.88, 0.58, 0.18], 1.0),
        "semantic-purlin" => ([0.12, 0.56, 0.72], 1.0),
        "semantic-principal-rafter" => ([0.82, 0.18, 0.10], 1.0),
        "semantic-tie-beam" => ([0.88, 0.67, 0.12], 1.0),
        "semantic-king-post" => ([0.58, 0.22, 0.70], 1.0),
        "semantic-strut" => ([0.22, 0.68, 0.28], 1.0),
        "semantic-wall-plate" => ([0.18, 0.38, 0.78], 1.0),
        "semantic-masonry" => ([0.54, 0.50, 0.44], 1.0),
        "diagnostic-context" => ([0.38, 0.40, 0.42], 0.18),
        "diagnostic-bearing" => ([0.05, 1.0, 0.22], 1.0),
        "diagnostic-bearing-carried" => ([1.0, 0.08, 0.08], 1.0),
        "diagnostic-bearing-carrier" => ([0.05, 0.95, 0.25], 1.0),
        "diagnostic-bearing-normal" => ([1.0, 0.10, 0.08], 1.0),
        "diagnostic-bearing-tangent-0" => ([0.12, 0.45, 1.0], 1.0),
        "diagnostic-bearing-tangent-1" => ([1.0, 0.75, 0.08], 1.0),
        "diagnostic-support" => ([0.10, 0.35, 1.0], 1.0),
        "diagnostic-transfer-bearing" => ([0.05, 0.92, 0.24], 1.0),
        "diagnostic-transfer-joint" => ([0.92, 0.28, 0.10], 1.0),
        "diagnostic-transfer-ground" => ([0.10, 0.30, 1.0], 1.0),
        _ => ([0.72, 0.72, 0.72], 1.0),
    }
}

fn transform_point(placement: &Placement3, point: [f32; 3]) -> Vec3 {
    let point = [
        f64::from(point[0]),
        f64::from(point[1]),
        f64::from(point[2]),
    ];
    let rows = &placement.rows;
    [
        rows[0][0] * point[0] + rows[0][1] * point[1] + rows[0][2] * point[2] + rows[0][3],
        rows[1][0] * point[0] + rows[1][1] * point[1] + rows[1][2] * point[2] + rows[1][3],
        rows[2][0] * point[0] + rows[2][1] * point[1] + rows[2][2] * point[2] + rows[2][3],
    ]
}

fn transform_vector(placement: &Placement3, vector: [f32; 3]) -> Vec3 {
    let vector = [
        f64::from(vector[0]),
        f64::from(vector[1]),
        f64::from(vector[2]),
    ];
    let rows = &placement.rows;
    [
        rows[0][0] * vector[0] + rows[0][1] * vector[1] + rows[0][2] * vector[2],
        rows[1][0] * vector[0] + rows[1][1] * vector[1] + rows[1][2] * vector[2],
        rows[2][0] * vector[0] + rows[2][1] * vector[1] + rows[2][2] * vector[2],
    ]
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(value: Vec3) -> Vec3 {
    scale(value, 1.0 / dot(value, value).sqrt())
}

#[cfg(test)]
mod tests {
    use basilica_ruin::BasilicaParams;

    use super::*;

    #[test]
    fn full_layer_preserves_one_group_per_present_element() {
        let model = StructuralModel::western_bay(&BasilicaParams::default());
        let scene = emit(&model, Layer::Full).expect("clean graph emits");
        assert_eq!(scene.group_count(), model.elements.len());
        let paths: Vec<String> = scene
            .render_list
            .items
            .iter()
            .map(|item| item.path.to_string())
            .collect();
        let expected: Vec<String> = model
            .elements
            .iter()
            .map(|element| element.key.clone())
            .collect();
        assert_eq!(paths, expected);
    }

    #[test]
    fn diagnostic_layers_add_only_named_graph_diagnostics() {
        let model = StructuralModel::western_bay(&BasilicaParams::default());
        let load_path = emit(&model, Layer::LoadPath).expect("load path emits");
        assert_eq!(
            load_path.group_count(),
            model.elements.len() + model.transfers.len()
        );
        let expected_load_paths: Vec<&str> = model
            .elements
            .iter()
            .map(|element| element.key.as_str())
            .chain(model.transfers.iter().map(|transfer| transfer.key.as_str()))
            .collect();
        assert_eq!(
            load_path
                .render_list
                .items
                .iter()
                .map(|item| item.path.to_string())
                .collect::<Vec<_>>(),
            expected_load_paths
        );
        let bearings = emit(&model, Layer::Bearings).expect("bearings emit");
        assert_eq!(
            bearings.group_count(),
            model.elements.len()
                + model.bearings.len()
                + model.supports.len()
                + SELECTED_BEARING_DIAGNOSTICS
        );
        let mut expected_bearing_paths: Vec<String> = model
            .elements
            .iter()
            .map(|element| element.key.clone())
            .collect();
        for bearing in &model.bearings {
            expected_bearing_paths.push(bearing.key.clone());
            if bearing.key == SELECTED_BEARING {
                expected_bearing_paths.extend(
                    [
                        "carried-anchor",
                        "carrier-anchor",
                        "normal",
                        "tangent-0",
                        "tangent-1",
                    ]
                    .map(|suffix| format!("{}-{suffix}", bearing.key)),
                );
            }
        }
        expected_bearing_paths.extend(model.supports.iter().map(|support| support.key.clone()));
        assert_eq!(
            bearings
                .render_list
                .items
                .iter()
                .map(|item| item.path.to_string())
                .collect::<Vec<_>>(),
            expected_bearing_paths
        );
    }

    #[test]
    fn two_independent_builds_export_byte_identically() {
        let first_model = StructuralModel::western_bay(&BasilicaParams::default());
        let second_model = StructuralModel::western_bay(&BasilicaParams::default());
        assert_eq!(
            first_model.deterministic_signature(),
            second_model.deterministic_signature()
        );
        for layer in Layer::ALL {
            let first = emit(&first_model, layer).expect("first build emits");
            let second = emit(&second_model, layer).expect("second build emits");
            let first_obj = export_obj(&first.compiled, &first.render_list, "layer.mtl");
            let second_obj = export_obj(&second.compiled, &second.render_list, "layer.mtl");
            assert_eq!(first_obj, second_obj, "{} OBJ differs", layer.label());
        }
    }

    #[test]
    fn every_layer_emits_deep_valid_geometry() {
        let model = StructuralModel::western_bay(&BasilicaParams::default());
        for layer in Layer::ALL {
            let scene = emit(&model, layer).expect("layer emits");
            for part in scene.compiled.parts() {
                for body in &part.bodies {
                    assert!(
                        body.tri
                            .positions
                            .iter()
                            .flatten()
                            .all(|value| value.is_finite()),
                        "{} positions must be finite",
                        layer.label()
                    );
                    assert!(
                        !body.tri.indices.is_empty(),
                        "{} body has triangles",
                        layer.label()
                    );
                }
            }
        }
    }
}
