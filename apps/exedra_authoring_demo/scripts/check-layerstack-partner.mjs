// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { readFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export const workspace = new URL("../../../", import.meta.url);
export const partner = new URL(".local/partners/layerstack/", workspace);
export const bindingPackage = new URL("packages/layerstack/", partner);

/** The Rust scene adapter and official TypeScript package use one source revision. */
export async function layerstackRevision() {
  const cargo = await readFile(new URL("Cargo.toml", workspace), "utf8");
  const entries = [
    ...cargo.matchAll(
      /^(layerstack(?:_[a-z0-9_]+)?)\s*=.*\brev\s*=\s*"([0-9a-f]{40})"/gm,
    ),
  ];
  const names = new Set(entries.map((match) => match[1]));
  for (const required of [
    "layerstack",
    "layerstack_schemas",
    "layerstack_usda",
    "layerstack_wasm",
  ]) {
    if (!names.has(required))
      throw new Error(`The native scene integration must pin ${required}`);
  }
  const pins = new Set(entries.map((match) => match[2]));
  if (pins.size !== 1)
    throw new Error(
      "All native Layerstack dependencies must share one exact revision",
    );
  return [...pins][0];
}

export async function checkPartner() {
  const revision = await layerstackRevision();
  const actual = execFileSync(
    "git",
    ["-C", fileURLToPath(partner), "rev-parse", "HEAD"],
    { encoding: "utf8" },
  ).trim();
  if (actual !== revision)
    throw new Error(
      `Layerstack TypeScript source ${actual} differs from Rust ${revision}`,
    );
  const dirty = execFileSync(
    "git",
    [
      "-C",
      fileURLToPath(partner),
      "status",
      "--porcelain",
      "--untracked-files=no",
      "--",
      "packages/layerstack",
    ],
    { encoding: "utf8" },
  ).trim();
  if (dirty)
    throw new Error(
      "Layerstack's official TypeScript package has uncommitted changes",
    );
  const pkg = JSON.parse(
    await readFile(new URL("package.json", bindingPackage), "utf8"),
  );
  if (
    pkg.name !== "@forest-rs/layerstack" ||
    Object.keys(pkg.dependencies ?? {}).length !== 0
  )
    throw new Error(
      "Expected Layerstack's official dependency-free TypeScript binding",
    );
  return revision;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (process.argv.includes("--revision")) {
    console.log(await layerstackRevision());
  } else {
    console.log(
      `Official TypeScript binding and Rust use Layerstack ${await checkPartner()}`,
    );
  }
}
