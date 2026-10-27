// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

/** Public dependencies must install on a clean machine without registry credentials. */
export function checkLockfile(lock) {
  const errors = [];
  for (const [name, entry] of Object.entries(lock.packages ?? {})) {
    if (!entry.resolved) continue;
    if (name === "node_modules/@exedra/authoring" && entry.link === true && entry.resolved === "../../packages/exedra") continue;
    let url;
    try { url = new URL(entry.resolved); } catch { errors.push(`${name}: unexpected local dependency`); continue; }
    if (url.protocol !== "https:" || url.hostname !== "registry.npmjs.org" || url.username || url.password || url.port)
      errors.push(`${name}: dependency must resolve from the public npm registry`);
    if (!entry.integrity) errors.push(`${name}: dependency integrity is missing`);
  }
  return errors;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const lock = JSON.parse(await readFile(new URL("../package-lock.json", import.meta.url), "utf8"));
  const errors = checkLockfile(lock);
  if (errors.length) {
    console.error(errors.join("\n"));
    process.exitCode = 1;
  } else {
    console.log("Browser dependency lockfile uses integrity-checked public npm packages and the local SDK.");
  }
}
