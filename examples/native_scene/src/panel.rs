// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_analytic::{
    End,
    extrusion::{Extrusion, Pocket},
};
use exedra_compile::{PartSource, PreparedSource};
use exedra_constructive::builders;
use exedra_curves::profile::{ProfilePolicy, SegTag};
use exedra_math::Placement3;
use exedra_scene::{
    SceneErrorKind,
    composition::*,
    schemas::{
        procedural::{ProceduralEvaluator, ProceduralInputs},
        value,
    },
};

pub(super) const SCHEMA: &str = "ExampleNativePanel";
pub(super) const SYSTEM: &str = "example:nativePanel";
pub(super) const WIDTH: &str = "primvars:example:width";
pub(super) const DEPTH: &str = "primvars:example:depth";
pub(super) const VENTS: SegTag = SegTag(2);
pub(super) const RECESS: SegTag = SegTag(3);

#[derive(Copy, Clone, Debug)]
pub(super) struct PanelGenerator;
impl ProceduralEvaluator for PanelGenerator {
    type Output = PreparedSource;
    type Error = SceneErrorKind;
    fn system(&self) -> &str {
        SYSTEM
    }
    fn evaluate(
        &self,
        inputs: &mut ProceduralInputs<'_>,
    ) -> Result<PreparedSource, SceneErrorKind> {
        let mut read = |name| {
            let raw = inputs
                .attribute(inputs.recipe(), name)
                .map_err(SceneErrorKind::Input)?
                .ok_or(SceneErrorKind::Parameter(name))?;
            value::read_double(&raw, inputs.tokens()).ok_or(SceneErrorKind::Parameter(name))
        };
        let width = read(WIDTH)?;
        let depth = read(DEPTH)?;
        let make = || -> Result<Extrusion, Box<dyn std::error::Error>> {
            let policy = ProfilePolicy::new(1e-6);
            let vent = builders::slot_centered(22.0, 3.0)?.tagged(VENTS);
            let profile = builders::rounded_rect_centered(width, 80.0, 6.0)?
                .tagged(SegTag(1))
                .with_cutouts_at(
                    &vent,
                    [(-36.0, 22.0), (-12.0, 22.0), (12.0, 22.0), (36.0, 22.0)],
                )?
                .to_profile(&policy)?;
            let recess = builders::slot_centered(48.0, 8.0)?
                .translated((0.0, -10.0))?
                .to_profile(&policy)?;
            Ok(
                Extrusion::new(profile, Placement3::IDENTITY, 12.0)?.with_pocket(
                    Pocket::from_profile(&recess, End::End, depth)?.tagged(RECESS),
                    &policy,
                )?,
            )
        };
        make()
            .map(|solid| PreparedSource::new(PartSource::Native(solid.into())))
            .map_err(|error| SceneErrorKind::Generator {
                code: "example.panel",
                message: error.to_string(),
            })
    }
}
pub(super) fn register(builder: &mut SchemaRegistryBuilder, tokens: &TokenInterner) {
    let system = tokens.intern(SYSTEM);
    builder.register(
        SchemaDefinition::typed(tokens.intern(SCHEMA))
            .with_parent(tokens.intern("GenerativeProcedural"))
            .with_override(
                PropertyDefinition::attribute(tokens.intern("proceduralSystem"))
                    .with_type(PropertyType::parse("token").unwrap())
                    .with_fallback(Value::Token(system)),
            )
            .with_property(
                PropertyDefinition::attribute(tokens.intern(WIDTH))
                    .with_type(PropertyType::parse("double").unwrap()),
            )
            .with_property(
                PropertyDefinition::attribute(tokens.intern(DEPTH))
                    .with_type(PropertyType::parse("double").unwrap()),
            ),
    );
}
