// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { AuthoringSession, ExedraError, extrude, placement, profiles } from "@exedra/authoring";
import { AuthoringKernel, default as initialize } from "../../../packages/exedra/dist/wasm_pkg/exedra_authoring_web_bridge.js";
import { authorPanel, authorFlangedSleeve } from "../src/sample.ts";
import { galleryCases } from "./gallery-cases.mjs";

await initialize({module_or_path:await readFile(new URL("../../../packages/exedra/dist/wasm_pkg/exedra_authoring_web_bridge_bg.wasm",import.meta.url))});
const timestamp="2026-10-27T00:00:00Z";
const policy={tolerance:0.12};
const faceAttributes=part=>part.faces().map(face=>({id:face.nativeId,features:face.features,role:face.role,surface:face.surface}));
function equalDisplay(a,b) {
  for(const key of ["positions","normals","indices","nativeFaces"])assert.deepEqual(a[key],b[key],`same-policy ${key} changed`);
}
function within(display) {
  assert.equal(display.report.withinTolerance,true,"must-succeed display did not meet tolerance");
  assert.equal(display.nativeFaces.length*3,display.indices.length);
  assert.ok(display.timing.kernelMs>=0&&Number.isFinite(display.timing.kernelMs));
}
const results=[];
for(const sample of galleryCases) {
  const session=new AuthoringSession(new AuthoringKernel(),{units:"mm"});
  let original,fresh;
  try {
    original=session.build(sample.make());
    const held=new Map();
    for(const part of Object.values(original.parts)) {
      assert.equal(part.measurements.volume.status,sample.measures??"available");assert.equal(part.measurements.surfaceArea.status,sample.measures??"available");
      assert.equal(part.stepSupport.status,sample.step);
      const first=part.realize(policy),repeat=part.realize(policy);within(first);within(repeat);equalDisplay(first,repeat);
      assert.equal(repeat.report.cacheHits,1);
      held.set(part.name,{part,first,selection:first.pickTriangle(0),attributes:faceAttributes(part)});
    }
    fresh=session.build(sample.make());
    for(const part of Object.values(fresh.parts)) {
      const previous=held.get(part.name),display=part.realize(policy);within(display);equalDisplay(previous.first,display);
      assert.equal(part.fingerprint,previous.part.fingerprint);assert.deepEqual(part.measurements,previous.part.measurements);assert.deepEqual(faceAttributes(part),previous.attributes);
      assert.equal(display.report.cacheHits,1);
      assert.throws(()=>part.requireSelection(previous.selection),error=>error instanceof ExedraError&&error.diagnostic.code==="selection.stale");
      if(sample.step==="supported") {
        const file=part.exportStep({timestamp}),old=previous.part.exportStep({timestamp});
        assert.equal(file.text,old.text);assert.equal(file.report.units,"mm");assert.equal(file.report.faces,part.measurements.faces);
        for(const entity of file.report.correspondence.faces)if(entity!==null)assert.match(file.text,new RegExp(`#${entity}=ADVANCED_FACE\\(`));
        assert.equal(file.timing.outcome,"success");
      } else {
        assert.throws(()=>part.exportStep({timestamp}),error=>error instanceof ExedraError&&error.diagnostic.code==="exchange.unsupportedSurface"&&error.diagnostic.face===part.stepSupport.diagnostic.face&&error.diagnostic.geometryFamily===part.stepSupport.diagnostic.geometryFamily&&error.timing.outcome==="refused");
      }
    }
    fresh.dispose();fresh=undefined;
    const fine=[];
    for(const {part,selection} of held.values()) {
      const display=part.realize({tolerance:0.035});within(display);part.requireSelection(selection);
      const faces=new Set(part.faces().map(face=>face.nativeId));assert.ok(display.nativeFaces.every(id=>faces.has(id)));
      fine.push({part:part.name,triangles:display.report.triangles,kernelMs:display.timing.kernelMs,transferMs:display.timing.transferMs});
    }
    const latest=session.lastOperation;session.stats();assert.strictEqual(session.lastOperation,latest,"diagnostic reads changed timing state");
    original.dispose();original=undefined;
    for(const {first} of held.values())assert.throws(()=>first.pickTriangle(0),error=>error instanceof ExedraError&&error.diagnostic.code==="lifecycle.documentDisposed");
    const stats=session.stats();assert.equal(stats.documents,0);assert.equal(stats.nativeParts,0);assert.equal(stats.cacheEntries,0);
    results.push({sample:sample.id,outcome:"success",step:sample.step,fine,remainingDocuments:stats.documents});
  } finally {fresh?.dispose();original?.dispose();session.dispose();}
}
// Explicit non-success inventory: invalid authoring and finite-precision output
// are different from unsupported STEP geometry and successful target accuracy.
const session=new AuthoringSession(new AuthoringKernel(),{units:"mm"});
try {
  const good=session.build(galleryCases[0].make());
  assert.throws(()=>session.build(authorPanel({width:120,height:80,thickness:12,pocketDepth:18,cornerRadius:6,ventRadius:3})),error=>error instanceof ExedraError&&error.diagnostic.code==="pocket.invalid"&&error.timing.outcome==="refused");
  assert.equal(session.stats().documents,1);good.parts.panel.realize(policy);good.dispose();
  const far=session.build({plate:extrude(profiles.rectangle({width:4,height:4}),{height:10,placement:placement({translation:[1_000_000.03,0,0]})})});
  const limited=far.parts.plate.realize({tolerance:0.01});assert.equal(limited.report.withinTolerance,false);assert.equal(limited.timing.outcome,"success");assert.equal(far.parts.plate.measurements.volume.value,160);far.dispose();
  const huge=session.build({plate:extrude(profiles.rectangle({width:1e100,height:1e100}),{height:1e200})});
  assert.equal(huge.parts.plate.measurements.volume.status,"numericLimit");assert.equal(huge.parts.plate.measurements.surfaceArea.status,"available");huge.dispose();
  assert.equal(session.stats().documents,0);assert.equal(session.stats().cacheEntries,0);
  results.push({sample:"recovery-and-availability",outcomes:["invalid-authoring-refused","accuracy-target-unmet","native-measure-numeric-limit"]});
} finally {session.dispose();}

