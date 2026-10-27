// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{boxed::Box, string::String, sync::Arc};
use core::fmt;
use exedra_constructive::{evaluate::GeometryReport, ir::RecipeError};
use layerstack::{EditError, LayerStore, PathId};

/// A failed scene evaluation or publication, with owned source context.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneError {
    /// Source prim spelling, independent of store-local handles.
    pub path: String,
    /// Structured refusal or underlying failure.
    pub kind: SceneErrorKind,
}

/// Why scene inputs could not produce a complete generated update.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SceneErrorKind {
    /// Compose with `UsdGeom`, `UsdProc` and Exedra primitive schemas.
    MissingSchemas,
    /// Incomplete composition must be repaired before generating geometry.
    Composition {
        /// Number of errors available through `Stage::composition_errors`.
        errors: usize,
    },
    /// Numeric sample times must be finite.
    InvalidTime,
    /// A composed parameter has no readable value at the requested time.
    Parameter(&'static str),
    /// This authored primitive kind has no scene schema in the current vocabulary.
    UnsupportedPrimitive,
    /// The ordinary recipe builder rejected the composed parameters.
    Recipe(RecipeError),
    /// The shared geometry compiler failed, retaining input and evaluation context.
    Compile(Box<exedra_compile::CompileError>),
    /// Evaluation refused publishable geometry; retains its complete report.
    Refused(Arc<GeometryReport>),
    /// One source must publish exactly one body at its generated child.
    BodyCount {
        /// Number of emitted bodies; split multi-body generators into distinct source prims.
        found: usize,
    },
    /// The supplied stage/store do not use this binding's original interners.
    ForeignStore,
    /// The publication layer is missing or its target cannot map an output.
    PublicationTarget,
    /// A tracked input has an incompatible kind or could not be decoded.
    Input(layerstack_schemas::procedural::ProceduralInputError),
    /// Authored dispatch does not match the typed primitive's evaluator.
    ProceduralSystem {
        /// System understood by this typed evaluator.
        expected: String,
        /// Composed system value, or absence.
        actual: Option<String>,
    },
    /// Native polygon validation or publication preparation failed.
    Mesh(layerstack_schemas::MeshPublicationError),
    /// Consumed input evidence no longer justifies the prepared publication.
    StaleInputs(String),
    /// An initial publication site already has geometry opinions owned elsewhere.
    OutputCollision,
    /// Generated output cannot become another construction input.
    GeneratedInput,
    /// Geometry contains nonfinite coordinates or exceeds USD index capacity.
    UnrepresentableGeometry,
    /// Native atomic publication rejected the transaction.
    Publish(EditError),
}

impl SceneError {
    pub(crate) fn new(store: &dyn LayerStore, path: PathId, kind: SceneErrorKind) -> Self {
        Self {
            path: store.paths().display(path, store.tokens()),
            kind,
        }
    }
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.path.is_empty() {
            write!(f, "{}: ", self.path)?;
        }
        match &self.kind {
            SceneErrorKind::MissingSchemas => f.write_str("missing scene schemas"),
            SceneErrorKind::Composition { errors } => write!(f, "{errors} composition errors"),
            SceneErrorKind::InvalidTime => f.write_str("nonfinite sample time"),
            SceneErrorKind::Parameter(name) => write!(f, "unreadable parameter {name}"),
            SceneErrorKind::UnsupportedPrimitive => f.write_str("unsupported scene primitive kind"),
            SceneErrorKind::Recipe(error) => error.fmt(f),
            SceneErrorKind::Compile(error) => error.fmt(f),
            SceneErrorKind::Refused(_) => f.write_str("constructive evaluation refused geometry"),
            SceneErrorKind::BodyCount { found } => {
                write!(f, "one scene source needs one body, found {found}")
            }
            SceneErrorKind::ForeignStore => f.write_str("scene binding belongs to another store"),
            SceneErrorKind::PublicationTarget => f.write_str("invalid publication edit target"),
            SceneErrorKind::Input(error) => error.fmt(f),
            SceneErrorKind::ProceduralSystem { expected, actual } => {
                write!(f, "procedural system {actual:?}, expected {expected}")
            }
            SceneErrorKind::Mesh(error) => error.fmt(f),
            SceneErrorKind::StaleInputs(reason) => write!(f, "stale publication inputs: {reason}"),
            SceneErrorKind::OutputCollision => {
                f.write_str("publication site already has geometry opinions")
            }
            SceneErrorKind::GeneratedInput => {
                f.write_str("generated output cannot be construction input")
            }
            SceneErrorKind::UnrepresentableGeometry => {
                f.write_str("geometry exceeds finite USD representation")
            }
            SceneErrorKind::Publish(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for SceneError {}
