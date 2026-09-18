// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::boxed::Box;

use crate::ir::{NodeId, NodeKind, Placement3, ProfileId, Recipe};
use crate::loft::LoftError;
use crate::tessellate::TessellateError;

/// Detached authored context for one section involved in a loft refusal.
#[derive(Clone, Debug, PartialEq)]
pub struct LoftSectionContext {
    /// Section index within the failed loft.
    pub index: usize,
    /// Recipe-local profile id.
    pub profile: ProfileId,
    /// Resolved section label, if authored; never inferred from numeric ids.
    pub source: Option<Box<str>>,
    /// Authored placement relative to the loft's incoming construction frame.
    /// The geometric witness uses the tessellation invocation's construction frame,
    /// which may include transforms absent from this authored placement.
    pub placement: Placement3,
}

/// Hard evaluation failure with owned author-facing context.
///
/// Returned context remains usable after the recipe is dropped. Numeric ids
/// remain recipe-scoped; labels are opaque references, not persistent selections.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalError {
    /// The failing node.
    pub node: NodeId,
    /// Resolved source bound directly to the failing node, when authored.
    /// An unlabeled child does not inherit a parent's label.
    pub source: Option<Box<str>>,
    /// Both authored sections of a refused smooth-loft band, in order.
    pub loft_sections: Option<Box<[LoftSectionContext; 2]>>,
    /// The underlying tessellation or retained-operation failure and evidence.
    pub error: TessellateError,
}

impl EvalError {
    // Evaluation enriches failures once at its public boundary, after internal
    // handling of recoverable refusals. Successful paths allocate no context.
    pub(super) fn new(node: NodeId, error: TessellateError) -> Self {
        Self {
            node,
            source: None,
            loft_sections: None,
            error,
        }
    }

    pub(super) fn with_recipe_context(mut self, recipe: &Recipe) -> Self {
        self.source = recipe.source_of(self.node).map(Box::from);
        if let TessellateError::Loft(LoftError::Foldover { band, .. }) = &self.error
            && let Some(NodeKind::Loft { sections, .. }) = recipe.node(self.node).map(|n| &n.kind)
            && let Some(pair) = sections.get(*band..).and_then(|tail| tail.get(..2))
        {
            self.loft_sections = Some(Box::new(core::array::from_fn(|i| {
                let section = &pair[i];
                LoftSectionContext {
                    index: band + i,
                    profile: section.profile,
                    source: section
                        .source
                        .and_then(|id| recipe.source(id))
                        .map(Box::from),
                    placement: section.placement,
                }
            })));
        }
        self
    }
}

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "node {:?}", self.node)?;
        if let Some(source) = &self.source {
            write!(f, " ({source:?})")?;
        }
        write!(f, " failed to evaluate: {}", self.error)?;
        if let Some(sections) = &self.loft_sections {
            for section in sections.iter() {
                write!(f, "; section {}", section.index)?;
                if let Some(source) = &section.source {
                    write!(f, " ({source:?})")?;
                }
            }
        }
        Ok(())
    }
}

impl core::error::Error for EvalError {}
