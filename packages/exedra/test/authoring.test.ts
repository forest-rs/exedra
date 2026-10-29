import assert from "node:assert/strict";
import { test } from "node:test";
import {
  AuthoringSession,
  AuthoringWorkplane,
  Solid,
  ExedraError,
  extrude,
  revolve,
  sphericalCap,
  sweep,
  loft,
  paths,
  placement,
  profiles,
  charts,
  uvTransform,
  type NativeTransport,
} from "../src/index.ts";

function host() {
  let next = 0,
    released = 0,
    freed = 0,
    transfers = 0;
  const requests: any[] = [];
  const native: NativeTransport = {
    build_document(text) {
      const graph = JSON.parse(text);
      requests.push(graph);
      if (graph.nodes.some((n: any) => n.depth === 99))
        throw JSON.stringify({
          code: "pocket.invalid",
          operation: "solid.pocket",
          message: "Depth exceeds panel thickness",
          node: 1,
        });
      return JSON.stringify({
        id: ++next,
        units: graph.units,
        parts: graph.roots.map((r: any, index: number) => ({
          chartKind: null,
          family: "extrusion",
          name: r.name,
          index,
          fingerprint: "f123456789abcdef123456789abcdef0",
          stepSupport: { status: "supported" },
          measurements: {
            volume: { status: "available", value: 10 },
            surfaceArea: { status: "available", value: 20 },
            boundsKind: "conservativeNativeSupport",
            bounds: [
              [0, 0, 0],
              [2, 3, 4],
            ],
            faces: 2,
            edges: 3,
            vertices: 4,
          },
          faces: [
            {
              id: 0,
              role: "floor",
              surface: "plane",
              feature: "recess",
              features: ["recess"],
            },
            {
              id: 1,
              role: "wall",
              surface: "cylinder",
              feature: "vents",
              features: ["vents"],
            },
          ],
        })),
      });
    },
    release_document() {
      released++;
    },
    stats_json() {
      return JSON.stringify({
        documents: next - released,
        nativeParts: next - released,
        cacheEntries: 0,
      });
    },
    workplane_json() {
      return JSON.stringify({
        origin: [0, 0, 7],
        xAxis: [1, 0, 0],
        yAxis: [0, 1, 0],
        zAxis: [0, 0, 1],
      });
    },
    realize() {
      return {
        take_positions() {
          transfers++;
          return new Float32Array(9);
        },
        take_normals() {
          transfers++;
          return new Float32Array(9);
        },
        take_indices() {
          transfers++;
          return new Uint32Array([0, 1, 2]);
        },
        take_native_faces() {
          transfers++;
          return new Uint32Array([1]);
        },
        take_uvs() {
          transfers++;
          return new Float32Array(6);
        },
        report_json() {
          return JSON.stringify({
            hasUvs: false,
            chart: null,
            work: {
              attempts: 1,
              completedBoundarySamples: 3,
              completedTriangles: 1,
            },
            curveRefinement: null,
            tolerance: 0.1,
            maxDeviation: 0.01,
            triangles: 1,
            vertices: 3,
          });
        },
        free() {
          freed++;
        },
      };
    },
    export_step() {
      throw new Error("unused");
    },
    free() {
      freed++;
    },
  };
  return { native, requests, counters: () => ({ released, freed, transfers }) };
}
function sample() {
  const vent = profiles.slot({ length: 22, radius: 3 }).tagged("vents");
  const panel = extrude(
    profiles.rectangle({ width: 120, height: 80 }).withCutoutsAt(vent, [
      [-12, 20],
      [12, 20],
    ]),
    { height: 12 },
  ).pocket(profiles.circle({ radius: 8 }).tagged("recess"), {
    end: "end",
    depth: 5,
  });
  return {
    panel,
    pin: panel
      .floor("recess")
      .workplane()
      .offset(0.5)
      .extrude(profiles.circle({ radius: 3 }), { height: 7 }),
  };
}
function code(code: string) {
  return (error: unknown) =>
    error instanceof ExedraError && error.diagnostic.code === code;
}

