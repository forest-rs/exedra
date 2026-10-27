// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/** Model lengths are expressed in the document's explicitly selected unit. */
export type LengthUnit = "mm" | "m";
export type XY = readonly [number, number];
export type XYZ = readonly [number, number, number];
export type NativeFamily = "extrusion" | "revolution" | "sweep" | "loft";
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
export type End = "start" | "end";

export interface Diagnostic {
  code: string;
  operation: string;
  message: string;
  node?: number;
  profile?: number;
  feature?: string;
  family?: NativeFamily;
  measure?: "volume" | "surfaceArea";
  face?: number;
  geometryFamily?: string;
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
  readonly diagnostic: Diagnostic;
  readonly timing?: KernelCallTiming;
  constructor(diagnostic: Diagnostic, timing?: KernelCallTiming) {
    super(diagnostic.message);
    this.name = "ExedraError";
    this.diagnostic = Object.freeze({ ...diagnostic });
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

type ProfileNode =
  | { kind: "polygon"; points: readonly XY[] }
  | { kind: "rectangle"; width: number; height: number }
  | { kind: "roundedRectangle"; width: number; height: number; radius: number }
  | { kind: "circle"; radius: number }
  | { kind: "slot"; length: number; radius: number }
  | { kind: "tagged"; profile: Profile; name: string }
  | { kind: "translated"; profile: Profile; offset: XY }
  | { kind: "cutouts"; profile: Profile; template: Profile; positions: readonly XY[] };
const profileNodes = new WeakMap<Profile, ProfileNode>();

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
  rectangle: Profile.rectangle,
  roundedRectangle: Profile.roundedRectangle,
  circle: Profile.circle,
  slot: Profile.slot,
});

type FaceSelector = { kind: "floor"; feature: string } | { kind: "cap"; end: End };
type SolidNode =
  | { kind: "extrude"; profile: Profile; height: number; placement: Placement }
  | { kind: "revolve"; profile: Profile; angle: number; placement: Placement }
  | { kind: "pocket"; base: Solid; profile: Profile; end: End; depth: number; at: XY }
  | { kind: "onFace"; host: Solid; face: FaceSelector; profile: Profile; height: number; offset: number; at: XY };
const solidNodes = new WeakMap<Solid, SolidNode>();