// Independent shoulder/radius controls: exact native integrals verify each dimension.
const flangeBase={boreRadius:12,sleeveOuterRadius:18,flangeOuterRadius:22,shoulderHeight:8.75,height:35,angle:270};
const flangeVolume=p=>p.angle*Math.PI/360*((p.flangeOuterRadius**2-p.boreRadius**2)*p.shoulderHeight+(p.sleeveOuterRadius**2-p.boreRadius**2)*(p.height-p.shoulderHeight));
const flangeSession=new AuthoringSession(new AuthoringKernel(),{units:"mm"});
try {
 for(const edit of [{},{height:40},{shoulderHeight:10},{sleeveOuterRadius:20},{flangeOuterRadius:25},{boreRadius:13},{angle:180},{sleeveOuterRadius:22}]){
  const p={...flangeBase,...edit},document=flangeSession.build(authorFlangedSleeve(p));
  try {const part=document.parts.sleeve;assert.equal(part.measurements.volume.status,"available");assert.ok(Math.abs(part.measurements.volume.value-flangeVolume(p))<1e-8);assert.ok(Math.abs(part.measurements.bounds[1][2]-p.height)<1e-8);assert.ok(Math.abs(part.measurements.bounds[1][0]-Math.max(p.flangeOuterRadius,p.sleeveOuterRadius))<1e-8);}finally{document.dispose();}
 }
 const good=flangeSession.build(authorFlangedSleeve(flangeBase));
 assert.throws(()=>flangeSession.build(authorFlangedSleeve({...flangeBase,boreRadius:20})),error=>error instanceof ExedraError&&error.diagnostic.code==="profile.invalid");
 assert.throws(()=>authorFlangedSleeve({...flangeBase,shoulderHeight:40}),error=>error instanceof ExedraError&&error.diagnostic.code==="input.flangeHeight");
 good.parts.sleeve.realize(policy);good.dispose();assert.equal(flangeSession.stats().documents,0);assert.equal(flangeSession.stats().cacheEntries,0);
 results.push({sample:"independent-flange-dimensions",outcome:"success",independentEdits:8,defaultVolume:flangeVolume(flangeBase)});
}finally{flangeSession.dispose();}

console.log(JSON.stringify({samples:results},null,2));