test("reusable descriptors lower once and all shared native dependencies are explicit", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const doc = session.build(sample()),
    graph = h.requests[0];
  assert.equal(graph.profiles.filter((p: any) => p.kind === "slot").length, 1);
  assert.equal(graph.nodes.length, 3);
  assert.equal(graph.nodes[2].host, graph.roots[0].node);
  assert.equal(doc.parts.panel.fingerprint, "f123456789abcdef123456789abcdef0");
  doc.dispose();
  session.dispose();
});
test("quality retains face scope; a fresh authored revision rejects old selections", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const first = session.build(sample()),
    selection = first.parts.panel.floor("recess");
  const coarse = first.parts.panel.realize({ tolerance: 0.45 }),
    fine = first.parts.panel.realize({ tolerance: 0.035 });
  assert.equal(coarse.pickTriangle(0).partId, fine.pickTriangle(0).partId);
  assert.equal(first.parts.panel.requireSelection(selection).nativeId, 0);
  assert.deepEqual(selection.workplane().origin, [0, 0, 7]);
  const second = session.build(sample());
  assert.throws(
    () => second.parts.panel.requireSelection(selection),
    code("selection.stale"),
  );
  first.dispose();
  assert.throws(() => fine.pickTriangle(0), code("lifecycle.documentDisposed"));
  session.dispose();
  assert.deepEqual(h.counters(), { released: 2, freed: 3, transfers: 8 });
});
test("failed authoring leaves existing documents usable and preserves typed diagnostics", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const first = session.build(sample());
  const bad = extrude(profiles.rectangle({ width: 20, height: 20 }), {
    height: 12,
  }).pocket(profiles.circle({ radius: 3 }), { end: "end", depth: 99 });
  assert.throws(() => session.build({ bad }), code("pocket.invalid"));
  assert.equal(first.parts.panel.floor("recess").nativeId, 0);
  first.dispose();
  first.dispose();
  session.dispose();
  session.dispose();
  assert.equal(h.counters().released, 1);
  assert.throws(
    () => session.build(sample()),
    code("lifecycle.sessionDisposed"),
  );
});
test("unbounded placement iterators are refused before reaching the native host", () => {
  function* endless() {
    while (true) yield [0, 0] as const;
  }
  assert.throws(
    () =>
      profiles
        .rectangle({ width: 10, height: 10 })
        .withCutoutsAt(profiles.circle({ radius: 1 }), endless()),
    code("budget.cutouts"),
  );
});

test("public workplane seams snapshot mutable caller arrays and selectors", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const hostSolid = extrude(profiles.rectangle({ width: 20, height: 20 }), {
    height: 12,
  });
  const at: [number, number] = [1, 2],
    face: { kind: "cap"; end: "start" | "end" } = { kind: "cap", end: "end" };
  const pin = new AuthoringWorkplane(hostSolid, face, 0, at).extrude(
    profiles.circle({ radius: 1 }),
    { height: 1 },
  );
  const direct = Solid.onFace(
    hostSolid,
    face,
    profiles.circle({ radius: 1 }),
    1,
    0,
    at,
  );
  session.build({ pin, direct }).dispose();
  at[0] = 99;
  face.end = "start";
  session.build({ pin, direct }).dispose();
  assert.deepEqual(h.requests[0], h.requests[1]);
  session.dispose();
});

test("native measurement bounds are immutable nested query values", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const doc = session.build(sample()),
    bounds = doc.parts.panel.measurements.bounds;
  assert.throws(() => {
    (bounds[0] as number[])[0] = 123;
  }, TypeError);
  assert.deepEqual(bounds, [
    [0, 0, 0],
    [2, 3, 4],
  ]);
  session.dispose();
});

