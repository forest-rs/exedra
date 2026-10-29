// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/** Model lengths are expressed in the document's explicitly selected unit. */
export type LengthUnit = "mm" | "m";
export type XY = readonly [number, number];
export type XYZ = readonly [number, number, number];
export type NativeFamily = "extrusion" | "revolution" | "sweep" | "loft" | "boredPlate" | "body";
/** Rigid placement intent: fixed-axis X, Y, Z rotations in radians, then translation. */
export interface Placement {
  readonly translation: XYZ;
  readonly rotationXYZ: XYZ;
}
function point3(value: XYZ): XYZ { return Object.freeze(value.map((v,i)=>finite(v,`coordinate ${i}`))) as unknown as XYZ; }
/** Geometry and validation of the frame stay in Rust; this snapshots placement intent. */
export function placement(options: { translation?: XYZ; rotationXYZ?: XYZ } = {}): Placement {
  return Object.freeze({ translation: point3(options.translation ?? [0,0,0]), rotationXYZ: point3(options.rotationXYZ ?? [0,0,0]) });
}
/** Row-major mapping from construction lengths to dimensionless texture repeats. */
export interface UvTransform {readonly matrix:readonly [XY,XY];readonly offset:XY;}
/** Copies a texture-space transform; geometric chart validation remains in Rust. */
export function uvTransform(options:{matrix?:readonly [XY,XY];offset?:XY}={}):UvTransform {
  const matrix=options.matrix??[[1,0],[0,1]];
  return Object.freeze({matrix:Object.freeze([point(matrix[0]),point(matrix[1])]) as readonly [XY,XY],offset:point(options.offset??[0,0])});
}
interface ChartMaps {readonly wall:UvTransform;readonly caps:UvTransform;}
export type ExtrudeChart=ChartMaps & {readonly kind:"extrude"};
export type RevolveChart=ChartMaps & {readonly kind:"revolve";readonly referenceRadius:number};
export type SweepChart=ChartMaps & {readonly kind:"sweep"};
export type LoftChart=ChartMaps & {readonly kind:"loft";readonly referenceSection:number;readonly restLength:number;readonly stations:"sectionIndex"|"datumDistance"};
export type ConstructionChart=ExtrudeChart|RevolveChart|SweepChart|LoftChart;
function chartMaps(options:{wall?:UvTransform;caps?:UvTransform}):ChartMaps {return {wall:uvTransform(options.wall),caps:uvTransform(options.caps)};}
/** Immutable display policy. It changes construction UVs, never native source identity. */
export const charts=Object.freeze({
  /** Availability is reported per installed source; retained extrusions may refuse this chart. */
  extrude(options:{wall?:UvTransform;caps?:UvTransform}={}):ExtrudeChart {return Object.freeze({kind:"extrude",...chartMaps(options)});},
  /** U uses referenceRadius × angle; other radii stretch this rest chart. */
  revolve(options:{referenceRadius:number;wall?:UvTransform;caps?:UvTransform}):RevolveChart {return Object.freeze({kind:"revolve",referenceRadius:finite(options.referenceRadius,"reference radius"),...chartMaps(options)});},
  /** Exact source profile distance and centerline distance; bends stretch inner/outer rails. */
  sweep(options:{wall?:UvTransform;caps?:UvTransform}={}):SweepChart {return Object.freeze({kind:"sweep",...chartMaps(options)});},
  /** Exact reference-profile distance and explicit rest length, divided by the station rule. */
  loft(options:{restLength:number;referenceSection?:number;stations?:"sectionIndex"|"datumDistance";wall?:UvTransform;caps?:UvTransform}):LoftChart {
    const section=options.referenceSection??0;
    if(!Number.isInteger(section)||section<0||section>0xffff_ffff)fail("input.referenceSection","Reference section must be an unsigned 32-bit integer");
    return Object.freeze({kind:"loft",referenceSection:section,restLength:finite(options.restLength,"rest length"),stations:options.stations??"datumDistance",...chartMaps(options)});
  },
});
function snapshotChart(chart:ConstructionChart):ConstructionChart {
  switch(chart.kind){case "extrude":return charts.extrude(chart);case "revolve":return charts.revolve(chart);case "sweep":return charts.sweep(chart);case "loft":return charts.loft(chart);default:fail("input.chart","Unknown construction chart kind");}
}
export type End = "start" | "end";

/** Position and local segment identity within the refused authored profile. */
export interface ProfileCornerLocation {readonly boundary:"outer"|"hole";readonly hole?:number;readonly segment:number;readonly point:XY;}
export interface Diagnostic {
  code: string;
  operation: string;
  message: string;
  node?: number;
  profile?: number;
  segment?:number;
  startRadius?:number;
  endRadius?:number;
  radiusRoundoffBound?:number;
  radius?:number;
  height?:number;
  axisOffset?:number;
  rimRadius?:number;
  cornerPoint?:XY;
  cornerTolerance?:number;
  corner?:ProfileCornerLocation;
  cornerCandidates?:readonly ProfileCornerLocation[];
  requestedSize?:number;
  segmentLength?:number;
  requestedTrim?:number;
  minimumSeparation?:number;
  feature?: string;
  family?: NativeFamily;
  measure?: "volume" | "surfaceArea";
  face?: number;
  geometryFamily?: string;
  semiAngle?: number;
  semiAngleAbsRange?: readonly [number,number];
  clearance?:number;
  requiredClearance?:number;
  axisDot?:number;
  minimumAxisDot?:number;
  end?:End;
  operand?:"left"|"right";
  vertex?:number;
  axis?:number;
  cell?:readonly [number,number,number];
  required?:number;
  maximum?:number;
}
/** Host-observed elapsed milliseconds at the SDK/Wasm boundary; never GPU or presentation time. */
export interface KernelCallTiming {
  readonly operation: "document.build" | "part.realize" | "part.workplane" | "part.exportStep";
  readonly outcome: "success" | "refused";
  readonly sdkPrepareMs: number;
  readonly kernelMs: number;
  readonly transferMs: number;
  readonly metadataMs: number;
  readonly cleanupMs: number;
  readonly totalMs: number;
  readonly documentId?: string;
  readonly partId?: string;
  readonly diagnosticCode?: string;
}
type TimingPhase = "sdkPrepareMs" | "kernelMs" | "transferMs" | "metadataMs" | "cleanupMs";
class CallTiming {
  private readonly start=performance.now();
  private readonly phases={sdkPrepareMs:0,kernelMs:0,transferMs:0,metadataMs:0,cleanupMs:0};
  private completed: KernelCallTiming | null=null;
  private documentId?: string;
  private readonly partId?: string;
  private readonly operation:KernelCallTiming["operation"];
  private record:((timing:KernelCallTiming)=>void) | null;
  constructor(operation:KernelCallTiming["operation"],record:(timing:KernelCallTiming)=>void,scope?:{documentId:string;partId?:string}) {this.operation=operation;this.record=record;this.documentId=scope?.documentId;this.partId=scope?.partId;}
  phase<T>(phase:TimingPhase,action:()=>T):T {const start=performance.now();try{return action()}finally{this.phases[phase]+=Math.max(0,performance.now()-start)}}
  scope(documentId:string):void {this.documentId=documentId;}
  finish(outcome:KernelCallTiming["outcome"],diagnosticCode?:string):KernelCallTiming {
    if(!this.completed){this.completed=Object.freeze({operation:this.operation,outcome,...this.phases,totalMs:Math.max(0,performance.now()-this.start),documentId:this.documentId,partId:this.partId,diagnosticCode});const record=this.record;this.record=null;record?.(this.completed);}
    return this.completed;
  }
  refusal(error:unknown):ExedraError {const original=this.phase("metadataMs",()=>asExedraError(error));return new ExedraError(original.diagnostic,this.finish("refused",original.diagnostic.code));}
  get value():KernelCallTiming {if(!this.completed)throw new Error("SDK call timing is read before the operation completed");return this.completed;}
}
/** A structured authoring, native geometry, lifecycle or realization refusal. */
export class ExedraError extends Error {
  readonly diagnostic: Readonly<Diagnostic>;
  readonly timing?: KernelCallTiming;
  constructor(diagnostic: Diagnostic, timing?: KernelCallTiming) {
    super(diagnostic.message);
    this.name = "ExedraError";
    const snapshot={...diagnostic};
    const freezeCorner=(corner:ProfileCornerLocation):ProfileCornerLocation=>Object.freeze({...corner,point:Object.freeze([...corner.point]) as XY});
    if(snapshot.cornerPoint)snapshot.cornerPoint=Object.freeze([...snapshot.cornerPoint]) as XY;
    if(snapshot.corner)snapshot.corner=freezeCorner(snapshot.corner);
    if(snapshot.cornerCandidates)snapshot.cornerCandidates=Object.freeze(snapshot.cornerCandidates.map(freezeCorner));
    this.diagnostic = Object.freeze(snapshot);
    this.timing=timing && Object.freeze({...timing});
  }
}
function fail(code: string, message: string): never {
  throw new ExedraError({ code, operation: "authoring", message });
}
function finite(value: number, name: string): number {
  if (!Number.isFinite(value)) fail("input.nonFinite", `${name} must be finite`);
  return value;
}
function name(value: string): string {
  if (!value.trim() || new TextEncoder().encode(value).length > 128)
    fail("input.name", "Names must contain text and fit in 128 UTF-8 bytes");
  return value;
}
function point(value: XY): XY {
  return Object.freeze([finite(value[0], "x"), finite(value[1], "y")]);
}

