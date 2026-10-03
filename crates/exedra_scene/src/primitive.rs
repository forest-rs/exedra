// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::{
    NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder, RecipeError,
};
use layerstack::{
    EditTarget, LayerStore, PathId, PropertyDefinition, PropertyPath, PropertySpec, PropertyType,
    SchemaDefinition, SchemaRegistryBuilder, Specifier, TokenInterner, Transaction, Value,
};
use layerstack_schemas::{PrimView, Scene, Time, value};

use crate::{SceneError, SceneErrorKind};

/// Typed box construction, minimum corner at the local origin, derived from `Xform`.
pub const BOX_SCHEMA: &str = "ExedraBox";
/// Typed capped +Z cylinder construction, base center at the local origin.
pub const CYLINDER_SCHEMA: &str = "ExedraCylinder";
/// Box extents as a varying `double3`, all strictly positive.
pub const SIZE: &str = "exedra:size";
/// Cylinder radius as a varying `double`, strictly positive.
pub const RADIUS: &str = "exedra:radius";
/// Cylinder height as a varying `double`, strictly positive.
pub const HEIGHT: &str = "exedra:height";
/// Cylinder side count as a uniform `uint`, at least three.
pub const SEGMENTS: &str = "exedra:segments";

/// Registers Exedra's typed construction schemas into a caller-owned registry.
///
/// Register the `OpenUSD` `UsdGeom` domain in the same builder so `Xform` exists.
/// All registry tokens must belong to the store used to compose the stage.
/// Dimensions have no fallback: missing construction intent is a refusal.
pub fn register_schemas(builder: &mut SchemaRegistryBuilder, tokens: &mut TokenInterner) {
    let xform = tokens.intern("Xform");
    let box_schema = SchemaDefinition::typed(tokens.intern(BOX_SCHEMA))
        .with_parent(xform)
        .with_property(
            PropertyDefinition::attribute(tokens.intern(SIZE)).with_type(PropertyType::new(
                "double3",
                false,
                Value::Vec3d([0.0; 3]),
            )),
        );
    let cylinder_schema = SchemaDefinition::typed(tokens.intern(CYLINDER_SCHEMA))
        .with_parent(xform)
        .with_property(
            PropertyDefinition::attribute(tokens.intern(RADIUS)).with_type(PropertyType::new(
                "double",
                false,
                Value::Double(0.0),
            )),
        )
        .with_property(
            PropertyDefinition::attribute(tokens.intern(HEIGHT)).with_type(PropertyType::new(
                "double",
                false,
                Value::Double(0.0),
            )),
        )
        .with_property(
            PropertyDefinition::attribute(tokens.intern(SEGMENTS))
                .with_type(PropertyType::new("uint", false, Value::UInt(0)))
                .uniform(),
        );
    builder.register(box_schema).register(cylinder_schema);
}

/// Prepares native authoring of one typed primitive with default-time parameters.
///
/// The target determines layer and namespace mapping. Apply through `LiveStage`
/// to obtain the ordinary change report and inverse transaction. Further edits
/// and animation use the same native property addresses and transactions.
///
/// # Errors
///
/// Runs the ordinary recipe builder's parameter validation before authoring.
/// Applying the transaction can also reject existing specs or unmappable paths.
pub fn define_primitive(
    store: &mut dyn LayerStore,
    target: &EditTarget,
    path: PathId,
    spec: PrimitiveSpec,
) -> Result<Transaction, SceneError> {
    let _ = recipe(spec).map_err(|e| SceneError::new(store, path, SceneErrorKind::Recipe(e)))?;
    let mut transaction = Transaction::new();
    let schema = match spec {
        PrimitiveSpec::Box { .. } => BOX_SCHEMA,
        PrimitiveSpec::Cylinder { .. } => CYLINDER_SCHEMA,
        _ => {
            return Err(SceneError::new(
                store,
                path,
                SceneErrorKind::UnsupportedPrimitive,
            ));
        }
    };
    transaction.create_prim(
        target.prim(path),
        Specifier::Def,
        Some(store.tokens_mut().intern(schema)),
    );
    let mut property = |name: &str, type_name: &str, value: Value, uniform: bool| {
        let ty = PropertyType::new(type_name, false, value.clone());
        let mut spec = PropertySpec::typed_attribute(ty).with_default(value);
        if uniform {
            spec = spec.uniform();
        }
        let name = store.tokens_mut().intern(name);
        transaction.create_property(target.property(PropertyPath::new(path, name)), spec);
    };
    match spec {
        PrimitiveSpec::Box { size } => property(SIZE, "double3", Value::Vec3d(size), false),
        PrimitiveSpec::Cylinder {
            radius,
            height,
            segments,
        } => {
            property(RADIUS, "double", Value::Double(radius), false);
            property(HEIGHT, "double", Value::Double(height), false);
            property(SEGMENTS, "uint", Value::UInt(segments), true);
        }
        _ => {
            return Err(SceneError::new(
                store,
                path,
                SceneErrorKind::UnsupportedPrimitive,
            ));
        }
    }
    Ok(transaction)
}

pub(crate) fn recipe(spec: PrimitiveSpec) -> Result<Recipe, RecipeError> {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref("primitive");
    let root = builder.with_source(source).add(NodeKind::Primitive {
        spec,
        placement: Placement3::IDENTITY,
    })?;
    builder.finish(root)
}

pub(crate) fn read(scene: Scene<'_>, path: PathId, time: Time) -> Result<Recipe, SceneError> {
    let view = PrimView::new(scene, path);
    let error = |kind| SceneError::new(scene.store(), path, kind);
    let double = |name| {
        match time {
            Time::Default => view.read_value(name, value::read_double),
            Time::At {
                code,
                interpolation,
            } => view.read_value_at(name, code, interpolation, value::read_double),
        }
        .ok_or_else(|| error(SceneErrorKind::Parameter(name)))
    };
    let spec = if scene.is_a(path, BOX_SCHEMA) {
        let size = match time {
            Time::Default => view.read_value(SIZE, value::read_double3),
            Time::At {
                code,
                interpolation,
            } => view.read_value_at(SIZE, code, interpolation, value::read_double3),
        }
        .ok_or_else(|| error(SceneErrorKind::Parameter(SIZE)))?;
        PrimitiveSpec::Box { size }
    } else {
        let segments = view
            .read_value(SEGMENTS, value::read_uint)
            .ok_or_else(|| error(SceneErrorKind::Parameter(SEGMENTS)))?;
        PrimitiveSpec::Cylinder {
            radius: double(RADIUS)?,
            height: double(HEIGHT)?,
            segments,
        }
    };
    recipe(spec).map_err(|e| error(SceneErrorKind::Recipe(e)))
}
