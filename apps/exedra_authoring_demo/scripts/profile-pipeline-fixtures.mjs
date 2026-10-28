// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Compare unchanged authored geometry across actual generated Wasm backends.
import assert from "node:assert/strict";
import {createHash} from "node:crypto";
import {readFile,writeFile,mkdir} from "node:fs/promises";
import {resolve,dirname} from "node:path";
import {pathToFileURL} from "node:url";
import * as THREE from "three";
import {GLTFExporter} from "three/examples/jsm/exporters/GLTFExporter.js";
import {AuthoringSession} from "@exedra/authoring";
import {authorFlangedSleeve} from "../src/sample.ts";
import {galleryCases} from "./gallery-cases.mjs";

const [wasmPath,outputPath,baselinePath]=process.argv.slice(2);
assert.ok(wasmPath&&outputPath,"expected Wasm package directory and output JSON, optionally a baseline JSON");
const baseline=baselinePath?JSON.parse(await readFile(baselinePath,"utf8")):undefined;
const baselineIds=baseline&&new Set(baseline.map(fixture=>fixture.sample));
const cases=baselineIds?galleryCases.filter(fixture=>baselineIds.has(fixture.id)):galleryCases;
if(baseline)assert.equal(cases.length,baseline.length,"Every unchanged baseline sample must still be represented");
const wasm=await import(pathToFileURL(resolve(wasmPath,"exedra_authoring_web_bridge.js")).href);
await wasm.default({module_or_path:await readFile(resolve(wasmPath,"exedra_authoring_web_bridge_bg.wasm"))});
// GLTFExporter's binary-only browser adapter; geometry and serialization stay real.
globalThis.FileReader??=class{result=null;onloadend=null;readAsArrayBuffer(blob){void blob.arrayBuffer().then(value=>{this.result=value;this.onloadend?.()})}};
const hash=bytes=>createHash("sha256").update(bytes).digest("hex");
const results=[];
const sharpSleeve=()=>authorFlangedSleeve({boreRadius:12,sleeveOuterRadius:18,flangeOuterRadius:22,shoulderHeight:8.75,height:35,angle:270});
for(const fixture of cases){
 const session=new AuthoringSession(new wasm.AuthoringKernel(),{units:"mm"});const document=session.build(fixture.id==="sleeve"?sharpSleeve():fixture.make());
 const scene=new THREE.Scene(),owned=[];const parts=[];
 try{
  for(const part of Object.values(document.parts)){
   const display=part.realize({tolerance:.12});assert.equal(display.report.withinTolerance,true);
   const geometry=new THREE.BufferGeometry();geometry.setAttribute("position",new THREE.BufferAttribute(display.positions,3));geometry.setAttribute("normal",new THREE.BufferAttribute(display.normals,3));geometry.setIndex(new THREE.BufferAttribute(display.indices,1));
   const material=new THREE.MeshStandardMaterial({color:0x87abb7});material.name="native";
   const mesh=new THREE.Mesh(geometry,material);mesh.name=part.name;scene.add(mesh);owned.push({geometry,material});
   parts.push({name:part.name,fingerprint:part.fingerprint,measurements:part.measurements,faces:part.faces().map(face=>({id:face.nativeId,role:face.role,surface:face.surface,features:face.features})),buffers:Object.fromEntries(["positions","normals","indices","nativeFaces"].map(key=>[key,hash(Buffer.from(display[key].buffer,display[key].byteOffset,display[key].byteLength))]))});
  }
  const glb=Buffer.from(await new GLTFExporter().parseAsync(scene,{binary:true}));
  results.push({sample:fixture.id,parts,glbSha256:hash(glb),glbBytes:glb.length});
  await mkdir(dirname(resolve(outputPath)),{recursive:true});await writeFile(resolve(dirname(outputPath),`fixture-${fixture.id}.glb`),glb);
 }finally{for(const item of owned){item.geometry.dispose();item.material.dispose()}document.dispose();session.dispose()}
}
if(baseline)assert.deepEqual(results,baseline,"Unchanged sample fingerprint, metadata, buffers or GLB changed");
await writeFile(outputPath,JSON.stringify(results,null,2));console.log(`Verified ${results.length} actual Wasm geometry/fingerprint/GLB fixtures${baselinePath?" against baseline":""}.`);