/** Immutable native construction intent, distinct from an installed/queryable part. */
export class Solid {
  private constructor(node: SolidNode) {
    solidNodes.set(this, Object.freeze(node));
    Object.freeze(this);
  }
  static extrude(profile: Profile, options: { height: number; placement?: Placement }): Solid {
    return new Solid({ kind: "extrude", profile, height: finite(options.height, "height"), placement: placement(options.placement) });
  }
  /** Revolves profile X as positive radius and profile Y as height, about local +Y. */
  static revolve(profile: Profile, options: { angle?: number; placement?: Placement } = {}): Solid {
    return new Solid({ kind: "revolve", profile, angle: finite(options.angle ?? 2*Math.PI,"angle"), placement: placement(options.placement) });
  }
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
export const extrude = Solid.extrude;
export const revolve = Solid.revolve;

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
  | { readonly status: "available"; readonly value: number }
  | { readonly status: "unavailable" | "numericLimit"; readonly diagnostic: Readonly<Diagnostic> };
export interface NativeFace {
  id: number;
  features: readonly string[];
  feature?: string;
  role: "cap" | "wall" | "floor";
  surface: "plane" | "cylinder" | "cone" | "torus" | "ruled";
}
export interface RealizationReport {
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
export interface StepReport {
  schema: string;
  units: LengthUnit;
  declaredUncertainty: number;
  maxDeviation: number;
  maxNativeSupportGap: number;
  frameRoundoffBound: number;
  entities: number;
  faces: number;
  edges: number;
  /** Entries use native local IDs as indices; unused topology entries are null. */
  correspondence: {
    vertices: readonly (number | null)[];
    edges: readonly (number | null)[];
    loops: readonly (number | null)[];
    faces: readonly (number | null)[];
  };
}
interface PartMetadata { family: NativeFamily; name: string; index: number; fingerprint: string; stepSupport: StepSupport; measurements: NativeMeasurements; faces: NativeFace[] }
interface DocumentMetadata { id: number; units: LengthUnit; parts: PartMetadata[] }

/** Buffer-transfer surface implemented by the Wasm bridge. Each buffer is taken once. */
export interface NativeBuffers {
  take_positions(): Float32Array;
  take_normals(): Float32Array;
  take_indices(): Uint32Array;
  take_native_faces(): Uint32Array;
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
  const profileIds = new Map<Profile, number>(), nodeIds = new Map<Solid, number>();
  const visiting = new Set<object>(), profileSpecs: unknown[] = [], nodes: unknown[] = [];
  const visitProfile = (profile: Profile): number => {
    const known = profileIds.get(profile); if (known !== undefined) return known;
    if (visiting.has(profile)) fail("graph.cycle", "Profile graph contains a cycle");
    if (visiting.size >= 128) fail("budget.depth", "Authoring graph nesting exceeds the traversal budget");
    visiting.add(profile);
    const source = profileNodes.get(profile); if (!source) fail("graph.profile", "Expected an Exedra Profile");
    let wire: unknown;
    switch (source.kind) {
      case "tagged": wire = { ...source, profile: visitProfile(source.profile) }; break;
      case "translated": wire = { ...source, profile: visitProfile(source.profile) }; break;
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
    const profile = visitProfile(source.profile);
    const wire = source.kind === "extrude" || source.kind === "revolve" ? { ...source, profile }
      : source.kind === "pocket" ? { ...source, profile, base: visit(source.base) }
      : { ...source, profile, host: visit(source.host) };
    if (nodes.length === 256) fail("budget.nodes", "At most 256 construction nodes are allowed");
    const id = nodes.length; nodes.push(wire); nodeIds.set(solid, id); visiting.delete(solid); return id;
  };
  const roots = Object.entries(parts).map(([key, solid]) => ({ name: name(key), node: visit(solid) }));
  return { units, separation, profiles: profileSpecs, nodes, roots };
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
  readonly name: string;
  readonly id: string;
  readonly fingerprint: string;
  readonly measurements: Readonly<NativeMeasurements>;
  readonly stepSupport: StepSupport;
  private readonly document: NativeDocument;
  private readonly index: number;
  private readonly faceTable: readonly NativeFace[];
  constructor(document: NativeDocument, metadata: PartMetadata) {
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
  /** Returns one labelled floor, with distinct missing and ambiguous diagnostics. */
  floor(feature: string): FaceSelection {
    const floors = this.faces(feature).filter(f => f.role === "floor");
    if (!floors.length) fail("selection.missing", `No floor named '${feature}' exists in '${this.name}'`);
    if (floors.length !== 1) fail("selection.ambiguous", `More than one floor is named '${feature}' in '${this.name}'`);
    return floors[0];
  }
  realize(options: { tolerance: number; maxVertices?: number }): Realization {
    const call=this.document.beginCall("part.realize",this.id);
    try {
      const {transport,document}=this.document.backend();
      const request=call.phase("sdkPrepareMs",()=>JSON.stringify({tolerance:finite(options.tolerance,"tolerance"),maxVertices:options.maxVertices ?? 250000}));
      const raw=call.phase("kernelMs",()=>transport.realize(document,this.index,request));
      try {
        const buffers=call.phase("transferMs",()=>({positions:raw.take_positions(),normals:raw.take_normals(),indices:raw.take_indices(),nativeFaces:raw.take_native_faces()}));
        const report=call.phase("metadataMs",()=>JSON.parse(raw.report_json()));
        return new Realization(this,buffers.positions,buffers.normals,buffers.indices,buffers.nativeFaces,report,call);
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
  readonly role: NativeFace["role"];
  readonly surface: NativeFace["surface"];
  constructor(part: NativePart, face: NativeFace) {
    this.part = part;
    this.partId = part.id; this.nativeId = face.id; this.features = Object.freeze([...face.features]); this.feature = this.features.length === 1 ? this.features[0] : undefined;
    this.role = face.role; this.surface = face.surface; Object.freeze(this);
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
  readonly report: Readonly<RealizationReport>;
  /** Host boundary timing including buffer transfer and handle release. */
  get timing():KernelCallTiming {return this.call.value;}
  private readonly part: NativePart;
  private readonly call:CallTiming;
  constructor(part: NativePart, positions: Float32Array, normals: Float32Array, indices: Uint32Array, nativeFaces: Uint32Array, report: RealizationReport, call:CallTiming) {
    this.call=call; this.part = part; this.positions = positions; this.normals = normals; this.indices = indices; this.nativeFaces = nativeFaces; this.report = Object.freeze(report);
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
