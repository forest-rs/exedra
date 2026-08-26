// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use exedra_assembly::{
    Assembly, CompiledParts, InstanceAddress, PartCompiler, RenderList, flatten,
};
use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::tessellate::EvalPolicy;
use joiner::{
    Construction, OrientedBox, TransferKind, TransferTarget, instance_address, lower_selected,
};

use crate::model::{ElementRole, Vec3};
use exedra_math::{add, cross, dot, normalize, scale, sub};

const SELECTED_BEARING: &str = "bearing-principal-south-east-on-wall-plate";
#[cfg(test)]
const SELECTED_BEARING_DIAGNOSTICS: usize = 5;

fn address_path(address: &InstanceAddress) -> String {
    address.to_string().trim_start_matches('/').to_owned()
}

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

    /// Whether an element carrying this opaque `joiner` role label belongs in
    /// the layer. An unrecognised label is kept: the lab never silently drops
    /// geometry it does not recognise.
    fn includes_label(self, label: &str) -> bool {
        ElementRole::from_label(label).is_none_or(|role| self.includes(role))
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

pub(crate) fn emit(construction: &Construction, layer: Layer) -> Result<EmittedScene, String> {
    // The elements come from `joiner`: one root instance per geometry-bearing
    // element, keyed by the element key, with every part edit already
    // composed into its recipe. The lab only re-binds materials for the
    // layer it is drawing and adds its own diagnostic markers on top.
    let mut assembly = lower_selected(construction, |element| layer.includes_label(&element.role))
        .map_err(|error| format!("lower structural construction: {error}"))?;

    for element in construction.elements() {
        let Some(role) = ElementRole::from_label(&element.role) else {
            continue;
        };
        let Some(address) = instance_address(&element.key) else {
            continue;
        };
        let Some(instance) = assembly.instance_by_address(&address) else {
            continue;
        };
        assembly
            .bind_material(instance, "surface", layer.element_material(role))
            .map_err(|error| format!("bind material on {}: {error}", element.key))?;
    }

    match layer {
        Layer::LoadPath => add_transfer_diagnostics(&mut assembly, construction)?,
        Layer::Bearings => {
            add_bearing_diagnostics(&mut assembly, construction)?;
            add_support_diagnostics(&mut assembly, construction)?;
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
    construction: &Construction,
) -> Result<(), String> {
    for transfer in construction.transfers() {
        let Some(from) = construction.element(&transfer.from) else {
            continue;
        };
        if !from.present {
            continue;
        }
        let from_point = from.extent.center();
        let to_point = match &transfer.to {
            TransferTarget::Element(key) => {
                let Some(target) = construction.element(key) else {
                    continue;
                };
                if !target.present {
                    continue;
                }
                target.extent.center()
            }
            TransferTarget::Support(key) => {
                let Some(support) = construction.support(key) else {
                    continue;
                };
                let Some(element) = construction.element(&support.element) else {
                    continue;
                };
                let center = element.extent.center();
                [center[0], center[1], 0.0]
            }
            _ => continue,
        };
        let material = match transfer.kind {
            TransferKind::Contact => "diagnostic-transfer-bearing",
            TransferKind::Joint => "diagnostic-transfer-joint",
            TransferKind::Ground => "diagnostic-transfer-ground",
            _ => "diagnostic-transfer-bearing",
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

fn add_bearing_diagnostics(
    assembly: &mut Assembly,
    construction: &Construction,
) -> Result<(), String> {
    for bearing in construction.contacts() {
        let Some(carried) = construction.element(&bearing.carried.element) else {
            continue;
        };
        let Some(carrier) = construction.element(&bearing.carrier.element) else {
            continue;
        };
        if !carried.present || !carrier.present {
            continue;
        }
        let carried_point = carried.extent.anchor(bearing.carried.local);
        let carrier_point = carrier.extent.anchor(bearing.carrier.local);
        let center = scale(add(carried_point, carrier_point), 0.5);
        add_solid(
            assembly,
            &bearing.key,
            &marker(center, 0.09),
            "diagnostic-bearing",
            "bearing",
            bearing.evidence.class.label(),
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

fn add_support_diagnostics(
    assembly: &mut Assembly,
    construction: &Construction,
) -> Result<(), String> {
    for support in construction.supports() {
        let Some(element) = construction.element(&support.element) else {
            continue;
        };
        let center = element.extent.center();
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
    Placement3::from_axes(solid.axes[0], solid.axes[1], solid.axes[2], solid.origin)
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
    let across =
        normalize(cross(reference, along)).expect("authored frame axes are non-degenerate");
    let normal = normalize(cross(along, across)).expect("authored frame axes are non-degenerate");
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
        let reflected = linear_determinant(&item.world) < 0.0;
        let body = &compiled
            .part(item.part)
            .expect("render item part is compiled")
            .bodies[item.body as usize];
        let name = address_path(&item.address).replace('/', "_");
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
            let (b, c) = if reflected {
                // Baking a negative-determinant transform into positions
                // reverses orientation. OBJ has no remaining node transform
                // to signal that reflection to a viewer, so restore outward
                // winding while keeping each corner's UV and normal attached.
                (vertex_base + triangle[2], vertex_base + triangle[1])
            } else {
                (vertex_base + triangle[1], vertex_base + triangle[2])
            };
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

fn linear_determinant(placement: &Placement3) -> f64 {
    let rows = &placement.rows;
    rows[0][0] * (rows[1][1] * rows[2][2] - rows[1][2] * rows[2][1])
        - rows[0][1] * (rows[1][0] * rows[2][2] - rows[1][2] * rows[2][0])
        + rows[0][2] * (rows[1][0] * rows[2][1] - rows[1][1] * rows[2][0])
}

#[cfg(test)]
mod tests {
    use basilica_ruin::BasilicaParams;

    use super::*;

    fn present_elements(construction: &Construction) -> usize {
        construction
            .elements()
            .iter()
            .filter(|element| element.present)
            .count()
    }

    fn element_keys(construction: &Construction) -> Vec<String> {
        construction
            .elements()
            .iter()
            .filter(|element| element.present)
            .map(|element| element.key.clone())
            .collect()
    }

    fn obj_vector(line: &str) -> Vec3 {
        let mut fields = line.split_whitespace().skip(1);
        [
            fields.next().expect("x").parse().expect("finite x"),
            fields.next().expect("y").parse().expect("finite y"),
            fields.next().expect("z").parse().expect("finite z"),
        ]
    }

    fn obj_corner(corner: &str) -> (usize, usize) {
        let mut fields = corner.split('/');
        let position = fields
            .next()
            .expect("position index")
            .parse::<usize>()
            .expect("numeric position index")
            - 1;
        fields.next().expect("texture index");
        let normal = fields
            .next()
            .expect("normal index")
            .parse::<usize>()
            .expect("numeric normal index")
            - 1;
        (position, normal)
    }

    #[test]
    fn full_layer_preserves_one_group_per_present_element() {
        let model = crate::model::western_bay(&BasilicaParams::default());
        let scene = emit(&model, Layer::Full).expect("clean graph emits");
        assert_eq!(scene.group_count(), present_elements(&model));
        let paths: Vec<String> = scene
            .render_list
            .items
            .iter()
            .map(|item| address_path(&item.address))
            .collect();
        let expected: Vec<String> = element_keys(&model);
        assert_eq!(paths, expected);
    }

    #[test]
    fn reflected_roof_obj_winding_agrees_with_exported_normals() {
        let model = crate::model::western_bay(&BasilicaParams::default());
        let scene = emit(&model, Layer::Full).expect("clean graph emits");
        let reflected = scene
            .render_list
            .items
            .iter()
            .find(|item| address_path(&item.address) == "roof-covering-north")
            .expect("north roof covering");
        assert!(
            linear_determinant(&reflected.world) < 0.0,
            "fixture must exercise a reflected world placement"
        );

        let (obj, _) = export_obj(&scene.compiled, &scene.render_list, "layer.mtl");
        let positions: Vec<Vec3> = obj
            .lines()
            .filter(|line| line.starts_with("v "))
            .map(obj_vector)
            .collect();
        let normals: Vec<Vec3> = obj
            .lines()
            .filter(|line| line.starts_with("vn "))
            .map(obj_vector)
            .collect();
        let mut in_reflected_group = false;
        let mut triangle_count = 0;
        for line in obj.lines() {
            if let Some(group) = line.strip_prefix("g ") {
                in_reflected_group = group == "roof-covering-north";
            } else if in_reflected_group && line.starts_with("f ") {
                let corners: Vec<(usize, usize)> =
                    line.split_whitespace().skip(1).map(obj_corner).collect();
                let geometric = cross(
                    sub(positions[corners[1].0], positions[corners[0].0]),
                    sub(positions[corners[2].0], positions[corners[0].0]),
                );
                for (_, normal) in corners {
                    assert!(
                        dot(geometric, normals[normal]) > 0.0,
                        "reflected OBJ winding must agree with its exported normals"
                    );
                }
                triangle_count += 1;
            }
        }
        assert!(
            triangle_count > 0,
            "fixture must export reflected triangles"
        );
    }

    #[test]
    fn diagnostic_layers_add_only_named_graph_diagnostics() {
        let model = crate::model::western_bay(&BasilicaParams::default());
        let load_path = emit(&model, Layer::LoadPath).expect("load path emits");
        assert_eq!(
            load_path.group_count(),
            present_elements(&model) + model.transfers().len()
        );
        let expected_load_paths: Vec<String> = element_keys(&model)
            .into_iter()
            .chain(
                model
                    .transfers()
                    .iter()
                    .map(|transfer| transfer.key.clone()),
            )
            .collect();
        assert_eq!(
            load_path
                .render_list
                .items
                .iter()
                .map(|item| address_path(&item.address))
                .collect::<Vec<_>>(),
            expected_load_paths
        );
        let bearings = emit(&model, Layer::Bearings).expect("bearings emit");
        assert_eq!(
            bearings.group_count(),
            present_elements(&model)
                + model.contacts().len()
                + model.supports().len()
                + SELECTED_BEARING_DIAGNOSTICS
        );
        let mut expected_bearing_paths: Vec<String> = element_keys(&model);
        for bearing in model.contacts() {
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
        expected_bearing_paths.extend(model.supports().iter().map(|support| support.key.clone()));
        assert_eq!(
            bearings
                .render_list
                .items
                .iter()
                .map(|item| address_path(&item.address))
                .collect::<Vec<_>>(),
            expected_bearing_paths
        );
    }

    #[test]
    fn two_independent_builds_export_byte_identically() {
        let first_model = crate::model::western_bay(&BasilicaParams::default());
        let second_model = crate::model::western_bay(&BasilicaParams::default());
        assert_eq!(
            crate::model::deterministic_signature(&first_model),
            crate::model::deterministic_signature(&second_model)
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
        let model = crate::model::western_bay(&BasilicaParams::default());
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
