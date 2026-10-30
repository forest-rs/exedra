// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { copyFile, mkdir } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  bindingPackage,
  checkPartner,
  workspace,
} from "./check-layerstack-partner.mjs";

await checkPartner();
const generated = new URL("packages/exedra/dist/scene_wasm/", workspace);
const bindingGlue = new URL("wasm/", bindingPackage);
await mkdir(bindingGlue, { recursive: true });

// Build the official package against the actual unchanged glue of our combined
// module. The application uses its public Layerstack constructor over that
// already initialized module, so both APIs share one stage and memory.
await copyFile(
  new URL("exedra_scene_web_bridge.js", generated),
  new URL("layerstack.js", bindingGlue),
);
await copyFile(
  new URL("exedra_scene_web_bridge.d.ts", generated),
  new URL("layerstack.d.ts", bindingGlue),
);
// Preserve the generated default-loader URL, even though our app supplies the
// initialized module rather than loading another instance.
await copyFile(
  new URL("exedra_scene_web_bridge_bg.wasm", generated),
  new URL("exedra_scene_web_bridge_bg.wasm", bindingGlue),
);
// The official convenience loader has its own documented default URL. Keep it
// valid too. Both filenames contain the same combined module; production asset
// checks verify that bundling does not ship an additional standalone module.
await copyFile(
  new URL("exedra_scene_web_bridge_bg.wasm", generated),
  new URL("layerstack_bg.wasm", bindingGlue),
);
execFileSync("npm", ["run", "build"], {
  cwd: fileURLToPath(bindingPackage),
  stdio: "inherit",
});
