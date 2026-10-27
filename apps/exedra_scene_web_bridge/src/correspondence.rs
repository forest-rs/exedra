// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    backend::{Backend, Result},
    packet,
    protocol::refusal,
};
use exedra_scene::{Time, composition::stage::Prototype, composition::*, schemas};

fn unsupported(message: impl Into<String>) -> Box<dyn std::error::Error> {
    refusal("scene.instanceCorrespondence", message)
}

pub(crate) fn validate_all(b: &mut Backend) -> Result<(u64, usize, usize)> {
    let revision = b.live.change_cursor().revision();
    if b.verified_instances == Some((revision, b.source_revision)) {
        return Ok((revision, 0, 0));
    }
    let mut prototypes = 0;
    let mut instances = 0;
    let mut transforms = schemas::XformCache::new(Time::Default);
    let source = b
        .snapshot
        .as_ref()
        .and_then(|s| s.sources.first())
        .ok_or("sync before verifying instances")?;
    let kind = b
        .live
        .stage()
        .view(&b.store)
        .prim(source.path)
        .and_then(|p| p.type_name());
    for prototype in b.live.stage().prototypes() {
        let mut representative = None;
        for &path in prototype
            .instances()
            .iter()
            .filter(|p| b.placement_set.contains(p))
        {
            let root = b
                .live
                .stage()
                .view(&b.store)
                .prim(path)
                .ok_or_else(|| unsupported("tracked instance disappeared"))?;
            if root.type_name() != kind {
                return Err(unsupported(
                    "instance root type differs from the retained source",
                ));
            }
            representative.get_or_insert(path);
            instances += 1;
        }
        let Some(instance) = representative else {
            continue;
        };
        prototypes += 1;
        validate_instance(b, instance, &mut transforms)?;
        validate_contents(b, prototype)?;
    }
    if instances != b.placements.len() {
        return Err(unsupported(
            "tracked paths no longer match current native instance roots",
        ));
    }
    Ok((revision, prototypes, instances))
}

fn validate_contents(b: &Backend, prototype: Prototype<'_>) -> Result<()> {
    let scene = schemas::Scene::new(b.live.stage(), &b.store);
    for prim in prototype.prims() {
        if prim.relative_path().len() == 1
            && b.store.tokens.resolve(prim.relative_path()[0]) == exedra_scene::GENERATED_CHILD
        {
            continue;
        }
        if scene.is_a(prim.representative(), "Boundable") {
            return Err(unsupported(
                "prototype contains additional boundable geometry outside the retained mesh",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_pick(b: &Backend, instance: PathId) -> Result<()> {
    validate_instance(b, instance, &mut schemas::XformCache::new(Time::Default))?;
    let id = b
        .live
        .stage()
        .view(&b.store)
        .prim(instance)
        .and_then(|p| p.prototype())
        .ok_or_else(|| unsupported("instance has no current prototype"))?;
    let prototype = b
        .live
        .stage()
        .prototypes()
        .find(|p| p.id() == id)
        .ok_or_else(|| unsupported("instance prototype is missing"))?;
    validate_contents(b, prototype)
}

pub(crate) fn validate_instance(
    b: &Backend,
    instance: PathId,
    transforms: &mut schemas::XformCache,
) -> Result<()> {
    let source = b
        .snapshot
        .as_ref()
        .and_then(|s| s.sources.first())
        .ok_or("sync before verifying instance")?;
    let view = b.live.stage().view(&b.store);
    let root = view
        .prim(instance)
        .ok_or_else(|| unsupported("instance is missing"))?;
    let source_root = view
        .prim(source.path)
        .ok_or_else(|| unsupported("retained source prim is missing"))?;
    if !root.is_instance() || root.type_name() != source_root.type_name() {
        return Err(unsupported(
            "instance root composition differs from retained native source",
        ));
    }
    let mesh = root
        .children()
        .find(|p| p.name() == exedra_scene::GENERATED_CHILD)
        .ok_or_else(|| unsupported("instance has no generated mesh proxy"))?;
    if mesh.type_name() != Some("Mesh") {
        return Err(unsupported("generated instance child is not a Mesh"));
    }
    let expected = packet::mesh_values_at(b, source.output)?;
    let actual = packet::mesh_values_at(b, mesh.path())?;
    for ((expected, origin), (actual, actual_origin)) in expected.iter().zip(&actual) {
        if origin.is_none() || origin != actual_origin || !expected.shares_storage(actual) {
            return Err(unsupported(format!(
                "{} resolves geometry or provenance from a different source",
                b.store.paths.display(instance, &b.store.tokens)
            )));
        }
    }
    let scene = schemas::Scene::new(b.live.stage(), &b.store);
    for path in [source.output, mesh.path()] {
        let local = transforms
            .local_transform(&scene, path)
            .ok_or_else(|| unsupported("mesh transform is unavailable"))?;
        if local.resets_xform_stack
            || !local.problems.is_empty()
            || local.matrix != schemas::LocalTransform::identity().matrix
        {
            return Err(unsupported(
                "generated mesh child transforms are outside this view's occurrence contract",
            ));
        }
    }
    for prim in [view.prim(source.output).ok_or("source mesh missing")?, mesh] {
        for (name, expected) in [
            ("subdivisionScheme", "none"),
            ("orientation", "rightHanded"),
        ] {
            if !matches!(prim.attribute(name).and_then(|a|a.get(Time::Default)).map(|v|v.value),Some(Value::Token(token)) if b.store.tokens.resolve(token)==expected)
            {
                return Err(unsupported(
                    "unsupported composed mesh orientation or subdivision",
                ));
            }
        }
        for (name, interpolation) in [
            ("primvars:normals", "faceVarying"),
            (exedra_scene::NATIVE_FACE, "uniform"),
        ] {
            if !matches!(prim.property(name).and_then(|p|p.metadata("interpolation")).map(|v|v.value),Some(ResolvedValue::Scalar(Value::Token(token))) if b.store.tokens.resolve(token)==interpolation)
            {
                return Err(unsupported("unsupported composed mesh interpolation"));
            }
        }
    }
    Ok(())
}
