// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared native panel provider for the command-line and browser scene examples.
//!
//! This unpublished application library reuses one construction definition. It
//! does not own a stage or add panel-specific vocabulary to the scene adapter.

mod panel;

use exedra_scene::{
    GeneratorRegistration, GeneratorRegistrationError, SceneEvaluator, composition::*,
};

/// Demo typed procedural schema.
pub const PANEL_SCHEMA: &str = panel::SCHEMA;
/// Demo procedural system token.
pub const PANEL_SYSTEM: &str = panel::SYSTEM;
/// Width property consumed through tracked native composition reads.
pub const PANEL_WIDTH: &str = panel::WIDTH;
/// Blind recess depth consumed through tracked native composition reads.
pub const PANEL_DEPTH: &str = panel::DEPTH;

/// Registers the shared example's typed native schema in a caller's registry.
pub fn register_panel_schema(builder: &mut SchemaRegistryBuilder, tokens: &TokenInterner) {
    panel::register(builder, tokens);
}

/// Installs the example's immutable generator on the caller's scene adapter.
///
/// # Errors
/// Propagates invalid generator registration without changing the definition.
pub fn register_panel_generator(
    evaluator: &mut SceneEvaluator,
) -> Result<GeneratorRegistration, GeneratorRegistrationError> {
    evaluator.register_generator(panel::SCHEMA, 1, panel::PanelGenerator)
}