test("polygon and revolution placements snapshot caller-owned authoring inputs", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const vertices: [number, number][] = [
    [12, 0],
    [19, 0],
    [19, 30],
    [12, 30],
  ];
  const translation: [number, number, number] = [2, 3, 4],
    rotation: [number, number, number] = [Math.PI / 2, 0, 0];
  const collar = revolve(profiles.polygon(vertices).tagged("collar"), {
    angle: Math.PI,
    placement: placement({ translation, rotationXYZ: rotation }),
  });
  session.build({ collar }).dispose();
  vertices[0][0] = 99;
  translation[0] = 100;
  rotation[0] = 0;
  session.build({ collar }).dispose();
  assert.deepEqual(h.requests[0], h.requests[1]);
  assert.equal(h.requests[0].nodes[0].kind, "revolve");
  session.dispose();
});

test("call timing is scoped immutable evidence and diagnostic reads do not create events", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const doc = session.build(sample()),
    build = doc.buildTiming;
  assert.equal(build.operation, "document.build");
  assert.equal(build.outcome, "success");
  assert.equal(build.documentId, doc.id);
  assert.ok(Object.isFrozen(build));
  assert.strictEqual(session.lastOperation, build);
  session.stats();
  assert.strictEqual(session.lastOperation, build);
  const display = doc.parts.panel.realize({ tolerance: 0.1 }),
    timing = display.timing;
  assert.equal(timing.operation, "part.realize");
  assert.equal(timing.partId, doc.parts.panel.id);
  for (const key of [
    "kernelMs",
    "sdkPrepareMs",
    "transferMs",
    "metadataMs",
    "cleanupMs",
    "totalMs",
  ] as const)
    assert.ok(Number.isFinite(timing[key]) && timing[key] >= 0);
  assert.equal(h.counters().freed, 1);
  session.stats();
  assert.strictEqual(session.lastOperation, timing);
  assert.strictEqual(doc.buildTiming, build);
  h.native.export_step = () => {
    throw JSON.stringify({
      code: "exchange.unsupportedSurface",
      operation: "part.exportStep",
      message: "Torus faces are not supported",
    });
  };
  assert.throws(
    () => doc.parts.panel.exportStep({ timestamp: "2026-10-27T00:00:00Z" }),
    (error) => {
      assert.ok(error instanceof ExedraError);
      assert.equal(error.timing?.outcome, "refused");
      assert.equal(error.timing?.operation, "part.exportStep");
      assert.equal(error.timing?.diagnosticCode, "exchange.unsupportedSurface");
      return true;
    },
  );
  const refusal = session.lastOperation;
  session.stats();
  assert.strictEqual(session.lastOperation, refusal);
  doc.dispose();
  assert.strictEqual(display.timing, timing);
  session.dispose();
});

test("transfer or metadata failures release raw buffers and retain refusal timing", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" }),
    doc = session.build(sample());
  const realize = h.native.realize;
  h.native.realize = (...args) => {
    const raw = realize(...args);
    return {
      ...raw,
      report_json() {
        throw new Error("malformed metadata");
      },
    };
  };
  assert.throws(
    () => doc.parts.panel.realize({ tolerance: 0.1 }),
    (error) =>
      error instanceof ExedraError && error.timing?.outcome === "refused",
  );
  assert.equal(h.counters().freed, 1);
  assert.equal(session.lastOperation?.operation, "part.realize");
  assert.equal(doc.parts.panel.floor("recess").nativeId, 0);
  session.dispose();
});