/** One closed-outline segment; its start is the preceding endpoint, cyclically. */
export type ProfileSegment =
  | {readonly kind:"line";readonly to:XY;readonly feature?:string}
  | {readonly kind:"arc";readonly to:XY;readonly center:XY;readonly clockwise:boolean;readonly feature?:string};

type ProfileNode =
  | { kind:"corners";profile:Profile;at:readonly XY[];tolerance?:number;treatment:"fillet"|"chamfer";size:number;feature?:string }
  | { kind: "polygon"; points: readonly XY[] }
  | { kind:"loop";segments:readonly ProfileSegment[] }
  | { kind: "rectangle"; width: number; height: number }
  | { kind: "roundedRectangle"; width: number; height: number; radius: number }
  | { kind: "circle"; radius: number }
  | { kind: "slot"; length: number; radius: number }
  | { kind: "tagged"; profile: Profile; name: string }
  | { kind: "translated"; profile: Profile; offset: XY }
  | { kind: "cutouts"; profile: Profile; template: Profile; positions: readonly XY[] };
const profileNodes = new WeakMap<Profile, ProfileNode>();

/** Immutable positions to identify on one authored profile, not installed topology handles. */
export interface AuthoredProfileCorners {
  /** Exact tangent circular transitions between adjacent straight segments. */
  readonly fillet:(radius:number,options?:{feature?:string})=>Profile;
  /** Equal-distance trims joined by straight transitions. */
  readonly chamfer:(distance:number,options?:{feature?:string})=>Profile;
}

/** An immutable reusable profile description. Geometry is checked by Rust at build time. */
export class Profile {
  private constructor(node: ProfileNode) {
    profileNodes.set(this, Object.freeze(node));
    Object.freeze(this);
  }
  /** Closed line outline; vertices are authored once, with winding checked by Rust. */
  static polygon(vertices: Iterable<XY>): Profile {
    const points: XY[]=[];
    for (const vertex of vertices) {
      if (points.length===4096) fail("budget.profileSegments","At most 4096 profile vertices are allowed");
      points.push(point(vertex));
    }
    return new Profile({kind:"polygon",points:Object.freeze(points)});
  }
  /** Closed line/center-defined-arc outline. Rust validates winding, radii and separation.
   * Each start is the preceding endpoint, cyclically; full circles need two arcs.
   * Coordinates and optional per-segment feature labels are copied immediately.
   */
  static loop(input:Iterable<ProfileSegment>):Profile {
    const segments:ProfileSegment[]=[];
    for(const segment of input){
      if(segments.length===4096)fail("budget.profileSegments","At most 4096 profile segments are allowed");
      const common={to:point(segment.to),feature:segment.feature===undefined?undefined:name(segment.feature)};
      switch(segment.kind){
        case "line":segments.push(Object.freeze({kind:"line",...common}));break;
        case "arc":
          if(typeof segment.clockwise!=="boolean")fail("input.arcDirection","Arc clockwise must be a boolean");
          segments.push(Object.freeze({kind:"arc",...common,center:point(segment.center),clockwise:segment.clockwise}));break;
        default:fail("input.profileSegment","Expected a line or center-defined arc");
      }
    }
    return new Profile({kind:"loop",segments:Object.freeze(segments)});
  }
  static rectangle(options: { width: number; height: number }): Profile {
    return new Profile({ kind: "rectangle", width: finite(options.width, "width"), height: finite(options.height, "height") });
  }
  static roundedRectangle(options: { width: number; height: number; radius: number }): Profile {
    return new Profile({ kind: "roundedRectangle", width: finite(options.width, "width"), height: finite(options.height, "height"), radius: finite(options.radius, "radius") });
  }
  static circle(options: { radius: number }): Profile {
    return new Profile({ kind: "circle", radius: finite(options.radius, "radius") });
  }
  static slot(options: { length: number; radius: number }): Profile {
    return new Profile({ kind: "slot", length: finite(options.length, "length"), radius: finite(options.radius, "radius") });
  }
  /** Identifies one corner by profile-space position. Defaults to session separation; no snapping. */
  cornerAt(at:XY,options:{tolerance?:number}={}):AuthoredProfileCorners {
    return this.cornersAt([at],options);
  }
  /** Selects unique corners by profile-space positions; geometry/ambiguity are checked in Rust. */
  cornersAt(points:Iterable<XY>,options:{tolerance?:number}={}):AuthoredProfileCorners {
    const at:XY[]=[];
    for(const value of points){if(at.length===4096)fail("budget.corners","At most 4096 corner selections are allowed");at.push(point(value));}
    if(at.length===0)fail("input.corners","Select at least one profile corner");
    const positions=Object.freeze(at),tolerance=options.tolerance===undefined?undefined:finite(options.tolerance,"corner search distance");
    const apply=(treatment:"fillet"|"chamfer",size:number,feature?:string)=>new Profile({kind:"corners",profile:this,at:positions,tolerance,treatment,size:finite(size,"corner size"),feature:feature===undefined?undefined:name(feature)});
    return Object.freeze({fillet:(radius:number,options:{feature?:string}={})=>apply("fillet",radius,options.feature),chamfer:(distance:number,options:{feature?:string}={})=>apply("chamfer",distance,options.feature)});
  }
  /** Labels the whole feature, including every generated segment. */
  tagged(label: string): Profile {
    return new Profile({ kind: "tagged", profile: this, name: name(label) });
  }
  translated(offset: XY): Profile {
    return new Profile({ kind: "translated", profile: this, offset: point(offset) });
  }
  /** Reuses one filled template at every offset; Rust owns hole winding and validation. */
  withCutoutsAt(template: Profile, offsets: Iterable<XY>): Profile {
    const positions: XY[] = [];
    for (const offset of offsets) {
      if (positions.length === 1024) fail("budget.cutouts", "At most 1024 cutout placements are allowed");
      positions.push(point(offset));
    }
    return new Profile({ kind: "cutouts", profile: this, template, positions: Object.freeze(positions) });
  }
}
export const profiles = Object.freeze({
  polygon: Profile.polygon,
  loop: Profile.loop,
  rectangle: Profile.rectangle,
  roundedRectangle: Profile.roundedRectangle,
  circle: Profile.circle,
  slot: Profile.slot,
});

