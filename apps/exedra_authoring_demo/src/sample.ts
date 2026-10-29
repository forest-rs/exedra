// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import {
  ExedraError,
  extrude,
  profiles,
  revolve,
  sphericalCap,
  sweep,
  loft,
  paths,
  placement,
  charts,
  uvTransform,
  type NativePart,
  type Profile,
  type Solid,
  type ConstructionChart,
} from "@exedra/authoring";

export interface PanelParameters {
  width: number;
  height: number;
  thickness: number;
  pocketDepth: number;
  cornerRadius: number;
  ventRadius: number;
}

/** This is the actual authoring function used by the live controls. */
export function authorPanel(parameters: PanelParameters) {
  // Reuse one tagged slot profile at four positions in the panel's XY outline.
  const vent = profiles
    .slot({ length: 22, radius: parameters.ventRadius })
    .tagged("vents");
  const outline = profiles
    .roundedRectangle({
      width: parameters.width,
      height: parameters.height,
      radius: parameters.cornerRadius,
    })
    .tagged("panel")
    .withCutoutsAt(vent, [
      [-36, 22],
      [-12, 22],
      [12, 22],
      [36, 22],
    ]);

  const recess = profiles.slot({ length: 48, radius: 8 }).tagged("recess");
  const panel = extrude(outline, { height: parameters.thickness }).pocket(
    recess,
    {
      end: "end",
      depth: parameters.pocketDepth,
      at: [0, -10],
    },
  );

  // The recess tag finds its floor; the pin is authored on that face's workplane.
  const pin = panel
    .floor("recess")
    .workplane()
    .translated([0, -10])
    .offset(0.5)
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
export function authorCollar(parameters: CollarParameters) {
  // Revolve sections use X for radius and Y for axial height. Rotate the result
  // so that the finished collar's axis follows world Z.
  const section = profiles
    .rectangle({ width: parameters.wall, height: parameters.collarHeight })
    .translated([
      parameters.radius + parameters.wall / 2,
      parameters.collarHeight / 2,
    ])
    .tagged("collar");

  return {
    collar: revolve(section, {
      angle: (parameters.angle * Math.PI) / 180,
      placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
    }),
  };
}

/** Same retained revolution, with circular or rounded sections and optional bore. */
export function authorRoundedTurn(
  parameters: CollarParameters,
  kind: "roundedRing" | "torus" | "hollowTorus",
) {
  let section: Profile;

  if (kind === "roundedRing") {
    section = profiles
      .roundedRectangle({
        width: parameters.wall,
        height: parameters.collarHeight,
        radius: parameters.rounding ?? 2,
      })
      .translated([
        parameters.radius + parameters.wall / 2,
        parameters.collarHeight / 2,
      ])
      .tagged("ring");
  } else {
    // A centered bore belongs to the circular section before it is moved away
    // from the revolution axis. Its tag identifies the resulting inner wall.
    const circularSection =
      kind === "hollowTorus"
        ? profiles
            .circle({ radius: parameters.wall })
            .tagged("outer")
            .withCutoutsAt(
              profiles
                .circle({ radius: parameters.innerRadius ?? 4 })
                .tagged("bore"),
              [[0, 0]],
            )
        : profiles.circle({ radius: parameters.wall }).tagged("seal");

    section = circularSection.translated([parameters.radius, parameters.wall]);
  }

  return {
    ring: revolve(section, {
      angle: (parameters.angle * Math.PI) / 180,
      placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
    }),
  };
}

export type ShoulderFinish =
  | { readonly kind: "fillet"; readonly radius: number }
  | { readonly kind: "chamfer"; readonly distance: number };

export interface FlangeParameters {
  boreRadius: number;
  sleeveOuterRadius: number;
  flangeOuterRadius: number;
  shoulderHeight: number;
  shoulder?: ShoulderFinish;
  height: number;
  angle: number;
}

/** Independent radial and axial dimensions; the native profile validates material geometry. */
export function authorFlangedSleeve(parameters: FlangeParameters) {
  if (!(
    parameters.shoulderHeight > 0 &&
    parameters.shoulderHeight < parameters.height
  )) {
    throw new ExedraError({
      code: "input.flangeHeight",
      operation: "sample.flangedSleeve",
      message: "Shoulder height must be positive and less than total height.",
    });
  }

  const boreRadius = parameters.boreRadius;
  const sleeveRadius = parameters.sleeveOuterRadius;
  const flangeRadius = parameters.flangeOuterRadius;
  const shoulderHeight = parameters.shoulderHeight;
  const totalHeight = parameters.height;

  // Trace the section in radius/height coordinates. Equal outer radii make a
  // plain sleeve, without a redundant shoulder vertex.
  let points: readonly (readonly [number, number])[];
  if (sleeveRadius === flangeRadius) {
    points = [
      [boreRadius, 0],
      [sleeveRadius, 0],
      [sleeveRadius, totalHeight],
      [boreRadius, totalHeight],
    ];
  } else {
    points = [
      [boreRadius, 0],
      [flangeRadius, 0],
      [flangeRadius, shoulderHeight],
      [sleeveRadius, shoulderHeight],
      [sleeveRadius, totalHeight],
      [boreRadius, totalHeight],
    ];
  }

  let section = profiles.polygon(points).tagged("sleeve");
  if (parameters.shoulder) {
    const shoulderCorner = section.cornerAt([sleeveRadius, shoulderHeight]);
    if (
      parameters.shoulder.kind === "fillet" &&
      parameters.shoulder.radius !== 0
    ) {
      section = shoulderCorner.fillet(parameters.shoulder.radius, {
        feature: "shoulder",
      });
    }
    if (
      parameters.shoulder.kind === "chamfer" &&
      parameters.shoulder.distance !== 0
    ) {
      section = shoulderCorner.chamfer(parameters.shoulder.distance, {
        feature: "shoulder",
      });
    }
  }

  return {
    sleeve: revolve(section, {
      angle: (parameters.angle * Math.PI) / 180,
      placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
    }),
  };
}

export interface SphereParameters {
  radius: number;
}

/** A center-defined half disk revolves to one spherical face with two exact poles. */
export function authorSphere(parameters: SphereParameters) {
  const radius = parameters.radius;
  // The arc runs between the two poles; its center fixes an exact semicircle.
  const section = profiles.loop([
    { kind: "line", to: [0, 0] },
    {
      kind: "arc",
      to: [0, 2 * radius],
      center: [0, radius],
      clockwise: false,
      feature: "shell",
    },
  ]);

  return {
    sphere: revolve(section, {
      placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
    }),
  };
}

export interface DomeParameters {
  radius: number;
  height: number;
}

/** Rust derives the canonical center-defined section, preserving the requested cap dimensions. */
export function authorDome(parameters: DomeParameters) {
  return {
    dome: sphericalCap({
      ...parameters,
      placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
    }),
  };
}

export interface PointedConeParameters {
  radius: number;
  height: number;
}

/** An ordinary triangular section creates an exact cone, planar base and welded apex. */
export function authorPointedCone(parameters: PointedConeParameters) {
  const section = profiles.loop([
    { kind: "line", to: [parameters.radius, 0], feature: "base" },
    { kind: "line", to: [0, parameters.height], feature: "shell" },
    { kind: "line", to: [0, 0] },
  ]);

  return {
    cone: revolve(section, {
      placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
    }),
  };
}

/** The same authored section as the Rust turned-fitting study. */
export function authorTurnedFitting(parameters: CollarParameters) {
  const outline: readonly (readonly [number, number])[] = [
    [22, 0],
    [22, 5],
    [18, 5],
    [18, 10],
    [20, 10],
    [20, 13],
    [18, 13],
    [18, 30],
    [14, 38],
    [14, 44],
    [11, 47],
    [7, 47],
    [7, 26],
    [10, 19],
    [10, 8],
    [7, 3],
    [7, 0],
  ];

  // Scale the study's radius/height section to the live bore, wall and height.
  const section = profiles
    .polygon(
      outline.map(
        ([radius, height]) =>
          [
            parameters.radius + ((radius - 7) * parameters.wall) / 15,
            (height * parameters.collarHeight) / 47,
          ] as const,
      ),
    )
    .tagged("fitting");

  return {
    fitting: revolve(section, {
      angle: (parameters.angle * Math.PI) / 180,
      placement: placement({ rotationXYZ: [Math.PI / 2, 0, 0] }),
    }),
  };
}

export interface SweepParameters {
  radius: number;
  bendRadius: number;
  angle: number;
  hollow: boolean;
}

/** Smooth exact line/arc rail, using the same path vocabulary as the Rust study. */
export function authorRail(parameters: SweepParameters) {
  const outer = profiles.circle({ radius: parameters.radius }).tagged("rail");
  let section = outer;
  if (parameters.hollow) {
    section = outer.withCutoutsAt(
      profiles.circle({ radius: parameters.radius * 0.6 }).tagged("bore"),
      [[0, 0]],
    );
  }

  // Start along world Y with the section's X axis along world X; the arc bends
  // in the XY plane while preserving a tangent join to the straight segment.
  const path = paths
    .start([0, 0, 0], { tangent: [0, 1, 0], sectionX: [1, 0, 0] })
    .lineBy([0, 20, 0])
    .tangentArc({
      radius: parameters.bendRadius,
      angle: (parameters.angle * Math.PI) / 180,
      normal: [0, 0, 1],
    });

  return {
    rail: sweep(section, path, {
      placement: placement({ translation: [0, 0, parameters.radius] }),
    }),
  };
}

export interface LoftParameters {
  height: number;
  twist: number;
  offset: number;
}

/** Four corresponding rounded sections; wall supports are native ruled surfaces. */
export function authorVariableLoft(parameters: LoftParameters) {
  const sizes = [
    [52, 38, 8],
    [80, 52, 12],
    [60, 40, 6],
    [68, 44, 10],
  ] as const;
  const angles = [
    0,
    parameters.twist,
    -parameters.twist * 0.6,
    parameters.twist * 0.8,
  ];
  const offsets = [
    [0, 0],
    [parameters.offset, 0],
    [-parameters.offset * 0.67, parameters.offset * 0.44],
    [0, 0],
  ] as const;

  // Keep section order consistent so corresponding edges connect between
  // stations. Each section lies in XY at its own height along world Z.
  return {
    loft: loft(
      sizes.map(([width, height, radius], sectionIndex) => ({
        profile: profiles
          .roundedRectangle({ width, height, radius })
          .tagged(`section ${sectionIndex + 1}`),
        placement: placement({
          translation: [
            offsets[sectionIndex][0],
            offsets[sectionIndex][1],
            (parameters.height * sectionIndex) / 3,
          ],
          rotationXYZ: [0, 0, (angles[sectionIndex] * Math.PI) / 180],
        }),
      })),
    ),
  };
}

export interface FinishParameters {
  size: number;
  height: number;
  treatment: "fillet" | "chamfer";
  inner: boolean;
}

/** Named profile axial edges only: outer rounds or inner chamfers, with native transition tags. */
export function authorFinishedPlate(parameters: FinishParameters) {
  const outer = profiles.rectangle({ width: 70, height: 50 }).tagged("body");
  let outline = outer;
  if (parameters.inner) {
    outline = outer.withCutoutsAt(
      profiles.rectangle({ width: 32, height: 22 }).tagged("bore"),
      [[0, 0]],
    );
  }

  const original = extrude(outline, { height: parameters.height });
  // Select only edges parallel to extrusion, using the source profile's tag.
  const edges = original.axialEdges(parameters.inner ? "bore" : "body");
  const finished =
    parameters.treatment === "fillet"
      ? edges.fillet({ radius: parameters.size, feature: "round" })
      : edges.chamfer({ distance: parameters.size, feature: "chamfer" });

  return { plate: finished };
}

export interface ChartParameters {
  repeat: number;
  referenceRadius: number;
  restLength: number;
  referenceSection: number;
  stations: "sectionIndex" | "datumDistance";
}

/** Actual retained native capability chooses the chart; no projected fallback is painted. */
export function constructionChart(
  part: NativePart,
  parameters: ChartParameters,
): ConstructionChart | undefined {
  if (part.chartKind === null) return undefined;

  const grid = uvTransform({
    matrix: [
      [1 / parameters.repeat, 0],
      [0, 1 / parameters.repeat],
    ],
  });

  switch (part.chartKind) {
    case "extrude":
      return charts.extrude({ wall: grid, caps: grid });
    case "revolve":
      return charts.revolve({
        referenceRadius: parameters.referenceRadius,
        wall: grid,
        caps: grid,
      });
    case "sweep":
      return charts.sweep({ wall: grid, caps: grid });
    case "loft":
      return charts.loft({
        referenceSection: parameters.referenceSection,
        restLength: parameters.restLength,
        stations: parameters.stations,
        wall: grid,
        caps: grid,
      });
  }
}

export interface BoreParameters {
  width: number;
  height: number;
  thickness: number;
  radius: number;
  angle: number;
  separation: number;
}

/** Exact cylinder/plate intersection: the complete rims are native ellipses. */
export function authorAngledBore(parameters: BoreParameters) {
  const plate = extrude(
    profiles
      .rectangle({ width: parameters.width, height: parameters.height })
      .tagged("plate"),
    { height: parameters.thickness },
  );
  const radians = (parameters.angle * Math.PI) / 180;

  // The bore passes through the plate's midplane and tilts toward world X.
  return {
    plate: plate.throughBore({
      point: [0, 0, parameters.thickness / 2],
      direction: [Math.sin(radians), 0, Math.cos(radians)],
      radius: parameters.radius,
      separation: parameters.separation,
      feature: "bore",
    }),
  };
}

export interface HousingParameters {
  width: number;
  bodyWidth: number;
  height: number;
  flangeDepth: number;
  thickness: number;
  passageRadius: number;
  passageY: number;
  passageAngle: number;
  mountSpacing: number;
  mountRadius: number;
  portY: number;
  portRadius: number;
  clearance: number;
}

/** Ordinary polygon stock and four independent exact cylinder cuts.
 * Every cut edits retained boundary geometry; features select generated faces.
 */
export function authorHousing(parameters: HousingParameters) {
  const halfWidth = parameters.width / 2;
  const bodyHalfWidth = parameters.bodyWidth / 2;
  const halfHeight = parameters.height / 2;
  const shoulderY = -halfHeight + parameters.flangeDepth;

  // Walk around the flange and narrower body in XY, then extrude along Z.
  const stock = extrude(
    profiles
      .polygon([
        [-halfWidth, -halfHeight],
        [halfWidth, -halfHeight],
        [halfWidth, shoulderY],
        [bodyHalfWidth, shoulderY],
        [bodyHalfWidth, halfHeight],
        [-bodyHalfWidth, halfHeight],
        [-bodyHalfWidth, shoulderY],
        [-halfWidth, shoulderY],
      ])
      .tagged("stock"),
    { height: parameters.thickness },
  );

  return machineHousing(stock, parameters);
}

/** Two actual native box unions attach mounting lugs before the same machining sequence. */
export function authorFusedHousing(parameters: HousingParameters) {
  const body = extrude(
    profiles
      .rectangle({ width: parameters.bodyWidth, height: parameters.height })
      .tagged("stock"),
    { height: parameters.thickness },
  );
  const lugWidth = (parameters.width - parameters.bodyWidth) / 2;
  const lugY = -parameters.height / 2 + parameters.flangeDepth / 2;
  const lugX = (parameters.width + parameters.bodyWidth) / 4;

  const leftLug = extrude(
    profiles
      .rectangle({ width: lugWidth, height: parameters.flangeDepth })
      .translated([-lugX, lugY])
      .tagged("left lug"),
    { height: parameters.thickness },
  );
  const rightLug = extrude(
    profiles
      .rectangle({ width: lugWidth, height: parameters.flangeDepth })
      .translated([lugX, lugY])
      .tagged("right lug"),
    { height: parameters.thickness },
  );

  // Fuse the lugs in order, then apply the same four cuts as polygon stock.
  return machineHousing(body.union(leftLug).union(rightLug), parameters);
}

function machineHousing(stock: Solid, parameters: HousingParameters) {
  const halfHeight = parameters.height / 2;
  const mountY = -halfHeight + parameters.flangeDepth / 2;

  // Each cut consumes the previous solid. Feature names identify the new walls
  // and rims, so both housing constructions expose the same machined features.
  const withLeftMount = stock.subtractThroughCylinder({
    point: [-parameters.mountSpacing / 2, mountY, parameters.thickness / 2],
    direction: [0, 0, 1],
    radius: parameters.mountRadius,
    separation: parameters.clearance,
    feature: "left mount",
  });
  const withBothMounts = withLeftMount.subtractThroughCylinder({
    point: [parameters.mountSpacing / 2, mountY, parameters.thickness / 2],
    direction: [0, 0, 1],
    radius: parameters.mountRadius,
    separation: parameters.clearance,
    feature: "right mount",
  });
  const passageAngle = (parameters.passageAngle * Math.PI) / 180;
  const withPassage = withBothMounts.subtractThroughCylinder({
    point: [0, parameters.passageY, parameters.thickness / 2],
    direction: [Math.sin(passageAngle), 0, Math.cos(passageAngle)],
    radius: parameters.passageRadius,
    separation: parameters.clearance,
    feature: "main passage",
  });
  const housing = withPassage.subtractThroughCylinder({
    point: [0, parameters.portY, parameters.thickness / 2],
    direction: [0, 0, 1],
    radius: parameters.portRadius,
    separation: parameters.clearance,
    feature: "service port",
  });

  return { housing };
}
