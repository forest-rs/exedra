// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import assert from "node:assert/strict";
import { test } from "node:test";
import { checkLockfile } from "./check-lockfile.mjs";

test("public packages and the explicit authored SDK/native stage links remain portable", () => {
  const packages = {
    "node_modules/three": { resolved: "https://registry.npmjs.org/three/-/three-0.179.1.tgz", integrity: "sha512-fixture" },
    "node_modules/@exedra/authoring": { resolved: "../../packages/exedra", link: true },
    "node_modules/@layerstack/client": { resolved: "../../.local/partners/layerstack/packages/client", link: true },
  };
  assert.deepEqual(checkLockfile({ packages }), []);
});

test("a developer registry, credentials, unexpected local dependency or absent integrity is refused", () => {
  for (const resolved of [
    "https://private-registry.example/three/-/three-0.179.1.tgz",
    "https://user:password@registry.npmjs.org/three/-/three-0.179.1.tgz",
    "file:../../unexpected-package",
  ]) {
    assert.ok(checkLockfile({ packages: { "node_modules/three": { resolved, integrity: "sha512-fixture" } } }).length);
  }
  assert.ok(checkLockfile({ packages: { "node_modules/three": { resolved: "https://registry.npmjs.org/three/-/three-0.179.1.tgz" } } }).length);
});
