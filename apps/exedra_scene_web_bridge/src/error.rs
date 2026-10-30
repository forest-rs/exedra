// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

pub(crate) fn refusal(
    code: &'static str,
    message: impl Into<String>,
) -> Box<dyn std::error::Error> {
    Box::new(layerstack_wasm::Error::new(code, message))
}

pub(crate) fn native(error: Box<dyn std::error::Error>) -> layerstack_wasm::Error {
    match error.downcast::<layerstack_wasm::Error>() {
        Ok(error) => *error,
        Err(error) => layerstack_wasm::Error::new("scene.operation", error.to_string()),
    }
}