type PathStep =
  | { readonly kind:"lineTo"; readonly to:XYZ }
  | { readonly kind:"lineBy"; readonly offset:XYZ }
  | { readonly kind:"tangentArc"; readonly radius:number; readonly angle:number; readonly normal:XYZ };
interface PathDescription { readonly start:XYZ; readonly tangent:XYZ; readonly sectionX:XYZ; readonly steps:readonly PathStep[] }
const pathNodes=new WeakMap<CurvePath,PathDescription>();
/** Immutable analytic line/arc path; Rust validates tangency and the section frame. */
export class CurvePath {
  private constructor(description:PathDescription) {pathNodes.set(this,Object.freeze(description));Object.freeze(this);}
  static start(at:XYZ,options:{tangent:XYZ;sectionX:XYZ}):CurvePath {
    return new CurvePath({start:point3(at),tangent:point3(options.tangent),sectionX:point3(options.sectionX),steps:Object.freeze([])});
  }
  private append(step:PathStep):CurvePath {
    const source=pathNodes.get(this)!;
    if(source.steps.length===128)fail("budget.path","A sweep path supports at most 128 steps");
    return new CurvePath({...source,steps:Object.freeze([...source.steps,Object.freeze(step)])});
  }
  lineTo(to:XYZ):CurvePath {return this.append({kind:"lineTo",to:point3(to)});}
  lineBy(offset:XYZ):CurvePath {return this.append({kind:"lineBy",offset:point3(offset)});}
  /** Signed turn in radians around the supplied world-space normal. */
  tangentArc(options:{radius:number;angle:number;normal:XYZ}):CurvePath {return this.append({kind:"tangentArc",radius:finite(options.radius,"radius"),angle:finite(options.angle,"angle"),normal:point3(options.normal)});}
}
export const paths=Object.freeze({start:CurvePath.start});
/** Corresponding section intent; the constructor snapshots the placement. */
export interface LoftSection {readonly profile:Profile;readonly placement:Placement;}

/** Exact row-major rigid axes and origin; geometry validation stays in Rust. */
export type NativeFrame = readonly [readonly [number,number,number,number],readonly [number,number,number,number],readonly [number,number,number,number]];
/** Exact unbounded curve support; edge ranges select bounded portions. */
export type NativeCurveDescription =
  | {readonly kind:"line";readonly origin:XYZ;readonly direction:XYZ}
  | {readonly kind:"circle";readonly frame:NativeFrame;readonly radius:number}
  | {readonly kind:"ellipse";readonly frame:NativeFrame;readonly semiMajor:number;readonly semiMinor:number};
/** Bounded traversal used by an exact ruled support. */
export interface NativeCurveSpanDescription {readonly curve:NativeCurveDescription;readonly range:XY;readonly reversed?:boolean;}
/** Exact supporting geometry. Face loops retain the actual bounded domain. */
export type NativeSurfaceDescription =
  | {readonly kind:"plane";readonly frame:NativeFrame}
  | {readonly kind:"cylinder"|"sphere";readonly frame:NativeFrame;readonly radius:number}
  | {readonly kind:"cone";readonly frame:NativeFrame;readonly radius:number;readonly slope:number}
  | {readonly kind:"torus";readonly frame:NativeFrame;readonly majorRadius:number;readonly minorRadius:number}
  | {readonly kind:"ruled";readonly curves:readonly [NativeCurveSpanDescription,NativeCurveSpanDescription]};
/** A shared edge or an explicit collapsed surface-pole boundary. IDs index tables. */
export type NativeEdgeDescription =
  | {readonly kind:"curve";readonly curve:NativeCurveDescription;readonly range:XY;readonly start:number;readonly end:number}
  | {readonly kind:"collapsed";readonly vertex:number};
/** One oriented occurrence of a shared native edge. */
export interface NativeCoedgeDescription {readonly edge:number;readonly reversed?:boolean;}
/** Trimmed supporting surface, with outer loop first and subsequent hole loops. */
export interface NativeFaceDescription {
  readonly surface:NativeSurfaceDescription;
  readonly u:XY;
  readonly v:XY;
  readonly sense:"forward"|"reverse";
  readonly loops:readonly number[];
  readonly features?:readonly string[];
}
/** Face parameters following a shared edge's canonical start-to-end direction.
 * Coedge reversal changes traversal, not this parameter correspondence.
 * Values are origin + linear*t + cosine*cos(t) + sine*sin(t).
 */
export type NativeTrimDescription =
  | {readonly kind:"line";readonly start:XY;readonly end:XY}
  | {readonly kind:"harmonic";readonly origin:XY;readonly linear:XY;readonly cosine:XY;readonly sine:XY;readonly range:XY};
/** Sparse per-loop face parameters, in coedge occurrence order. */
export interface NativeLoopTrimsDescription {readonly loop:number;readonly curves:readonly NativeTrimDescription[];}
/** Construction-independent authoritative f64 boundary tables.
 * Rust checks references, endpoint identity closure and exact support agreement.
 * Validation does not establish global self-intersection or shell containment.
 */
export interface NativeBodyDescription {
  readonly trims?:readonly NativeLoopTrimsDescription[];
  readonly vertices:readonly XYZ[];
  readonly edges:readonly NativeEdgeDescription[];
  readonly loops:readonly (readonly NativeCoedgeDescription[])[];
  readonly faces:readonly NativeFaceDescription[];
}
function snapshotBody(body:NativeBodyDescription):NativeBodyDescription {
  let work=body.vertices.length+body.edges.length+body.faces.length+body.loops.length+(body.trims?.length??0);
  for(const face of body.faces){work+=face.loops.length+(face.features?.length??0);for(const id of face.loops)work+=body.loops[id]?.length??0;}
  for(const ring of body.loops)work+=ring.length;
  for(const trims of body.trims??[])work+=trims.curves.length;
  if(work>8192)fail("budget.nativeSegments","Boundary tables exceed the 8192 native work budget");
  const freeze=(value:unknown):unknown=>{if(value&&typeof value==="object"){for(const child of Object.values(value))freeze(child);Object.freeze(value);}return value;};
  try {
    const copy=JSON.parse(JSON.stringify(body,(_key,value:unknown)=>{if(typeof value==="number")finite(value,"native body value");return value;})) as NativeBodyDescription;
    for(const face of copy.faces)for(const feature of face.features??[])name(feature);
    return freeze(copy) as NativeBodyDescription;
  } catch(error) {if(error instanceof ExedraError)throw error;fail("input.body","Expected finite acyclic native boundary tables");}
}

