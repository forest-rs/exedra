// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { authorPanel, authorCollar, authorRoundedTurn, authorFlangedSleeve, authorTurnedFitting } from "../src/sample.ts";

// Independent must-succeed inventory. Unsupported exports are specific expected
// outcomes, not an allowance for arbitrary failures or all-export-refused runs.
export const galleryCases = [
  { id: "panel", step: "supported", make: (delta=0) => authorPanel({width:120+delta,height:80,thickness:12,pocketDepth:5,cornerRadius:6,ventRadius:3}) },
  { id: "collar", step: "supported", make: (delta=0) => authorCollar({radius:12+delta,wall:7,collarHeight:30,angle:270}) },
  { id: "roundedRing", step: "unsupported", make: (delta=0) => authorRoundedTurn({radius:22+delta,wall:12,collarHeight:24,angle:360,rounding:2},"roundedRing") },
  { id: "torus", step: "unsupported", make: (delta=0) => authorRoundedTurn({radius:35+delta,wall:7,collarHeight:30,angle:360},"torus") },
  { id: "hollowTorus", step: "unsupported", make: (delta=0) => authorRoundedTurn({radius:35+delta,wall:7,collarHeight:30,angle:270,innerRadius:4},"hollowTorus") },
  { id: "sleeve", step: "supported", make: (delta=0) => authorFlangedSleeve({radius:12+delta,wall:10,collarHeight:35,angle:270}) },
  { id: "fitting", step: "unsupported", make: (delta=0) => authorTurnedFitting({radius:7+delta,wall:15,collarHeight:47,angle:270}) },
];
