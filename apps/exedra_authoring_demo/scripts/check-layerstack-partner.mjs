// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { readFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const workspace = new URL("../../../", import.meta.url);
const cargo = await readFile(new URL("Cargo.toml", workspace), "utf8");
const pins = [...cargo.matchAll(/^layerstack(?:_schemas|_usda)?\s*=.*\brev\s*=\s*"([0-9a-f]{40})"/gm)].map(match => match[1]);
if (pins.length !== 3 || new Set(pins).size !== 1) throw new Error("All three native Layerstack dependencies must share one exact revision");
const revision = pins[0];
if (process.argv.includes("--revision")) {
  console.log(revision);
} else {
  const partner = new URL(".local/partners/layerstack/", workspace);
  const actual = execFileSync("git", ["-C", fileURLToPath(partner), "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  if (actual !== revision) throw new Error(`Layerstack JS partner ${actual} differs from Rust ${revision}`);
  const dirty = execFileSync("git", ["-C", fileURLToPath(partner), "status", "--porcelain", "--untracked-files=no", "--", "packages/client"], { encoding: "utf8" }).trim();
  if (dirty) throw new Error("Layerstack JS partner has uncommitted client changes");
  const pkg = JSON.parse(await readFile(new URL("packages/client/package.json", partner), "utf8"));
  if (pkg.name !== "@layerstack/client" || Object.keys(pkg.dependencies ?? {}).length !== 0) throw new Error("Expected Layerstack's dependency-free native stage client");
  console.log(`Native stage client and Rust use Layerstack ${revision}`);
}