type FaceSelector = { kind: "floor"; feature: string } | { kind: "cap"; end: End };
type SolidNode =
  | {kind:"union"|"intersect"|"subtract";base:Solid;other:Solid}
  | {kind:"intersectHalfSpace";base:Solid;point:XYZ;normal:XYZ;feature?:string}
  | {kind:"body";body:NativeBodyDescription;placement:Placement}
  | {kind:"sphericalCap";radius:number;height:number;placement:Placement}
  | {kind:"throughBore"|"subtractThroughCylinder";base:Solid;point:XYZ;direction:XYZ;radius:number;separation:number;feature?:string}
  | { kind:"sweep"; profile:Profile; path:CurvePath; placement:Placement }
  | { kind:"loft"; sections:readonly LoftSection[]; placement:Placement }
  | { kind:"axialFinish"; base:Solid; feature:string; treatment:"fillet"|"chamfer"; size:number; transition:string }
  | { kind: "extrude"; profile: Profile; height: number; placement: Placement }
  | { kind: "revolve"; profile: Profile; angle: number; placement: Placement }
  | { kind: "pocket"; base: Solid; profile: Profile; end: End; depth: number; at: XY }
  | { kind: "onFace"; host: Solid; face: FaceSelector; profile: Profile; height: number; offset: number; at: XY };
const solidNodes = new WeakMap<Solid, SolidNode>();

/** An unoriented cylinder axis in the body's local construction space, before placement. */
export interface ThroughBoreOptions {
  readonly point:XYZ;
  readonly direction:XYZ;
  readonly radius:number;
  /** Minimum remaining material from the tool to untouched boundaries, in document units. Distinct from numeric separation/display tolerance. */
  readonly separation:number;
  readonly feature?:string;
}
/** A body-local clipping plane retaining dot(position - point, normal) <= 0. */
export interface HalfSpaceOptions {
  readonly point:XYZ;
  readonly normal:XYZ;
  /** Feature attached to the new planar cap. */
  readonly feature?:string;
}
/** Immutable native construction intent, distinct from an installed/queryable part. */
export class Solid {
  private constructor(node: SolidNode) {
    solidNodes.set(this, Object.freeze(node));
    Object.freeze(this);
  }
  /** Installs exact boundary tables without reconstructing a constructor family.
   * Inputs are copied immediately. Geometry is validated at document build time.
   * Generic sources expose native selections, bounds and STEP support; construction
   * charts and exact integral measurements are explicitly unavailable.
   */
  static body(body:NativeBodyDescription,options:{placement?:Placement}={}):Solid {
    return new Solid({kind:"body",body:snapshotBody(body),placement:placement(options.placement)});
  }
  static extrude(profile: Profile, options: { height: number; placement?: Placement }): Solid {
    return new Solid({ kind: "extrude", profile, height: finite(options.height, "height"), placement: placement(options.placement) });
  }
  /** Revolves profile X as positive radius and profile Y as height, about local +Y. */
  static revolve(profile: Profile, options: { angle?: number; placement?: Placement } = {}): Solid {
    return new Solid({ kind: "revolve", profile, angle: finite(options.angle ?? 2*Math.PI,"angle"), placement: placement(options.placement) });
  }
  /** Exact full-turn spherical cap, from a planar base at local Y=0 toward +Y.
   * Height may reach twice the radius for a full sphere. Rust builds the canonical
   * profile revolution and refuses dimensions that lose their rim or height in f64.
   */
  static sphericalCap(options:{radius:number;height:number;placement?:Placement}):Solid {
    return new Solid({kind:"sphericalCap",radius:finite(options.radius,"sphere radius"),height:finite(options.height,"cap height"),placement:placement(options.placement)});
  }
  /** Smooth open line/arc sweep; the native constructor checks supported section geometry. */
  static sweep(profile:Profile,path:CurvePath,options:{placement?:Placement}={}):Solid {
    return new Solid({kind:"sweep",profile,path,placement:placement(options.placement)});
  }
  /** Exact ruled loft through corresponding section segments; no automatic unwrapping/rematching. */
  static loft(sections:Iterable<LoftSection>,options:{placement?:Placement}={}):Solid {
    const values:LoftSection[]=[];
    for(const section of sections){if(values.length===64)fail("budget.sections","A loft supports at most 64 sections");values.push(Object.freeze({profile:section.profile,placement:placement(section.placement)}));}
    return new Solid({kind:"loft",sections:Object.freeze(values),placement:placement(options.placement)});
  }
  /** Selects profile axial edges by authored feature; rims and pocket edges are outside this operation. */
  axialEdges(feature:string):AuthoredAxialEdges {return new AuthoredAxialEdges(this,name(feature));}
  /** @internal Lowered by the named axial-edge selection. */
  static axialFinish(base:Solid,feature:string,treatment:"fillet"|"chamfer",size:number,transition:string):Solid {
    return new Solid({kind:"axialFinish",base,feature:name(feature),treatment,size:finite(size,"finish size"),transition:name(transition)});
  }
  /** Retained rectangular-plate bore constructor, with exact conic rim evidence. */
  throughBore(options:ThroughBoreOptions):Solid {
    return new Solid({kind:"throughBore",base:this,point:point3(options.point),direction:point3(options.direction),radius:finite(options.radius,"bore radius"),separation:finite(options.separation,"bore clearance"),feature:options.feature===undefined?undefined:name(options.feature)});
  }
  /** Exact cylinder subtraction through two parallel planar exits.
   * Repeated disjoint cuts preserve existing native IDs. Polygonal and full conic
   * exit trims are supported; conservative separation checks can refuse valid cuts.
   * Input must already be an embedded outward-oriented regular solid; structural
   * validation does not certify global self-intersection or shell containment.
   * Point/direction are in body-local construction space, before placement.
   */
  subtractThroughCylinder(options:ThroughBoreOptions):Solid {
    return new Solid({kind:"subtractThroughCylinder",base:this,point:point3(options.point),direction:point3(options.direction),radius:finite(options.radius,"bore radius"),separation:finite(options.separation,"bore clearance"),feature:options.feature===undefined?undefined:name(options.feature)});
  }
  /** Exact union of orthogonal planar, axis-aligned line bodies in the left local frame.
   * Orientations must match exactly; translations are converted once. Curves and
   * near-axis rotations are refused. Empty results cannot become document parts.
   */
  union(other:Solid):Solid {return new Solid({kind:"union",base:this,other});}
  /** Exact rectilinear intersection; uses the same support/frame limits as union. */
  intersect(other:Solid):Solid {return new Solid({kind:"intersect",base:this,other});}
  /** Exact rectilinear difference; uses the same support/frame limits as union. */
  subtract(other:Solid):Solid {return new Solid({kind:"subtract",base:this,other});}
  /** Intersects a strictly convex planar line body with a body-local halfspace.
   * Curves, holes, nonconvex bodies and planes near existing vertices are refused.
   * A new cap receives feature. Exact integrals are unavailable for this output.
   */
  intersectHalfSpace(options:HalfSpaceOptions):Solid {return new Solid({kind:"intersectHalfSpace",base:this,point:point3(options.point),normal:point3(options.normal),feature:options.feature===undefined?undefined:name(options.feature)});}
  pocket(profile: Profile, options: { end: End; depth: number; at?: XY }): Solid {
    return new Solid({ kind: "pocket", base: this, profile, end: options.end, depth: finite(options.depth, "depth"), at: point(options.at ?? [0, 0]) });
  }
  /** An authored floor selector; missing/ambiguous features are diagnosed at build time. */
  floor(feature: string): AuthoredFace {
    return new AuthoredFace(this, { kind: "floor", feature: name(feature) });
  }
  cap(end: End): AuthoredFace {
    return new AuthoredFace(this, { kind: "cap", end });
  }
  /** @internal Lowered only by the authoring workplane. */
  static onFace(host: Solid, face: FaceSelector, profile: Profile, height: number, offset: number, at: XY): Solid {
    return new Solid({ kind: "onFace", host, face: Object.freeze({ ...face }), profile, height: finite(height, "height"), offset: finite(offset, "offset"), at: point(at) });
  }
}
export const nativeBody = Solid.body;
export const extrude = Solid.extrude;
export const revolve = Solid.revolve;
export const sphericalCap = Solid.sphericalCap;
export const sweep = Solid.sweep;
export const loft = Solid.loft;
/** Authored selection resolved against the native extrusion during build. */
export class AuthoredAxialEdges {
  private readonly host:Solid;
  private readonly feature:string;
  constructor(host:Solid,feature:string){this.host=host;this.feature=name(feature);Object.freeze(this);}
  fillet(options:{radius:number;feature:string}):Solid{return Solid.axialFinish(this.host,this.feature,"fillet",options.radius,options.feature);}
  chamfer(options:{distance:number;feature:string}):Solid{return Solid.axialFinish(this.host,this.feature,"chamfer",options.distance,options.feature);}
}

