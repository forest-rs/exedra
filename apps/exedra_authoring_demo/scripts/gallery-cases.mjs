// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { authorPanel, authorCollar, authorRoundedTurn, authorFlangedSleeve, authorSphere, authorDome, authorPointedCone, authorTurnedFitting, authorRail, authorVariableLoft, authorFinishedPlate, authorAngledBore, authorHousing, authorFusedHousing, authorValveBlock } from "../src/sample.ts";

// Independent must-succeed inventory. Unsupported exports are specific expected
// outcomes, not an allowance for arbitrary failures or all-export-refused runs.
export const galleryCases = [
  { id: "panel", step: "supported", make: (delta=0) => authorPanel({width:120+delta,height:80,thickness:12,pocketDepth:5,cornerRadius:6,ventRadius:3}) },
  { id: "collar", step: "supported", make: (delta=0) => authorCollar({radius:12+delta,wall:7,collarHeight:30,angle:270}) },
  { id: "roundedRing", step: "supported", make: (delta=0) => authorRoundedTurn({radius:22+delta,wall:12,collarHeight:24,angle:360,rounding:2},"roundedRing") },
  { id: "torus", step: "supported", make: (delta=0) => authorRoundedTurn({radius:35+delta,wall:7,collarHeight:30,angle:360},"torus") },
  { id: "hollowTorus", step: "supported", make: (delta=0) => authorRoundedTurn({radius:35+delta,wall:7,collarHeight:30,angle:270,innerRadius:4},"hollowTorus") },
  { id: "sleeve", step: "supported", make: (delta=0) => authorFlangedSleeve({boreRadius:12+delta,sleeveOuterRadius:18,flangeOuterRadius:22,shoulderHeight:8.75,height:35,angle:270,shoulder:{kind:"fillet",radius:2}}) },
  { id: "fitting", step: "supported", make: (delta=0) => authorTurnedFitting({radius:7+delta,wall:15,collarHeight:47,angle:270}) },
  { id: "sphere", step: "supported", make: (delta=0) => authorSphere({radius:25+delta}) },
  { id: "dome", step: "supported", make: (delta=0) => authorDome({radius:25+delta,height:12}) },
  { id: "cone", step: "supported", make: (delta=0) => authorPointedCone({radius:25+delta,height:40}) },
  { id: "rail", step: "supported", make: (delta=0) => authorRail({radius:6+delta,bendRadius:40,angle:135,hollow:false}) },
  { id: "hollowRail", step: "supported", make: (delta=0) => authorRail({radius:8+delta,bendRadius:40,angle:135,hollow:true}) },
  { id: "loft", step: "supported", measures:"unavailable", make: (delta=0) => authorVariableLoft({height:120+delta,twist:14,offset:18}) },
  { id: "fillet", step: "supported", make: (delta=0) => authorFinishedPlate({size:5+delta,height:30,treatment:"fillet",inner:false}) },
  { id: "chamfer", step: "supported", make: (delta=0) => authorFinishedPlate({size:3+delta,height:30,treatment:"chamfer",inner:true}) },
  { id:"bore", step:"supported", make:(delta=0)=>authorAngledBore({width:100,height:70,thickness:20,radius:8+delta,angle:60,separation:2}) },
  {id:"housing",step:"supported",make:(delta=0)=>authorHousing({width:104+delta,bodyWidth:64,height:90,flangeDepth:24,thickness:18,passageRadius:12,passageY:15,passageAngle:20,mountSpacing:76,mountRadius:5,portY:-10,portRadius:5,clearance:1})},
  {id:"fusedHousing",step:"supported",make:(delta=0)=>authorFusedHousing({width:104+delta,bodyWidth:64,height:90,flangeDepth:24,thickness:18,passageRadius:12,passageY:15,passageAngle:20,mountSpacing:76,mountRadius:5,portY:-10,portRadius:5,clearance:1})},
  {id:"valveBlock",step:"supported",make:(delta=0)=>authorValveBlock({width:112+delta,depth:72,height:42,topAngle:12,portRadius:8,counterboreRadius:14,counterboreDepth:10,blindDepth:14,chamberRadius:7,chamberSpacing:10,chamberHeight:14})},
];