test("reusable paths, placed loft sections and named axial finishes preserve immutable intent", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const start: [number, number, number] = [0, 0, 0],
    normal: [number, number, number] = [0, 0, 1];
  const path = paths
    .start(start, { tangent: [0, 1, 0], sectionX: [1, 0, 0] })
    .lineBy([0, 30, 0])
    .tangentArc({ radius: 20, angle: Math.PI / 2, normal });
  const profile = profiles.circle({ radius: 3 }).tagged("rail");
  const translation: [number, number, number] = [0, 0, 30];
  const sections = [
    { profile, placement: placement() },
    { profile, placement: placement({ translation }) },
  ];
  const ruled = loft(sections);
  const finished = extrude(
    profiles.rectangle({ width: 20, height: 16 }).tagged("body"),
    { height: 12 },
  )
    .axialEdges("body")
    .fillet({ radius: 2, feature: "round" });
  const parts = {
    rail: sweep(profile, path),
    second: sweep(profile, path),
    ruled,
    finished,
  };
  const first = session.build(parts);
  first.dispose();
  start[0] = 99;
  normal[2] = 0;
  translation[2] = 999;
  sections.pop();
  const second = session.build(parts);
  second.dispose();
  assert.deepEqual(h.requests[0], h.requests[1]);
  assert.equal(h.requests[0].paths.length, 1);
  assert.equal(
    h.requests[0].nodes.filter((n: any) => n.kind === "sweep").length,
    2,
  );
  const finish = h.requests[0].nodes.find((n: any) => n.kind === "axialFinish");
  assert.equal(finish.feature, "body");
  assert.equal(finish.transition, "round");
  function* endless() {
    while (true) yield { profile, placement: placement() };
  }
  assert.throws(() => loft(endless()), code("budget.sections"));
  session.dispose();
});

test("construction chart descriptions snapshot caller transforms without changing model identity", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const matrix: [[number, number], [number, number]] = [
      [0.05, 0],
      [0, 0.05],
    ],
    offset: [number, number] = [0.25, 0.5];
  const chart = charts.revolve({
    referenceRadius: 28,
    wall: uvTransform({ matrix, offset }),
  });
  matrix[0][0] = 999;
  offset[0] = 999;
  let captured: any;
  const original = h.native.realize;
  h.native.realize = (document, part, request) => {
    captured = JSON.parse(request);
    return original(document, part, request);
  };
  const document = session.build(sample()),
    part = document.parts.panel;
  part.realize({ tolerance: 0.12, chart });
  assert.equal(captured.chart.wall.matrix[0][0], 0.05);
  assert.equal(captured.chart.wall.offset[0], 0.25);
  assert.equal(captured.chart.referenceRadius, 28);
  assert.equal(document.parts.panel.id, part.id);
  assert.ok(Object.isFrozen(chart));
  assert.ok(Object.isFrozen(chart.wall.matrix[0]));
  assert.throws(
    () => charts.loft({ restLength: 100, referenceSection: -1 }),
    code("input.referenceSection"),
  );
  assert.throws(
    () => uvTransform({ offset: [Infinity, 0] }),
    code("input.nonFinite"),
  );
  document.dispose();
  session.dispose();
});

test("charted buffer transfer is conditional, layout-checked and caller-owned after disposal", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const original = h.native.realize;
  let uvTakes = 0,
    malformed = false;
  h.native.realize = (...args) => {
    const raw = original(...args),
      report = JSON.parse(raw.report_json());
    return {
      ...raw,
      report_json: () =>
        JSON.stringify({ ...report, hasUvs: true, chart: charts.sweep() }),
      take_uvs() {
        uvTakes++;
        return new Float32Array(malformed ? 4 : 6);
      },
    };
  };
  const document = session.build(sample()),
    part = document.parts.panel;
  const display = part.realize({ tolerance: 0.12, chart: charts.sweep() });
  assert.equal(uvTakes, 1);
  assert.equal(display.uvs!.length, 6);
  assert.ok(Object.isFrozen(display.report.chart));
  malformed = true;
  assert.throws(
    () => part.realize({ tolerance: 0.12, chart: charts.sweep() }),
    (error) =>
      error instanceof ExedraError &&
      error.diagnostic.code === "bridge.uvs" &&
      error.timing?.outcome === "refused",
  );
  assert.equal(uvTakes, 2);
  assert.equal(h.counters().freed, 2);
  document.dispose();
  display.uvs![0] = 9;
  assert.equal(display.uvs![0], 9);
  assert.throws(
    () => display.pickTriangle(0),
    code("lifecycle.documentDisposed"),
  );
  session.dispose();
});

