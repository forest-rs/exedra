// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { AuthoringSession, ExedraError, extrude, revolve, placement, profiles, charts, uvTransform } from "@exedra/authoring";
import { AuthoringKernel, default as initialize } from "../../../packages/exedra/dist/wasm_pkg/exedra_authoring_web_bridge.js";
import { authorPanel, authorFlangedSleeve, authorSphere, authorDome, authorPointedCone } from "../src/sample.ts";
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

// Render vertices may split at UV/normal seams; closure is checked by exact
// emitted position identities, independently of the native edge tables.
function closedDisplay(display) {
  const point=index=>display.positions.slice(3*index,3*index+3);
  const key=p=>Array.from(p,value=>Object.is(value,-0)?0:value).join(",");
  const edges=new Map();
  for(let i=0;i<display.indices.length;i+=3){
    const indices=Array.from(display.indices.slice(i,i+3)),points=indices.map(point),keys=points.map(key);
    const ab=points[1].map((value,j)=>value-points[0][j]),ac=points[2].map((value,j)=>value-points[0][j]);
    const cross=[ab[1]*ac[2]-ab[2]*ac[1],ab[2]*ac[0]-ab[0]*ac[2],ab[0]*ac[1]-ab[1]*ac[0]];
    assert.ok(Math.hypot(...cross)>0,"zero-area pole triangle");
    const normal=[0,1,2].map(j=>indices.reduce((sum,index)=>sum+display.normals[3*index+j],0));
    assert.ok(cross.reduce((sum,value,j)=>sum+value*normal[j],0)>0,"triangle winding disagrees with emitted normals");
    for(let j=0;j<3;j++){
      const a=keys[j],b=keys[(j+1)%3],edge=a<b?`${a}|${b}`:`${b}|${a}`;
      const use=edges.get(edge)??[0,0];use[Number(a>b)]++;edges.set(edge,use);
    }
  }
  for(const use of edges.values())assert.deepEqual(use,[1,1],"emitted position mesh must be closed and oriented");
  for(let i=0;i<display.normals.length;i+=3)assert.ok(Math.abs(Math.hypot(...display.normals.slice(i,i+3))-1)<1e-6,"unit native normal");
}

const poleSession=new AuthoringSession(new AuthoringKernel(),{units:"mm"});
try {
 const radius=25,height=40;
 for(const fixture of [
  {id:"sphere",make:()=>authorSphere({radius}),volume:4*Math.PI*radius**3/3,area:4*Math.PI*radius**2,collapsed:2,surface:"sphere"},
  {id:"dome",make:()=>authorDome({radius,height:12}),volume:Math.PI*12**2*(radius-12/3),area:2*Math.PI*radius*12+Math.PI*12*(2*radius-12),collapsed:1,surface:"sphere"},
  {id:"cone",make:()=>authorPointedCone({radius,height}),volume:Math.PI*radius**2*height/3,area:Math.PI*radius*(radius+Math.hypot(radius,height)),collapsed:1,surface:"cone"},
 ]){
  const document=poleSession.build(fixture.make());
  try {
   const part=Object.values(document.parts)[0];
   assert.equal(part.family,"revolution");assert.equal(part.faces().filter(face=>face.surface===fixture.surface).length,1);
   if(fixture.id!=="dome")assert.equal(part.faces("shell")[0].surface,fixture.surface);
   assert.ok(Math.abs(part.measurements.volume.value-fixture.volume)<fixture.volume*1e-12);
   assert.ok(Math.abs(part.measurements.surfaceArea.value-fixture.area)<fixture.area*1e-12);
   for(const tolerance of [.45,.12,.035]){const display=part.realize({tolerance});within(display);closedDisplay(display);}
   const charted=part.realize({tolerance:.12,chart:charts.revolve({referenceRadius:radius})});within(charted);closedDisplay(charted);
   assert.ok(charted.uvs.every(Number.isFinite));
   const file=part.exportStep({timestamp});
   assert.equal(file.report.correspondence.edges.filter(edge=>edge?.kind==="collapsedVertex").length,fixture.collapsed);
   assert.ok(file.text.includes(fixture.surface==="sphere"?"SPHERICAL_SURFACE(":"CONICAL_SURFACE("));
  }finally{document.dispose();}
 }
 const held=poleSession.build(authorDome({radius,height:12}));
 try {
  for(const [height,code] of [[51,"revolve.cap.height"],[1e-20,"revolve.cap.heightResolution"]])assert.throws(()=>poleSession.build(authorDome({radius,height})),error=>error instanceof ExedraError&&error.diagnostic.code===code&&error.diagnostic.radius===radius&&error.diagnostic.height===height&&error.timing.outcome==="refused");
  held.parts.dome.realize(policy);
 }finally{held.dispose();}
 assert.equal(poleSession.stats().documents,0);assert.equal(poleSession.stats().cacheEntries,0);
 results.push({sample:"native-pole-displays",outcome:"success",closedSamples:3,qualities:3,charted:true,refusals:["invalid-height","lost-height-resolution"]});
}finally{poleSession.dispose();}

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
        for(const edge of file.report.correspondence.edges)if(edge!==null){
          assert.ok(edge.kind==="curve"||edge.kind==="collapsedVertex");
          assert.match(file.text,new RegExp(`#${edge.entity}=${edge.kind==="curve"?"EDGE_CURVE":"VERTEX_POINT"}\\(`));
        }
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