/** A selector in an authored graph, resolved against native geometry during build. */
export class AuthoredFace {
  private readonly host: Solid;
  private readonly selector: FaceSelector;
  constructor(host: Solid, selector: FaceSelector) {
    this.host = host;
    this.selector = Object.freeze({ ...selector });
    Object.freeze(this);
  }
  workplane(): AuthoringWorkplane {
    return new AuthoringWorkplane(this.host, this.selector);
  }
}
/** Native workplane construction intent. It is not an installed face/query value. */
export class AuthoringWorkplane {
  private readonly host: Solid;
  private readonly selector: FaceSelector;
  private readonly distance: number;
  private readonly at: XY;
  constructor(host: Solid, selector: FaceSelector, distance = 0, at: XY = [0, 0]) {
    this.host = host; this.selector = Object.freeze({ ...selector }); this.distance = finite(distance, "offset"); this.at = point(at);
    Object.freeze(this);
  }
  offset(distance: number): AuthoringWorkplane {
    return new AuthoringWorkplane(this.host, this.selector, finite(this.distance + distance, "offset"), this.at);
  }
  translated(at: XY): AuthoringWorkplane {
    return new AuthoringWorkplane(this.host, this.selector, this.distance, point([this.at[0] + at[0], this.at[1] + at[1]]));
  }
  extrude(profile: Profile, options: { height: number }): Solid {
    return Solid.onFace(this.host, this.selector, profile, finite(options.height, "height"), this.distance, this.at);
  }
}

/** Native source geometry measurements, independent of display tessellation. */
export interface NativeMeasurements {
  volume: NativeMeasurement;
  surfaceArea: NativeMeasurement;
  bounds: readonly [readonly [number, number, number], readonly [number, number, number]];
  boundsKind: "conservativeNativeSupport";
  faces: number;
  edges: number;
  vertices: number;
}
/** A native integral is never replaced with a tessellated estimate or a fabricated zero. */
export type NativeMeasurement =
  | { readonly status: "available"; readonly value: number; readonly basis?:"constructorAndAnalyticMachining"|"nativeBoolean"|"nativeBooleanAndAnalyticMachining" }
  | { readonly status: "unavailable" | "numericLimit"; readonly diagnostic: Readonly<Diagnostic> };
export interface NativeFace {
  id: number;
  features: readonly string[];
  feature?: string;
  end?:End;
  role: "cap" | "wall" | "floor" | "boundary";
  surface: "plane" | "cylinder" | "cone" | "torus" | "sphere" | "ruled";
}
/** Completed work that generated this output; retained on cache hits. Current-call counters are separate. */
export interface RealizationWork {
  readonly attempts:number;
  readonly completedBoundarySamples:number;
  readonly completedTriangles:number;
  readonly trimRefinementBatches:number;
  readonly trimTriangleTests:number;
  readonly trimGeneratedVertices:number;
  /** Analytic normal values attached to explicit-chart triangle corners, across completed attempts. */
  readonly trimNormalCorners:number;
  /** Peak temporary normal-table capacity bytes in a completed attempt; excludes allocator overhead and retained attributes. */
  readonly trimNormalScratchBytes:number;
}
/** A bounded optional retry. Improved does not imply that the final tolerance was met. */
export interface CurveRefinement {
  readonly initialDeviation:number;
  readonly curveTolerance:number;
  readonly outcome:"improved"|"notImproved"|"refused";
  readonly diagnostic?:Readonly<Diagnostic>;
}
export interface RealizationReport {
  hasUvs:boolean;
  chart:ConstructionChart|null;
  work:RealizationWork;
  curveRefinement:CurveRefinement|null;
  tolerance: number;
  maxDeviation: number;
  maxCurveDeviation: number;
  maxSurfaceDeviation: number;
  maxVertexDisplacement: number;
  withinTolerance: boolean;
  triangles: number;
  vertices: number;
  cacheHits: number;
  nativeRealizations: number;
}
/** Structural geometry support; export still validates tolerance, topology and budgets. */
export type StepSupport =
  | { readonly status: "supported" }
  | { readonly status: "unsupported"; readonly diagnostic: Readonly<Diagnostic> };
/** A native edge's exact STEP representation; collapsed boundaries have no 3D curve. */
export type StepEdgeEntity =
  | {readonly kind:"curve"; readonly entity:number}
  | {readonly kind:"collapsedVertex"; readonly entity:number};
export interface StepReport {
  schema: string;
  units: LengthUnit;
  declaredUncertainty: number;
  maxDeviation: number;
  maxNativeSupportGap: number;
  frameRoundoffBound: number;
  /** Length allowance for converting native support parameters to STEP. */
  parameterRoundoffBound: number;
  entities: number;
  faces: number;
  edges: number;
  /** Entries use native local IDs as indices; unused topology entries are null. */
  correspondence: {
    vertices: readonly (number | null)[];
    edges: readonly (StepEdgeEntity | null)[];
    loops: readonly (number | null)[];
    faces: readonly (number | null)[];
  };
}
/** Native conic rims and axis evidence. Axis inputs are construction-space; document values include placement. */
export interface NativeBore {
  readonly coordinateSpace:"construction";
  readonly point:XYZ;readonly direction:XYZ;readonly radius:number;
  readonly documentPoint:XYZ;readonly documentDirection:XYZ;
  readonly rims:readonly {readonly edge:number;readonly semiMajor:number;readonly semiMinor:number;readonly documentCenter:XYZ}[];
  readonly wallFace:number;readonly minimumClearance:number;readonly maxNativeGap:number;readonly axisDot:number;
}
/** Exact topology correspondence and analytic deltas for one retained-body cut. */
export interface NativeMachiningOperation {
  readonly point:XYZ;
  readonly direction:XYZ;
  readonly radius:number;
  readonly feature:string|null;
  readonly trimmedFaces:readonly [number,number];
  readonly rims:readonly [number,number];
  readonly wallFace:number;
  readonly seamEdge:number;
  readonly minimumClearance:number;
  readonly maxNativeGap:number;
  readonly removedVolume:number;
  readonly surfaceAreaDelta:number;
  readonly faces:readonly {readonly source:number;readonly result:number;readonly change:"retained"|"trimmed"}[];
}
/** Native edit history in source-local coordinates; no display approximation. */
export interface NativeMachining {readonly coordinateSpace:"construction";readonly operations:readonly NativeMachiningOperation[];}
/** Typed origins for one output face in a rectilinear operation's intermediate body. */
export interface NativeFaceSource {readonly operand:"left"|"right";readonly face:number;readonly reversed:boolean;}
/** Exact native operation evidence. IDs are local to the identified graph node. */
export type NativeSolidOperation =
  | {readonly kind:"union"|"intersect"|"subtract";readonly node:number;readonly left:number;readonly right:number;readonly planes:readonly [number,number,number];readonly cells:number;readonly faceTests:number;readonly faceTestBound:number;readonly materialCells:number;readonly materialComponents:number;readonly cavities:number;readonly boundaryShells:number;readonly faceSources:readonly (readonly NativeFaceSource[])[];readonly leftFaces:readonly (readonly number[])[];readonly rightFaces:readonly (readonly number[])[];readonly leftEdges:readonly (readonly number[])[];readonly rightEdges:readonly (readonly number[])[];readonly leftVertices:readonly (number|null)[];readonly rightVertices:readonly (number|null)[];readonly volume:number;readonly volumeBounds:readonly [number,number];readonly surfaceArea:number;readonly surfaceAreaBounds:readonly [number,number];readonly maxGap:number}
  | {readonly kind:"intersectHalfSpace";readonly node:number;readonly base:number;readonly point:XYZ;readonly normal:XYZ;readonly feature?:string;readonly outcome:"unchanged"|"clipped";readonly faces:readonly {readonly source:number;readonly result:number|null;readonly trimmed:boolean}[];readonly edgeImages:readonly (readonly number[])[];readonly splits:readonly {readonly source:number;readonly vertex:number;readonly fraction:number}[];readonly cap:number|null;readonly capEdges:readonly number[];readonly maxGap:number};