test("bore point and direction snapshot construction-space inputs before a placed source is built", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const point: [number, number, number] = [0, 0, 10],
    direction: [number, number, number] = [1, 0, 1];
  const plate = extrude(profiles.rectangle({ width: 100, height: 70 }), {
    height: 20,
    placement: placement({
      translation: [100, 200, 300],
      rotationXYZ: [0.2, 0.3, 0.4],
    }),
  }).throughBore({
    point,
    direction,
    radius: 8,
    separation: 2,
    feature: "bore",
  });
  session.build({ plate }).dispose();
  point[0] = 999;
  direction[2] = 0;
  session.build({ plate }).dispose();
  assert.deepEqual(h.requests[0], h.requests[1]);
  const bore = h.requests[0].nodes.find((n: any) => n.kind === "throughBore");
  assert.deepEqual(bore.point, [0, 0, 10]);
  assert.deepEqual(bore.direction, [1, 0, 1]);
  assert.equal(bore.separation, 2);
  session.dispose();
});

test("profile corner selections snapshot positions and share one treated profile across native operations", () => {
  const points: [number, number][] = [[22, 5]],
    options = { tolerance: 0.001 },
    feature = { feature: "shoulder" };
  const base = profiles
    .polygon([
      [7, 0],
      [22, 0],
      [22, 5],
      [18, 5],
      [18, 30],
      [7, 30],
    ])
    .tagged("body");
  const selected = base.cornersAt(points, options),
    rounded = selected.fillet(1, feature);
  points[0][0] = 99;
  points.push([7, 0]);
  options.tolerance = 99;
  feature.feature = "other";
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const path = paths
    .start([0, 0, 0], { tangent: [0, 1, 0], sectionX: [1, 0, 0] })
    .lineBy([0, 20, 0]);
  const make = () => ({
    plate: extrude(rounded, { height: 5 }),
    turn: revolve(rounded),
    rail: sweep(rounded, path),
    loft: loft([
      { profile: rounded, placement: placement() },
      { profile: rounded, placement: placement({ translation: [0, 0, 20] }) },
    ]),
  });
  const first = session.build(make());
  first.dispose();
  const second = session.build(make());
  second.dispose();
  assert.deepEqual(h.requests[0], h.requests[1]);
  const treated = h.requests[0].profiles.filter(
    (p: any) => p.kind === "corners",
  );
  assert.equal(treated.length, 1);
  assert.deepEqual(treated[0].at, [[22, 5]]);
  assert.equal(treated[0].tolerance, 0.001);
  assert.equal(treated[0].feature, "shoulder");
  assert.throws(() => base.cornersAt([]), code("input.corners"));
  function* endless() {
    while (true) yield [22, 5] as const;
  }
  assert.throws(() => base.cornersAt(endless()), code("budget.corners"));
  session.dispose();
});

test("corner diagnostics retain immutable query and candidate evidence", () => {
  const point: [number, number] = [1, 2],
    candidate = { boundary: "outer" as const, segment: 3, point };
  const error = new ExedraError({
    code: "profile.corner.ambiguous",
    operation: "profile.corners",
    message: "Several corners match",
    cornerPoint: point,
    cornerCandidates: [candidate],
  });
  point[0] = 99;
  candidate.segment = 44;
  assert.deepEqual(error.diagnostic.cornerPoint, [1, 2]);
  assert.equal(error.diagnostic.cornerCandidates![0].segment, 3);
  assert.throws(() => {
    (error.diagnostic.cornerCandidates![0].point as any)[0] = 77;
  }, TypeError);
});

