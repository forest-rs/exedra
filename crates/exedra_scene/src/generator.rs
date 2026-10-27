// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{BOX_SCHEMA, CYLINDER_SCHEMA, SceneErrorKind};
use alloc::{boxed::Box, rc::Rc, string::String, sync::Arc};
use core::fmt;
use exedra_compile::PreparedSource;
use layerstack_schemas::procedural::{ProceduralEvaluator, ProceduralInputs};

/// Host generator definition recorded beside each retained source.
///
/// The revision identifies generator code/configuration for inspection; it is an
/// opaque equality label, not a clock. Registering a replacement always resets
/// its tracked bindings, even when the host repeats this revision.
/// These fields are inspection labels, not a fingerprint of arbitrary captured
/// host state. Shared definition identity is exposed by `SourceGeometry`.
#[derive(Clone, Debug)]
pub struct GeneratorIdentity {
    /// Registered native schema type used for discovery.
    pub schema: String,
    /// Exact `proceduralSystem` understood by this definition.
    pub system: String,
    /// Caller-selected code/configuration revision.
    pub revision: u64,
}

/// Work caused by installing one immutable generator definition.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GeneratorRegistration {
    /// Whether this replaces a previous host definition for this schema.
    pub replaced: bool,
    /// Tracked source bindings reset; published output ownership stays retained.
    pub invalidated_sources: usize,
}

/// Invalid host generator registration; the existing definition stays installed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GeneratorRegistrationError {
    /// A generator must select a nonempty registered schema name.
    EmptySchema,
    /// A generator must understand a nonempty procedural system token.
    EmptySystem,
    /// Built-in box/cylinder definitions retain their documented meaning.
    ReservedSchema,
}
impl fmt::Display for GeneratorRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptySchema => "scene generator needs a schema name",
            Self::EmptySystem => "scene generator needs a procedural system",
            Self::ReservedSchema => "built-in scene generator schemas cannot be replaced",
        })
    }
}
impl core::error::Error for GeneratorRegistrationError {}

trait Lower {
    fn evaluate(&self, inputs: &mut ProceduralInputs<'_>)
    -> Result<PreparedSource, SceneErrorKind>;
}
impl<E: ProceduralEvaluator<Output = PreparedSource, Error = SceneErrorKind>> Lower for E {
    fn evaluate(
        &self,
        inputs: &mut ProceduralInputs<'_>,
    ) -> Result<PreparedSource, SceneErrorKind> {
        ProceduralEvaluator::evaluate(self, inputs)
    }
}
struct Definition {
    identity: Arc<GeneratorIdentity>,
    lower: Box<dyn Lower>,
}
#[derive(Clone)]
pub(crate) struct Generator(Rc<Definition>);
impl fmt::Debug for Generator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Generator").field(&self.0.identity).finish()
    }
}
impl Generator {
    pub(crate) fn new<E>(schema: &str, revision: u64, evaluator: E) -> Self
    where
        E: ProceduralEvaluator<Output = PreparedSource, Error = SceneErrorKind> + 'static,
    {
        Self(Rc::new(Definition {
            identity: Arc::new(GeneratorIdentity {
                schema: schema.into(),
                system: evaluator.system().into(),
                revision,
            }),
            lower: Box::new(evaluator),
        }))
    }
    pub(crate) fn validate<E>(schema: &str, evaluator: &E) -> Result<(), GeneratorRegistrationError>
    where
        E: ProceduralEvaluator<Output = PreparedSource, Error = SceneErrorKind>,
    {
        if schema.is_empty() {
            return Err(GeneratorRegistrationError::EmptySchema);
        }
        if [BOX_SCHEMA, CYLINDER_SCHEMA].contains(&schema) {
            return Err(GeneratorRegistrationError::ReservedSchema);
        }
        if evaluator.system().is_empty() {
            return Err(GeneratorRegistrationError::EmptySystem);
        }
        Ok(())
    }
    pub(crate) fn identity(&self) -> &Arc<GeneratorIdentity> {
        &self.0.identity
    }
    pub(crate) fn same_definition(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl ProceduralEvaluator for Generator {
    type Output = PreparedSource;
    type Error = SceneErrorKind;
    fn system(&self) -> &str {
        &self.0.identity.system
    }
    fn evaluate(
        &self,
        inputs: &mut ProceduralInputs<'_>,
    ) -> Result<PreparedSource, SceneErrorKind> {
        self.0.lower.evaluate(inputs)
    }
}
