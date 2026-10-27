// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { ExedraError, extrude, profiles, revolve, sweep, loft, paths, placement } from "@exedra/authoring";

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

export interface FlangeParameters {
  boreRadius:number;
  sleeveOuterRadius:number;
  flangeOuterRadius:number;
  shoulderHeight:number;
  height:number;
  angle:number;
}
/** Independent radial and axial dimensions; the native profile validates material geometry. */
export function authorFlangedSleeve(p:FlangeParameters) {
  if(!(p.shoulderHeight>0&&p.shoulderHeight<p.height))throw new ExedraError({code:"input.flangeHeight",operation:"sample.flangedSleeve",message:"Shoulder height must be positive and less than total height."});
  const r=p.boreRadius,upper=p.sleeveOuterRadius,bottom=p.flangeOuterRadius,t=p.shoulderHeight,h=p.height;
  const points:readonly (readonly [number,number])[]=upper===bottom?[[r,0],[upper,0],[upper,h],[r,h]]:[[r,0],[bottom,0],[bottom,t],[upper,t],[upper,h],[r,h]];
  const section=profiles.polygon(points).tagged("sleeve");
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

export interface SweepParameters {radius:number;bendRadius:number;angle:number;hollow:boolean;}
/** Smooth exact line/arc rail, using the same path vocabulary as the Rust study. */
export function authorRail(p:SweepParameters) {
  const outer=profiles.circle({radius:p.radius}).tagged("rail");
  const section=p.hollow?outer.withCutoutsAt(profiles.circle({radius:p.radius*.6}).tagged("bore"),[[0,0]]):outer;
  const path=paths.start([0,0,0],{tangent:[0,1,0],sectionX:[1,0,0]})
    .lineBy([0,20,0]).tangentArc({radius:p.bendRadius,angle:p.angle*Math.PI/180,normal:[0,0,1]});
  return {rail:sweep(section,path,{placement:placement({translation:[0,0,p.radius]})})};
}
export interface LoftParameters {height:number;twist:number;offset:number;}
/** Four corresponding rounded sections; wall supports are native ruled surfaces. */
export function authorVariableLoft(p:LoftParameters) {
  const sizes=[[52,38,8],[80,52,12],[60,40,6],[68,44,10]] as const;
  const angles=[0,p.twist,-p.twist*.6,p.twist*.8];
  const offsets=[[0,0],[p.offset,0],[-p.offset*.67,p.offset*.44],[0,0]] as const;
  return {loft:loft(sizes.map(([width,height,radius],i)=>({
    profile:profiles.roundedRectangle({width,height,radius}).tagged(`section ${i+1}`),
    placement:placement({translation:[offsets[i][0],offsets[i][1],p.height*i/3],rotationXYZ:[0,0,angles[i]*Math.PI/180]}),
  })))};
}
export interface FinishParameters {size:number;height:number;treatment:"fillet"|"chamfer";inner:boolean;}
/** Named profile axial edges only: outer rounds or inner chamfers, with native transition tags. */
export function authorFinishedPlate(p:FinishParameters) {
  const outer=profiles.rectangle({width:70,height:50}).tagged("body");
  const outline=p.inner?outer.withCutoutsAt(profiles.rectangle({width:32,height:22}).tagged("bore"),[[0,0]]):outer;
  const original=extrude(outline,{height:p.height});
  const edges=original.axialEdges(p.inner?"bore":"body");
  const finished=p.treatment === "fillet"?edges.fillet({radius:p.size,feature:"round"}):edges.chamfer({distance:p.size,feature:"chamfer"});
  return {plate:finished};
}
