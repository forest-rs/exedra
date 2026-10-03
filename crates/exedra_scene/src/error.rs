// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{boxed::Box, rc::Rc, string::String};
use core::fmt;
use exedra_constructive::{
    evaluate::{EvalError, GeometryReport},
    ir::RecipeError,
};
use layerstack::{ChangeHistoryError, EditError, LayerStore, PathId};

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
    /// Compose the stage with `OpenUSD` geometry and Exedra primitive schemas.
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
    /// The ordinary constructive evaluator failed.
    Evaluate(Box<EvalError>),
    /// Evaluation refused publishable geometry; retains its complete report.
    Refused(Rc<GeometryReport>),
    /// Generation requires an empty, exclusively owned, weakest local layer.
    GeneratedLayer,
    /// The generated layer was edited outside this evaluator.
    GeneratedLayerEdited,
    /// A reserved generated-child path already has authored content.
    OutputCollision,
    /// Generated output cannot become another construction input.
    GeneratedInput,
    /// Descendant authoring beneath native instances is not supported yet.
    NativeInstance,
    /// Geometry contains nonfinite coordinates or exceeds USD index capacity.
    UnrepresentableGeometry,
    /// The evaluator was used with a different live stage.
    Stage(ChangeHistoryError),
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
        write!(f, "{}: ", self.path)?;
        match &self.kind {
            SceneErrorKind::MissingSchemas => f.write_str("missing scene schemas"),
            SceneErrorKind::Composition { errors } => write!(f, "{errors} composition errors"),
            SceneErrorKind::InvalidTime => f.write_str("nonfinite sample time"),
            SceneErrorKind::Parameter(name) => write!(f, "unreadable parameter {name}"),
            SceneErrorKind::UnsupportedPrimitive => f.write_str("unsupported scene primitive kind"),
            SceneErrorKind::Recipe(error) => error.fmt(f),
            SceneErrorKind::Evaluate(error) => error.fmt(f),
            SceneErrorKind::Refused(_) => f.write_str("constructive evaluation refused geometry"),
            SceneErrorKind::GeneratedLayer => {
                f.write_str("invalid generated-layer ownership or strength")
            }
            SceneErrorKind::GeneratedLayerEdited => {
                f.write_str("generated layer changed outside its evaluator")
            }
            SceneErrorKind::OutputCollision => f.write_str("generated child is already authored"),
            SceneErrorKind::GeneratedInput => {
                f.write_str("generated output cannot be construction input")
            }
            SceneErrorKind::NativeInstance => {
                f.write_str("native instance descendant generation unsupported")
            }
            SceneErrorKind::UnrepresentableGeometry => {
                f.write_str("geometry exceeds finite USD representation")
            }
            SceneErrorKind::Stage(error) => write!(f, "live stage changed: {error:?}"),
            SceneErrorKind::Publish(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for SceneError {}