test("closed curve profiles snapshot center/direction/feature intent and lower one shared source", () => {
  const to: [number, number] = [3, 1],
    center: [number, number] = [3, 0];
  const arc = {
    kind: "arc" as const,
    to,
    center,
    clockwise: false,
    feature: "rim",
  };
  const segments = [
    { kind: "line" as const, to: [3, 0] as [number, number] },
    { kind: "line" as const, to: [4, 0] as [number, number] },
    arc,
  ];
  const profile = profiles.loop(segments),
    h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const first = session.build({
    plate: extrude(profile, { height: 2 }),
    turned: revolve(profile, { angle: 1.7 }),
  });
  first.dispose();
  to[1] = 99;
  center[0] = 99;
  arc.clockwise = true;
  arc.feature = "other";
  segments.pop();
  session
    .build({
      plate: extrude(profile, { height: 2 }),
      turned: revolve(profile, { angle: 1.7 }),
    })
    .dispose();
  assert.deepEqual(h.requests[0], h.requests[1]);
  assert.equal(h.requests[0].profiles.length, 1);
  assert.deepEqual(h.requests[0].profiles[0].segments[2], {
    kind: "arc",
    to: [3, 1],
    center: [3, 0],
    clockwise: false,
    feature: "rim",
  });
  function* endless() {
    while (true) yield { kind: "line" as const, to: [0, 0] as const };
  }
  assert.throws(() => profiles.loop(endless()), code("budget.profileSegments"));
  assert.throws(
    () =>
      profiles.loop([
        { kind: "arc", to: [1, 0], center: [Infinity, 0], clockwise: false },
      ]),
    code("input.nonFinite"),
  );
  assert.throws(
    () =>
      profiles.loop([
        { kind: "arc", to: [1, 0], center: [0, 0], clockwise: 1 as any },
      ]),
    code("input.arcDirection"),
  );
  session.dispose();
});

test("spherical cap intent preserves requested dimensions and placement for native validation", () => {
  const translation: [number, number, number] = [10, 20, 30],
    options = {
      radius: 25,
      height: 12,
      placement: placement({ translation, rotationXYZ: [Math.PI / 2, 0, 0] }),
    };
  const cap = sphericalCap(options),
    h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  session.build({ dome: cap }).dispose();
  options.radius = 99;
  options.height = 99;
  translation[2] = 999;
  session.build({ dome: cap }).dispose();
  assert.deepEqual(h.requests[0], h.requests[1]);
  assert.equal(h.requests[0].profiles.length, 0);
  assert.equal(h.requests[0].nodes[0].kind, "sphericalCap");
  assert.equal(h.requests[0].nodes[0].radius, 25);
  assert.equal(h.requests[0].nodes[0].height, 12);
  assert.deepEqual(h.requests[0].nodes[0].placement.translation, [10, 20, 30]);
  assert.throws(
    () => sphericalCap({ radius: Infinity, height: 12 }),
    code("input.nonFinite"),
  );
  session.dispose();
});

test("generic boundary authoring snapshots exact geometry and feature tables", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const frame: any = [
      [1, 0, 0, 0],
      [0, 1, 0, 0],
      [0, 0, 1, 0],
    ],
    vertices: any = [[0, 0, 0]],
    features = ["stock", "machined"];
  const description: any = {
    vertices,
    edges: [{ kind: "collapsed", vertex: 0 }],
    loops: [[{ edge: 0 }]],
    faces: [
      {
        surface: { kind: "plane", frame },
        u: [0, 2],
        v: [0, 3],
        sense: "forward",
        loops: [0],
        features,
      },
    ],
  };
  const intent = Solid.body(description);
  vertices[0][0] = 99;
  frame[0][3] = 88;
  features[0] = "mutated";
  session.build({ stock: intent });
  const wire = h.requests[0].nodes[0];
  assert.equal(wire.kind, "body");
  assert.equal(h.requests[0].profiles.length, 0);
  assert.equal(wire.body.vertices[0][0], 0);
  assert.equal(wire.body.faces[0].surface.frame[0][3], 0);
  assert.deepEqual(wire.body.faces[0].features, ["stock", "machined"]);
  assert.throws(
    () => Solid.body({ ...description, vertices: [[Infinity, 0, 0]] }),
    code("input.nonFinite"),
  );
  session.dispose();
});