/** Native history uses intermediate operation-local IDs, before document placement. */
export interface NativeSolidOperations {readonly coordinateSpace:"construction";readonly operations:readonly NativeSolidOperation[];}
function freezeEvidence<T>(value:T):T {if(value&&typeof value==="object"){for(const child of Object.values(value))freezeEvidence(child);Object.freeze(value);}return value;}
interface PartMetadata { solidOperations?:NativeSolidOperations; machining?:NativeMachining; bore:NativeBore|null; chartKind:ConstructionChart["kind"]|null; family: NativeFamily; name: string; index: number; fingerprint: string; stepSupport: StepSupport; measurements: NativeMeasurements; faces: NativeFace[] }
interface DocumentMetadata { id: number; units: LengthUnit; parts: PartMetadata[] }

/** Buffer-transfer surface implemented by the Wasm bridge. Each buffer is taken once. */
export interface NativeBuffers {
  take_positions(): Float32Array;
  take_normals(): Float32Array;
  take_indices(): Uint32Array;
  take_native_faces(): Uint32Array;
  take_uvs():Float32Array;
  report_json(): string;
  free(): void;
}
export interface NativeStepFile { take_text(): string; report_json(): string; free(): void }
/** Host integration seam; ordinary browser callers use createSession(). */
export interface NativeTransport {
  build_document(request: string): string;
  release_document(document: number): void;
  realize(document: number, part: number, request: string): NativeBuffers;
  export_step(document: number, part: number, request: string): NativeStepFile;
  stats_json(): string;
  workplane_json(document: number, part: number, face: number): string;
  free(): void;
}

function asExedraError(error:unknown):ExedraError {
  if(error instanceof ExedraError)return error;
  if(typeof error === "string") {try {const d=JSON.parse(error) as Diagnostic;if(d.code && d.message)return new ExedraError(d);}catch{/* A non-JSON host refusal is still diagnosed below. */}}
  return new ExedraError({code:"bridge.failure",operation:"bridge",message:String(error)});
}
function nativeCall<T>(action: () => T): T {try{return action()}catch(error){throw asExedraError(error)}}

function graph(parts: Record<string, Solid>, units: LengthUnit, separation: number): unknown {
  const profileIds = new Map<Profile, number>(), nodeIds = new Map<Solid, number>(), pathIds=new Map<CurvePath,number>();
  const visiting = new Set<object>(), profileSpecs: unknown[] = [], nodes: unknown[] = [], pathSpecs:unknown[]=[];
  const visitProfile = (profile: Profile): number => {
    const known = profileIds.get(profile); if (known !== undefined) return known;
    if (visiting.has(profile)) fail("graph.cycle", "Profile graph contains a cycle");
    if (visiting.size >= 128) fail("budget.depth", "Authoring graph nesting exceeds the traversal budget");
    visiting.add(profile);
    const source = profileNodes.get(profile); if (!source) fail("graph.profile", "Expected an Exedra Profile");
    let wire: unknown;
    switch (source.kind) {
      case "tagged": wire = { ...source, profile: visitProfile(source.profile) }; break;
      case "translated":case "corners": wire = { ...source, profile: visitProfile(source.profile) }; break;
      case "cutouts": wire = { ...source, profile: visitProfile(source.profile), template: visitProfile(source.template) }; break;
      default: wire = source;
    }
    if (profileSpecs.length === 256) fail("budget.profiles", "At most 256 profile descriptions are allowed");
    const id = profileSpecs.length; profileSpecs.push(wire); profileIds.set(profile, id); visiting.delete(profile); return id;
  };
  const visit = (solid: Solid): number => {
    const known = nodeIds.get(solid); if (known !== undefined) return known;
    if (visiting.has(solid)) fail("graph.cycle", "Solid graph contains a cycle");
    if (visiting.size >= 128) fail("budget.depth", "Authoring graph nesting exceeds the traversal budget");
    visiting.add(solid);
    const source = solidNodes.get(solid); if (!source) fail("graph.solid", "Expected an Exedra Solid");
    let wire:unknown;
    switch(source.kind){
      case "body":case "sphericalCap":wire=source;break;
      case "loft":wire={...source,sections:source.sections.map(s=>({...s,profile:visitProfile(s.profile)}))};break;
      case "union":case "intersect":case "subtract":wire={...source,base:visit(source.base),other:visit(source.other)};break;
      case "intersectHalfSpace":case "throughBore":case "subtractThroughCylinder":wire={...source,base:visit(source.base)};break;
      case "axialFinish":wire={...source,base:visit(source.base)};break;
      case "sweep": {
        let path=pathIds.get(source.path);
        if(path===undefined){const description=pathNodes.get(source.path);if(!description)fail("graph.path","Expected an Exedra CurvePath");if(pathSpecs.length===256)fail("budget.paths","At most 256 paths are allowed");path=pathSpecs.length;pathSpecs.push(description);pathIds.set(source.path,path);}
        wire={...source,profile:visitProfile(source.profile),path};break;
      }
      case "extrude":case "revolve":wire={...source,profile:visitProfile(source.profile)};break;
      case "pocket":wire={...source,profile:visitProfile(source.profile),base:visit(source.base)};break;
      case "onFace":wire={...source,profile:visitProfile(source.profile),host:visit(source.host)};break;
    }
    if (nodes.length === 256) fail("budget.nodes", "At most 256 construction nodes are allowed");
    const id = nodes.length; nodes.push(wire); nodeIds.set(solid, id); visiting.delete(solid); return id;
  };
  const roots = Object.entries(parts).map(([key, solid]) => ({ name: name(key), node: visit(solid) }));
  return { units, separation, profiles: profileSpecs, ...(pathSpecs.length?{paths:pathSpecs}:{}), nodes, roots };
}
/** Logical owners and payloads; document/cache payloads overlap and exclude render buffers and allocator overhead. */
export interface OwnershipStats {
  documents: number;
  nativeParts: number;
  cacheEntries: number;
  documentNativeSources: number;
  documentBoundaryBytes: number;
  cacheNativeSources: number;
  cacheBoundaryBytes: number;
  maxDocuments: number;
  cacheCapacity: number;
}
let nextSession = 1;

