// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { layerstackRevision, workspace } from "./check-layerstack-partner.mjs";

// Layerstack's public version report identifies the exact Rust source in this
// combined module. Both APIs operate on its one native stage and memory.
execFileSync(
  "wasm-pack",
  [
    "build",
    fileURLToPath(new URL("apps/exedra_scene_web_bridge/", workspace)),
    "--release",
    "--target",
    "web",
    "--out-dir",
    fileURLToPath(new URL("packages/exedra/dist/scene_wasm/", workspace)),
  ],
  {
    cwd: fileURLToPath(workspace),
    env: { ...process.env, LAYERSTACK_REVISION: await layerstackRevision() },
    stdio: "inherit",
  },
);
