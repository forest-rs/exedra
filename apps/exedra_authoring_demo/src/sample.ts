// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { extrude, profiles } from "@exedra/authoring";

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
