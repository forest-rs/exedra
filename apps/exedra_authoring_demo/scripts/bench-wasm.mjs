// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Actual generated-Wasm repeated-edit workload. Run after npm run build.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { AuthoringSession, charts, uvTransform } from "@exedra/authoring";
import { AuthoringKernel, default as initialize } from "../../../packages/exedra/dist/wasm_pkg/exedra_authoring_web_bridge.js";
import { galleryCases } from "./gallery-cases.mjs";
await initialize({module_or_path:await readFile(new URL("../../../packages/exedra/dist/wasm_pkg/exedra_authoring_web_bridge_bg.wasm",import.meta.url))});
const warmups=4,samples=17,policy={tolerance:0.12};
const constructionCharts=process.argv.includes("--charts");
const grid=uvTransform({matrix:[[.05,0],[0,.05]]});
const chartPolicy={revolve:charts.revolve({referenceRadius:28,wall:grid,caps:grid}),sweep:charts.sweep({wall:grid,caps:grid}),loft:charts.loft({restLength:120,wall:grid,caps:grid})};
function distribution(values){const sorted=values.toSorted((a,b)=>a-b);return {p50:sorted[Math.floor(sorted.length*.5)],p95:sorted[Math.floor(sorted.length*.95)]};}
const results=[];
for(const sample of galleryCases){
  const session=new AuthoringSession(new AuthoringKernel(),{units:"mm"});let previous;
  try{
    const observations=[];
    for(let i=0;i<warmups+samples;i++){
      const started=performance.now(),document=session.build(sample.make(i*.01));
      try{
        const views=Object.values(document.parts).map(part=>part.realize({...policy,chart:constructionCharts&&part.chartKind?chartPolicy[part.chartKind]:undefined}));
        assert.ok(views.every(view=>view.report.withinTolerance));
        previous?.dispose();previous=document;
        if(i>=warmups){
          const timings=[document.buildTiming,...views.map(view=>view.timing)];
          observations.push({totalMs:performance.now()-started,buildKernelMs:document.buildTiming.kernelMs,realizeKernelMs:views.reduce((sum,view)=>sum+view.timing.kernelMs,0),sdkPrepareMs:timings.reduce((sum,t)=>sum+t.sdkPrepareMs,0),transferMs:timings.reduce((sum,t)=>sum+t.transferMs,0),metadataMs:timings.reduce((sum,t)=>sum+t.metadataMs,0),triangles:views.reduce((sum,view)=>sum+view.report.triangles,0),bufferBytes:views.reduce((sum,view)=>sum+view.positions.byteLength+view.normals.byteLength+view.indices.byteLength+view.nativeFaces.byteLength+(view.uvs?.byteLength??0),0),uvVertices:views.reduce((sum,view)=>sum+(view.uvs?.length??0)/2,0),cacheHits:views.reduce((sum,view)=>sum+view.report.cacheHits,0)});
        }
      }catch(error){document.dispose();throw error;}
    }
    const live=session.stats();previous.dispose();previous=undefined;const released=session.stats();
    assert.equal(live.documents,1);assert.equal(released.documents,0);assert.equal(released.cacheEntries,0);
    results.push({sample:sample.id,samples,ms:Object.fromEntries(["totalMs","buildKernelMs","realizeKernelMs","sdkPrepareMs","transferMs","metadataMs"].map(key=>[key,distribution(observations.map(row=>row[key]))])),lastWork:observations.at(-1),live,released});
  }finally{previous?.dispose();session.dispose();}
}
console.log(JSON.stringify({policy,constructionCharts,warmups,environment:{runtime:process.version,platform:process.platform,architecture:process.arch},scope:"host elapsed, generated release Wasm; excludes GPU, view creation and presentation",results},null,2));