// Profile-level shoulder treatments retain exact native surfaces and moments.
const shoulderSession=new AuthoringSession(new AuthoringKernel(),{units:"mm"});
try {
 const cases=[{kind:"fillet",radius:0},{kind:"fillet",radius:.5},{kind:"fillet",radius:2},{kind:"fillet",radius:3.5},{kind:"chamfer",distance:2}];
 for(const shoulder of cases){
  const document=shoulderSession.build(authorFlangedSleeve({...flangeBase,shoulder}));
  try {
   const part=document.parts.sleeve,size=shoulder.kind==="fillet"?shoulder.radius:shoulder.distance;
   const moment=shoulder.kind==="fillet"?flangeBase.sleeveOuterRadius*size**2*(1-Math.PI/4)+size**3*(5/6-Math.PI/4):flangeBase.sleeveOuterRadius*size**2/2+size**3/6;
   assert.ok(Math.abs(part.measurements.volume.value-(flangeVolume(flangeBase)+flangeBase.angle*Math.PI/180*moment))<1e-7);
   const faces=part.faces("shoulder");assert.equal(faces.length,size===0?0:1);
   if(size!==0){assert.equal(faces[0].surface,shoulder.kind==="fillet"?"torus":"cone");const display=part.realize(policy);within(display);assert.ok(display.nativeFaces.includes(faces[0].nativeId));const fine=part.realize({tolerance:.035});within(fine);part.requireSelection(faces[0]);const file=part.exportStep({timestamp});assert.ok(file.text.includes(shoulder.kind==="fillet"?"TOROIDAL_SURFACE(":"CONICAL_SURFACE("));}
  }finally{document.dispose();}
 }
 const good=shoulderSession.build(authorFlangedSleeve({...flangeBase,shoulder:{kind:"fillet",radius:2}}));
 assert.throws(()=>shoulderSession.build(authorFlangedSleeve({...flangeBase,shoulder:{kind:"fillet",radius:5}})),error=>error instanceof ExedraError&&error.diagnostic.code==="profile.corner.overlap"&&error.diagnostic.segmentLength===4&&Number.isFinite(error.diagnostic.requestedTrim));
 good.parts.sleeve.realize(policy);good.dispose();assert.equal(shoulderSession.stats().documents,0);assert.equal(shoulderSession.stats().cacheEntries,0);
 results.push({sample:"profile-shoulder-treatments",outcome:"success",nativeTreatments:cases.length});
}finally{shoulderSession.dispose();}


