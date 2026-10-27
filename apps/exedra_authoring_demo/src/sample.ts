// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { extrude, profiles, revolve, placement } from "@exedra/authoring";

export interface PanelParameters {
  width: number;
  height: number;
  thickness: number;
  pocketDepth: number;
  cornerRadius: number;
  ventRadius: number;
}

/** This is the actual authoring function used by the live controls. */
export function authorPanel(p: PanelParameters) {
  const vent = profiles.slot({ length: 22, radius: p.ventRadius }).tagged("vents");
  const outline = profiles.roundedRectangle({
    width: p.width, height: p.height, radius: p.cornerRadius,
  }).tagged("panel").withCutoutsAt(vent, [
    [-36, 22], [-12, 22], [12, 22], [36, 22],
  ]);

  const recess = profiles.slot({ length: 48, radius: 8 }).tagged("recess");
  const panel = extrude(outline, { height: p.thickness })
    .pocket(recess, { end: "end", depth: p.pocketDepth, at: [0, -10] });

  const pin = panel.floor("recess").workplane()
    .translated([0, -10]).offset(0.5)
    .extrude(profiles.circle({ radius: 3 }).tagged("pin"), { height: 7 });

  return { panel, pin };
}

export interface CollarParameters {
  radius: number;
  wall: number;
  collarHeight: number;
  angle: number;
  rounding?: number;
  innerRadius?: number;
}

/** A positive-radius section remains native through full and partial revolutions. */
export function authorCollar(p: CollarParameters) {
  const section = profiles.rectangle({ width: p.wall, height: p.collarHeight })
    .translated([p.radius + p.wall / 2, p.collarHeight / 2]).tagged("collar");
  return {
    collar: revolve(section, {
      angle: p.angle * Math.PI / 180,
      placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
    }),
  };
}

/** Same retained revolution, with circular or rounded sections and optional bore. */
export function authorRoundedTurn(p: CollarParameters, kind: "roundedRing" | "torus" | "hollowTorus") {
  const outline=kind === "roundedRing"
    ? profiles.roundedRectangle({width:p.wall,height:p.collarHeight,radius:p.rounding ?? 2}).translated([p.radius+p.wall/2,p.collarHeight/2]).tagged("ring")
    : (kind === "hollowTorus"
      ? profiles.circle({radius:p.wall}).tagged("outer").withCutoutsAt(profiles.circle({radius:p.innerRadius ?? 4}).tagged("bore"),[[0,0]])
      : profiles.circle({radius:p.wall}).tagged("seal"))
      .translated([p.radius,p.wall]);
  return {ring:revolve(outline,{angle:p.angle*Math.PI/180,placement:placement({rotationXYZ:[Math.PI/2,0,0]})})};
}

export function authorFlangedSleeve(p: CollarParameters) {
  const r=p.radius,w=p.wall,h=p.collarHeight;
  const section=profiles.polygon([[r,0],[r+w,0],[r+w,h*.25],[r+w*.6,h*.25],[r+w*.6,h],[r,h]]).tagged("sleeve");
  return {sleeve:revolve(section,{angle:p.angle*Math.PI/180,placement:placement({rotationXYZ:[Math.PI/2,0,0]})})};
}

/** The same authored section as the Rust turned-fitting study. */
export function authorTurnedFitting(p: CollarParameters) {
  const outline: readonly (readonly [number,number])[]=[
    [22,0],[22,5],[18,5],[18,10],[20,10],[20,13],[18,13],[18,30],[14,38],[14,44],[11,47],[7,47],
    [7,26],[10,19],[10,8],[7,3],[7,0],
  ];
  const section=profiles.polygon(outline.map(([r,z])=>[p.radius+(r-7)*p.wall/15,z*p.collarHeight/47] as const)).tagged("fitting");
  return {fitting:revolve(section,{angle:p.angle*Math.PI/180,placement:placement({rotationXYZ:[Math.PI/2,0,0]})})};
}
