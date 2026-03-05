import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const root = process.cwd();
const distDir = join(root, "dist");
const assetsDir = join(distDir, "assets");

function expectFile(path, description) {
  const stat = statSync(path, { throwIfNoEntry: false });
  if (!stat || !stat.isFile()) {
    throw new Error(`missing ${description}: ${path}`);
  }
}

expectFile(join(distDir, "index.html"), "dist index");
const assetNames = readdirSync(assetsDir);
const hasWasm = assetNames.some((name) => /^cambium_web_bridge_bg-.*\.wasm$/.test(name));
const hasJs = assetNames.some((name) => /^index-.*\.js$/.test(name));

if (!hasWasm) {
  throw new Error("missing wasm asset in dist/assets");
}
if (!hasJs) {
  throw new Error("missing viewer JS bundle in dist/assets");
}

console.log("dist smoke check passed");