// Construction charts preserve exact source metadata and final native-face mapping.
const chartSession=new AuthoringSession(new AuthoringKernel(),{units:"mm"});
try {
 const grid=uvTransform({matrix:[[.05,0],[0,.05]]});
 for(const [id,chart] of [["collar",charts.revolve({referenceRadius:15.5,wall:grid,caps:grid})],["rail",charts.sweep({wall:grid,caps:grid})],["loft",charts.loft({restLength:120,wall:grid,caps:grid})]]) {
  const sample=galleryCases.find(sample=>sample.id===id),document=chartSession.build(sample.make());
  try {
   const part=Object.values(document.parts)[0],metadata=part.measurements,plain=part.realize(policy),charted=part.realize({...policy,chart}),repeat=part.realize({...policy,chart});
   within(charted);within(repeat);equalDisplay(charted,repeat);assert.deepEqual(charted.uvs,repeat.uvs);assert.equal(repeat.report.cacheHits,1);
   assert.equal(charted.uvs.length,charted.positions.length/3*2);assert.ok(charted.uvs.every(Number.isFinite));assert.equal(charted.report.chart.kind,chart.kind);
   assert.strictEqual(part.measurements,metadata);part.requireSelection(plain.pickTriangle(0));
   const ids=new Set(part.faces().map(face=>face.nativeId));assert.ok(charted.nativeFaces.every(face=>ids.has(face)));
   const saved=charted.uvs.slice();document.dispose();assert.deepEqual(charted.uvs,saved,"UV buffers did not remain caller-owned");
  } finally {document.dispose();}
 }
 const document=chartSession.build(galleryCases.find(sample=>sample.id==="bore").make());
 try {
  const part=document.parts.plate;assert.equal(part.chartKind,null);assert.equal(part.bore.coordinateSpace,"construction");assert.ok(Math.abs(part.bore.rims[0].semiMajor-16)<1e-10);assert.equal(part.bore.rims[0].semiMinor,8);
  assert.ok(part.exportStep({timestamp}).text.includes("ELLIPSE("));assert.throws(()=>part.realize({...policy,chart:charts.extrude()}),error=>error instanceof ExedraError&&error.diagnostic.code==="chart.unavailable");
 } finally {document.dispose();}
 assert.equal(chartSession.stats().documents,0);assert.equal(chartSession.stats().cacheEntries,0);
 results.push({sample:"construction-charts-and-bore-evidence",outcome:"success",chartedFamilies:3});
} finally {chartSession.dispose();}

// Authoritative centers feed ordinary native operations and typed recovery.
const centerSession=new AuthoringSession(new AuthoringKernel(),{units:"mm"});
try {
 const outline=profiles.loop([
  {kind:"line",to:[3,0]}, {kind:"line",to:[4,0]},
  {kind:"arc",to:[3,1],center:[3,0],clockwise:false,feature:"rim"},
 ]);
 const document=centerSession.build({plate:extrude(outline,{height:2}),turned:revolve(outline,{angle:1.7})});
 try {
  assert.ok(Math.abs(document.parts.plate.measurements.volume.value-Math.PI/2)<1e-12);
  assert.ok(Math.abs(document.parts.turned.measurements.volume.value-1.7*(3*Math.PI/4+1/3))<1e-12);
  for(const part of Object.values(document.parts)){
   const rim=part.faces("rim");assert.equal(rim.length,1);
   assert.equal(rim[0].surface,part.family==="extrusion"?"cylinder":"torus");
   const display=part.realize(policy);within(display);assert.ok(display.nativeFaces.includes(rim[0].nativeId));
   part.exportStep({timestamp});
  }
  const bad=profiles.loop([
   {kind:"line",to:[1e16,0]}, {kind:"line",to:[1e16+2,0]},
   {kind:"arc",to:[1e16,4],center:[1e16,0],clockwise:false,feature:"rim"},
  ]);
  assert.throws(()=>centerSession.build({bad:extrude(bad,{height:2})}),error=>error instanceof ExedraError&&error.diagnostic.code==="profile.arc.endpointRadii"&&error.diagnostic.startRadius===2&&error.diagnostic.endRadius===4&&error.diagnostic.radiusRoundoffBound<1e-12&&error.diagnostic.segment===2);
  document.parts.turned.realize(policy);
 }finally{document.dispose();}
 assert.equal(centerSession.stats().documents,0);assert.equal(centerSession.stats().cacheEntries,0);
 results.push({sample:"center-defined-profile",outcome:"success",nativeFamilies:2,refusedMismatch:"2 vs 4 at datum 1e16"});
}finally{centerSession.dispose();}

console.log(JSON.stringify({samples:results},null,2));