test("solid set operations share operands and snapshot body-local halfspace intent", () => {
  const h = host(),
    session = new AuthoringSession(h.native, { units: "mm" });
  const left = extrude(
    profiles.rectangle({ width: 10, height: 10 }).tagged("stock"),
    { height: 10 },
  );
  const right = extrude(
    profiles.rectangle({ width: 4, height: 4 }).tagged("lug"),
    { height: 10, placement: { translation: [4, 0, 0] } },
  );
  const point: [number, number, number] = [0, 0, 5],
    normal: [number, number, number] = [0, 0, 1];
  const clipped = left
    .union(right)
    .intersectHalfSpace({ point, normal, feature: "cut" });
  point[2] = 99;
  normal[2] = -1;
  const doc = session.build({
    clipped,
    overlap: left.intersect(right),
    difference: left.subtract(right),
  });
  const graph = h.requests[0];
  assert.equal(graph.nodes.filter((n: any) => n.kind === "extrude").length, 2);
  assert.deepEqual(
    graph.nodes.find((n: any) => n.kind === "union"),
    { kind: "union", base: 0, other: 1 },
  );
  assert.deepEqual(
    graph.nodes.find((n: any) => n.kind === "intersectHalfSpace"),
    {
      kind: "intersectHalfSpace",
      base: 2,
      point: [0, 0, 5],
      normal: [0, 0, 1],
      feature: "cut",
    },
  );
  assert.equal(graph.nodes.find((n: any) => n.kind === "intersect").other, 1);
  assert.equal(graph.nodes.find((n: any) => n.kind === "subtract").base, 0);
  assert.throws(
    () =>
      left.intersectHalfSpace({ point: [0, 0, Infinity], normal: [0, 0, 1] }),
    code("input.nonFinite"),
  );
  doc.dispose();
  session.dispose();
});

test("native table budgets count nested references, tags, records and expanded coedge visits", () => {
  const base = { vertices: [], edges: [], loops: [], faces: [] } as const;
  assert.throws(
    () =>
      Solid.body({ ...base, loops: Array.from({ length: 8193 }, () => []) }),
    code("budget.nativeSegments"),
  );
  assert.throws(
    () =>
      Solid.body({
        ...base,
        trims: Array.from({ length: 8193 }, () => ({ loop: 0, curves: [] })),
      }),
    code("budget.nativeSegments"),
  );
  const frame = [
    [1, 0, 0, 0],
    [0, 1, 0, 0],
    [0, 0, 1, 0],
  ] as const;
  assert.throws(
    () =>
      Solid.body({
        ...base,
        faces: [
          {
            surface: { kind: "plane", frame },
            u: [0, 1],
            v: [0, 1],
            sense: "forward",
            loops: Array(8193).fill(0),
          },
        ],
      }),
    code("budget.nativeSegments"),
  );
  assert.throws(
    () =>
      Solid.body({
        ...base,
        faces: [
          {
            surface: { kind: "plane", frame },
            u: [0, 1],
            v: [0, 1],
            sense: "forward",
            loops: [],
            features: Array(8193).fill("tag"),
          },
        ],
      }),
    code("budget.nativeSegments"),
  );
  assert.throws(
    () =>
      Solid.body({
        ...base,
        loops: [Array.from({ length: 100 }, () => ({ edge: 0 }))],
        faces: [
          {
            surface: { kind: "plane", frame },
            u: [0, 1],
            v: [0, 1],
            sense: "forward",
            loops: Array(100).fill(0),
          },
        ],
      }),
    code("budget.nativeSegments"),
  );
});