/** Owns native documents and a bounded compiler cache. Dispose when the host releases the session. */
export class AuthoringSession {
  private transport: NativeTransport | null;
  private readonly units: LengthUnit;
  private readonly separation: number;
  private readonly scope = `session-${nextSession++}`;
  private readonly documents = new Set<NativeDocument>();
  private latest: KernelCallTiming | null=null;
  /** The last operation attempt only; reading this or stats does not record another operation. */
  get lastOperation():KernelCallTiming | null {return this.latest;}
  /** @internal */
  beginCall(operation:KernelCallTiming["operation"],scope?:{documentId:string;partId?:string}):CallTiming {return new CallTiming(operation,timing=>{this.latest=timing},scope);}
  constructor(transport: NativeTransport, options: { units: LengthUnit; separation?: number }) {
    this.transport = transport;
    this.units = options.units;
    this.separation = finite(options.separation ?? (options.units === "mm" ? 1e-6 : 1e-9), "separation");
  }
  /** Builds the shared graph atomically; a refusal leaves earlier documents usable. */
  build<T extends Record<string, Solid>>(parts: T): NativeDocument & { readonly parts: { readonly [K in keyof T]: NativePart } } {
    const call=this.beginCall("document.build");
    try {
      const transport=this.backend();
      const request=call.phase("sdkPrepareMs",()=>JSON.stringify(graph(parts,this.units,this.separation)));
      const result=call.phase("kernelMs",()=>transport.build_document(request));
      const document=call.phase("metadataMs",()=>{
        const metadata=JSON.parse(result) as DocumentMetadata;
        const document=new NativeDocument(this,metadata,`${this.scope}/document-${metadata.id}`,call);
        this.documents.add(document);call.scope(document.id);return document;
      });
      return document as NativeDocument & {readonly parts:{readonly [K in keyof T]:NativePart}};
    } catch(error){throw call.refusal(error)} finally{call.finish("success")}
  }

  stats(): OwnershipStats {
    return JSON.parse(nativeCall(() => this.backend().stats_json()));
  }
  dispose(): void {
    if (!this.transport) return;
    for (const document of [...this.documents]) document.dispose();
    this.transport.free(); this.transport = null;
  }
  /** @internal */
  backend(): NativeTransport {
    if (!this.transport) fail("lifecycle.sessionDisposed", "This authoring session has been disposed");
    return this.transport;
  }
  /** @internal */
  release(document: NativeDocument, id: number): void {
    if (!this.documents.delete(document)) return;
    nativeCall(() => this.backend().release_document(id));
  }
}

/** One immutable installed revision. Quality changes realize this same native source. */
export class NativeDocument {
  readonly parts: Readonly<Record<string, NativePart>>;
  readonly id: string;
  readonly units: LengthUnit;
  private readonly session: AuthoringSession;
  private readonly handle: number;
  private live = true;
  private readonly buildCall:CallTiming;
  constructor(session: AuthoringSession, metadata: DocumentMetadata, scope: string, buildCall:CallTiming) {
    this.buildCall=buildCall; this.session = session; this.handle = metadata.id; this.id = scope; this.units = metadata.units;
    this.parts = Object.freeze(Object.fromEntries(metadata.parts.map(part => [part.name, new NativePart(this, part)])));
  }
  /** Build/validation boundary timing; remains a plain query after disposal. */
  get buildTiming():KernelCallTiming {return this.buildCall.value;}
  /** @internal */
  beginCall(operation:KernelCallTiming["operation"],partId:string):CallTiming {return this.session.beginCall(operation,{documentId:this.id,partId});}
  dispose(): void { if (this.live) { this.session.release(this, this.handle); this.live = false; } }
  /** @internal */
  backend(): { transport: NativeTransport; document: number } {
    if (!this.live) fail("lifecycle.documentDisposed", "This native document has been disposed");
    return { transport: this.session.backend(), document: this.handle };
  }
}

