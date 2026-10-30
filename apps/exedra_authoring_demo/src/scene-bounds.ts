// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import type { InstancedMesh } from "three";

/** Refresh broad-phase bounds after a prototype or occurrence changes. */
export function updateInstanceBounds(
  mesh: InstancedMesh,
  geometryChanged: boolean,
  transformsChanged: boolean,
): void {
  if (!geometryChanged && !transformsChanged) return;
  mesh.boundingBox = null;
  mesh.computeBoundingSphere();
}
