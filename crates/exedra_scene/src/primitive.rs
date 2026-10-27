// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_compile::{PartSource, PreparedSource};
use exedra_constructive::ir::{
    Node, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder, RecipeError,
};
use layerstack::{
    EditTarget, LayerStore, PathId, PropertyDefinition, PropertyPath, PropertySpec, PropertyType,
    SchemaDefinition, SchemaRegistryBuilder, Specifier, TokenInterner, Transaction, Value,
};
use layerstack_schemas::{
    procedural::{ProceduralEvaluator, ProceduralInputs},
    value,
};

use crate::{SceneError, SceneErrorKind};

/// Typed box construction, minimum corner at the local origin, derived from `GenerativeProcedural`.
pub const BOX_SCHEMA: &str = "ExedraBox";
/// Typed capped +Z cylinder construction, base center at the local origin.
pub const CYLINDER_SCHEMA: &str = "ExedraCylinder";
/// Box extents as a varying `double3`, all strictly positive.
pub const SIZE: &str = "primvars:exedra:size";
/// Cylinder radius as a varying `double`, strictly positive.
pub const RADIUS: &str = "primvars:exedra:radius";
/// Cylinder height as a varying `double`, strictly positive.
pub const HEIGHT: &str = "primvars:exedra:height";
/// Cylinder side count as a uniform `uint`, at least three.
pub const SEGMENTS: &str = "primvars:exedra:segments";

/// Registers Exedra's typed construction schemas into a caller-owned registry.
///
/// Register the `OpenUSD` `UsdGeom` and `UsdProc` domains in the same builder.
/// All registry tokens must belong to the store used to compose the stage.
/// Dimensions have no fallback: missing construction intent is a refusal.
pub fn register_schemas(builder: &mut SchemaRegistryBuilder, tokens: &mut TokenInterner) {
    let procedural = tokens.intern("GenerativeProcedural");
    let system = tokens.intern("proceduralSystem");
    let box_system = tokens.intern("exedra:box");
    let cylinder_system = tokens.intern("exedra:cylinder");
    let box_schema = SchemaDefinition::typed(tokens.intern(BOX_SCHEMA))
        .with_parent(procedural)
        .with_override(
            PropertyDefinition::attribute(system)
                .with_type(PropertyType::parse("token").expect("registered USD value type"))
                .with_fallback(Value::Token(box_system)),
        )
        .with_property(
            PropertyDefinition::attribute(tokens.intern(SIZE))
                .with_type(PropertyType::parse("double3").expect("registered USD value type")),
        );
    let cylinder_schema = SchemaDefinition::typed(tokens.intern(CYLINDER_SCHEMA))
        .with_parent(procedural)
        .with_override(
            PropertyDefinition::attribute(system)
                .with_type(PropertyType::parse("token").expect("registered USD value type"))
                .with_fallback(Value::Token(cylinder_system)),
        )
        .with_property(
            PropertyDefinition::attribute(tokens.intern(RADIUS))
                .with_type(PropertyType::parse("double").expect("registered USD value type")),
        )
        .with_property(
            PropertyDefinition::attribute(tokens.intern(HEIGHT))
                .with_type(PropertyType::parse("double").expect("registered USD value type")),
        )
        .with_property(
            PropertyDefinition::attribute(tokens.intern(SEGMENTS))
                .with_type(PropertyType::parse("uint").expect("registered USD value type"))
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
    let system = match spec {
        PrimitiveSpec::Box { .. } => "exedra:box",
        PrimitiveSpec::Cylinder { .. } => "exedra:cylinder",
        _ => unreachable!("validated supported primitive"),
    };
    let system = store.tokens_mut().intern(system);
    let system_name = store.tokens_mut().intern("proceduralSystem");
    transaction.create_property(
        target.property(PropertyPath::new(path, system_name)),
        PropertySpec::typed_attribute(
            PropertyType::parse("token").expect("registered USD value type"),
        )
        .with_default(Value::Token(system)),
    );
    let mut property = |name: &str, type_name: &str, value: Value, uniform: bool| {
        let ty = PropertyType::parse(type_name).expect("registered USD value type");
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
    let root = builder.add_node(
        Node::new(NodeKind::Primitive {
            spec,
            placement: Placement3::IDENTITY,
        })
        .with_source(source),
    )?;
    builder.finish(root)
}

/// One typed recipe system; lowering owns no compiler or stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lowering {
    Box,
    Cylinder,
}

impl ProceduralEvaluator for Lowering {
    type Output = PreparedSource;
    type Error = SceneErrorKind;

    fn system(&self) -> &str {
        match self {
            Self::Box => "exedra:box",
            Self::Cylinder => "exedra:cylinder",
        }
    }

    fn evaluate(
        &self,
        inputs: &mut ProceduralInputs<'_>,
    ) -> Result<PreparedSource, SceneErrorKind> {
        let path = inputs.recipe();
        let mut read = |name| {
            inputs
                .attribute(path, name)
                .map_err(SceneErrorKind::Input)?
                .ok_or(SceneErrorKind::Parameter(name))
        };
        let spec = match self {
            Self::Box => {
                let value = read(SIZE)?;
                let size = value::read_double3(&value, inputs.tokens())
                    .ok_or(SceneErrorKind::Parameter(SIZE))?;
                PrimitiveSpec::Box { size }
            }
            Self::Cylinder => {
                let radius = read(RADIUS)?;
                let height = read(HEIGHT)?;
                let segments = read(SEGMENTS)?;
                PrimitiveSpec::Cylinder {
                    radius: value::read_double(&radius, inputs.tokens())
                        .ok_or(SceneErrorKind::Parameter(RADIUS))?,
                    height: value::read_double(&height, inputs.tokens())
                        .ok_or(SceneErrorKind::Parameter(HEIGHT))?,
                    segments: value::read_uint(&segments, inputs.tokens())
                        .ok_or(SceneErrorKind::Parameter(SEGMENTS))?,
                }
            }
        };
        recipe(spec)
            .map(|r| PreparedSource::new(PartSource::Recipe(r)))
            .map_err(SceneErrorKind::Recipe)
    }
}