/** A queryable immutable native part, scoped to one installed document. */
export class NativePart {
  readonly family: NativeFamily;
  /** Structural chart operation, or null when native chart realization is unavailable. */
  readonly chartKind:ConstructionChart["kind"]|null;
  readonly bore:NativeBore|null;
  readonly machining:NativeMachining|null;
  readonly solidOperations:NativeSolidOperations|null;
  readonly name: string;
  readonly id: string;
  readonly fingerprint: string;
  readonly measurements: Readonly<NativeMeasurements>;
  readonly stepSupport: StepSupport;
  private readonly document: NativeDocument;
  private readonly index: number;
  private readonly faceTable: readonly NativeFace[];
  constructor(document: NativeDocument, metadata: PartMetadata) {
    this.chartKind=metadata.chartKind;
    this.solidOperations=metadata.solidOperations?freezeEvidence(metadata.solidOperations):null;
    this.machining=metadata.machining?Object.freeze({...metadata.machining,operations:Object.freeze(metadata.machining.operations.map(operation=>Object.freeze({...operation,point:point3(operation.point),direction:point3(operation.direction),trimmedFaces:Object.freeze([...operation.trimmedFaces]) as readonly [number,number],rims:Object.freeze([...operation.rims]) as readonly [number,number],faces:Object.freeze(operation.faces.map(face=>Object.freeze({...face})))})))}):null;
    this.bore=metadata.bore?Object.freeze({...metadata.bore,point:point3(metadata.bore.point),direction:point3(metadata.bore.direction),documentPoint:point3(metadata.bore.documentPoint),documentDirection:point3(metadata.bore.documentDirection),rims:Object.freeze(metadata.bore.rims.map(r=>Object.freeze({...r,documentCenter:point3(r.documentCenter)})))}):null;
    this.family = metadata.family; this.document = document; this.index = metadata.index; this.name = metadata.name;
    this.id = `${document.id}/part-${metadata.index}`; this.fingerprint = metadata.fingerprint;
    this.stepSupport = metadata.stepSupport.status === "supported" ? Object.freeze({...metadata.stepSupport}) : Object.freeze({...metadata.stepSupport,diagnostic:Object.freeze({...metadata.stepSupport.diagnostic})});
    const freezeMeasure = (value: NativeMeasurement): NativeMeasurement => value.status === "available" ? Object.freeze({...value}) : Object.freeze({...value,diagnostic:Object.freeze({...value.diagnostic})});
    this.measurements = Object.freeze({ ...metadata.measurements, volume:freezeMeasure(metadata.measurements.volume),surfaceArea:freezeMeasure(metadata.measurements.surfaceArea), bounds: Object.freeze(metadata.measurements.bounds.map(corner => Object.freeze([...corner]))) as unknown as NativeMeasurements["bounds"] });
    this.faceTable = Object.freeze(metadata.faces.map(face => Object.freeze({...face,features:Object.freeze([...face.features])})));
    Object.freeze(this);
  }
  faces(feature?: string): readonly FaceSelection[] {
    this.document.backend();
    return this.faceTable.filter(f => feature === undefined || f.features.includes(feature)).map(f => new FaceSelection(this, f));
  }
  /** Exact construction cap; full revolutions have no end caps and are refused explicitly. */
  cap(end:End):FaceSelection {
    this.document.backend();const face=this.faceTable.find(face=>face.role==="cap"&&face.end===end);
    if(!face)fail("selection.missingCap",`No ${end} cap exists in '${this.name}'`);
    return new FaceSelection(this,face);
  }
  /** Returns one labelled floor, with distinct missing and ambiguous diagnostics. */
  floor(feature: string): FaceSelection {
    const floors = this.faces(feature).filter(f => f.role === "floor");
    if (!floors.length) fail("selection.missing", `No floor named '${feature}' exists in '${this.name}'`);
    if (floors.length !== 1) fail("selection.ambiguous", `More than one floor is named '${feature}' in '${this.name}'`);
    return floors[0];
  }
  realize(options: { tolerance: number; maxVertices?: number; chart?:ConstructionChart }): Realization {
    const call=this.document.beginCall("part.realize",this.id);
    try {
      const {transport,document}=this.document.backend();
      const request=call.phase("sdkPrepareMs",()=>JSON.stringify({tolerance:finite(options.tolerance,"tolerance"),maxVertices:options.maxVertices ?? 250000,...(options.chart?{chart:snapshotChart(options.chart)}:{})}));
      const raw=call.phase("kernelMs",()=>transport.realize(document,this.index,request));
      try {
        const report=call.phase("metadataMs",()=>JSON.parse(raw.report_json()) as RealizationReport);
        const buffers=call.phase("transferMs",()=>({positions:raw.take_positions(),normals:raw.take_normals(),indices:raw.take_indices(),nativeFaces:raw.take_native_faces(),uvs:report.hasUvs?raw.take_uvs():null}));
        return call.phase("metadataMs",()=>new Realization(this,buffers.positions,buffers.normals,buffers.indices,buffers.nativeFaces,buffers.uvs,report,call));
      } finally {call.phase("cleanupMs",()=>raw.free());}
    } catch(error){throw call.refusal(error)} finally{call.finish("success")}
  }
  exportStep(options: { name?: string; timestamp: string; tolerance?: number }): StepFile {
    const call=this.document.beginCall("part.exportStep",this.id);
    try {
      const {transport,document}=this.document.backend();
      const request=call.phase("sdkPrepareMs",()=>JSON.stringify({name:options.name ?? this.name,timestamp:options.timestamp,tolerance:options.tolerance}));
      const raw=call.phase("kernelMs",()=>transport.export_step(document,this.index,request));
      try {
        const text=call.phase("transferMs",()=>raw.take_text());
        const report=call.phase("metadataMs",()=>JSON.parse(raw.report_json()));
        return {text,report,get timing(){return call.value}};
      } finally {call.phase("cleanupMs",()=>raw.free());}
    } catch(error){throw call.refusal(error)} finally{call.finish("success")}
  }
  /** Rejects a selection from a different installed revision, even if face indices happen to agree. */
  requireSelection(selection: FaceSelection): FaceSelection {
    this.document.backend();
    if (selection.partId !== this.id) fail("selection.stale", "The selection belongs to a different native part/revision");
    return this.face(selection.nativeId);
  }
  /** Reads the exact native supporting plane; curved faces return a typed refusal. */
  workplane(selection: FaceSelection): NativeWorkplane {
    const call=this.document.beginCall("part.workplane",this.id);
    try {
      const checked=this.requireSelection(selection);
      const {transport,document}=this.document.backend();
      const result=call.phase("kernelMs",()=>transport.workplane_json(document,this.index,checked.nativeId));
      const frame=call.phase("metadataMs",()=>JSON.parse(result) as Omit<NativeWorkplane,"timing">);
      return Object.freeze({origin:Object.freeze(frame.origin),xAxis:Object.freeze(frame.xAxis),yAxis:Object.freeze(frame.yAxis),zAxis:Object.freeze(frame.zAxis),get timing(){return call.value}});
    } catch(error){throw call.refusal(error)} finally{call.finish("success")}
  }
  /** @internal */
  face(id: number): FaceSelection {
    this.document.backend();
    const face = this.faceTable.find(f => f.id === id);
    if (!face) fail("selection.face", `Native face ${id} does not exist in '${this.name}'`);
    return new FaceSelection(this, face);
  }
}
/** Native face identity with its authored document scope; independent of display quality. */
export class FaceSelection {
  private readonly part: NativePart;
  readonly partId: string;
  readonly nativeId: number;
  readonly feature?: string;
  readonly features: readonly string[];
  readonly end?:End;
  readonly role: NativeFace["role"];
  readonly surface: NativeFace["surface"];
  constructor(part: NativePart, face: NativeFace) {
    this.part = part;
    this.partId = part.id; this.nativeId = face.id; this.features = Object.freeze([...face.features]); this.feature = this.features.length === 1 ? this.features[0] : undefined;
    this.end=face.end;this.role = face.role; this.surface = face.surface; Object.freeze(this);
  }
  workplane(): NativeWorkplane { return this.part.workplane(this); }
}
/** An installed native supporting frame, expressed in document coordinates. */
export interface NativeWorkplane {
  readonly timing:KernelCallTiming;
  readonly origin: readonly [number, number, number];
  readonly xAxis: readonly [number, number, number];
  readonly yAxis: readonly [number, number, number];
  readonly zAxis: readonly [number, number, number];
}
/** One native part export, with caller-owned text, report and boundary timing. */
export interface StepFile { readonly text:string; readonly report:StepReport; readonly timing:KernelCallTiming; }
/** CPU display buffers owned by the caller. The originating native part remains unchanged. */
export class Realization {
  readonly positions: Float32Array;
  readonly normals: Float32Array;
  readonly indices: Uint32Array;
  readonly nativeFaces: Uint32Array;
  /** Primary construction UVs, or null when no chart was applied; caller-owned. */
  readonly uvs:Float32Array|null;
  readonly report: Readonly<RealizationReport>;
  /** Host boundary timing including buffer transfer and handle release. */
  get timing():KernelCallTiming {return this.call.value;}
  private readonly part: NativePart;
  private readonly call:CallTiming;
  constructor(part: NativePart, positions: Float32Array, normals: Float32Array, indices: Uint32Array, nativeFaces: Uint32Array, uvs:Float32Array|null, report: RealizationReport, call:CallTiming) {
    this.call=call; this.part = part; this.positions = positions; this.normals = normals; this.indices = indices; this.nativeFaces = nativeFaces;this.uvs=uvs;
    this.report = Object.freeze({...report,chart:report.chart?snapshotChart(report.chart):null,work:Object.freeze({...report.work}),curveRefinement:report.curveRefinement?Object.freeze({...report.curveRefinement,diagnostic:report.curveRefinement.diagnostic?Object.freeze({...report.curveRefinement.diagnostic}):undefined}):null});
    if(uvs && uvs.length*3!==positions.length*2)fail("bridge.uvs","Construction UV/render vertex correspondence is incomplete");
    if(Boolean(uvs)!==Boolean(report.hasUvs))fail("bridge.uvs","Construction chart availability does not match the transferred UV buffer");
    if (nativeFaces.length * 3 !== indices.length) fail("bridge.correspondence", "Display triangle/native-face correspondence is incomplete");
  }
  pickTriangle(triangle: number): FaceSelection {
    if (!Number.isInteger(triangle) || triangle < 0 || triangle >= this.nativeFaces.length)
      fail("selection.triangle", "Triangle index is outside this realization");
    return this.part.face(this.nativeFaces[triangle]);
  }
}

/** Loads the Wasm kernel and creates an explicit ownership boundary for authored documents. */
export async function createSession(options: { units: LengthUnit; separation?: number }): Promise<AuthoringSession> {
  const wasm = await import("./wasm_pkg/exedra_authoring_web_bridge.js");
  await wasm.default();
  return new AuthoringSession(new wasm.AuthoringKernel(), options);
}
