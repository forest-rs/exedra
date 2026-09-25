// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Contact and clearance witnesses use the same flattened placements as GLB export.

use exedra_assembly::{RenderItem, flatten};

use super::*;
use crate::scene;

const PROBE: f64 = 0.00015;

fn point(placement: Placement3, p: [f64; 3]) -> [f64; 3] {
    placement.rows.map(|r| dot([r[0], r[1], r[2]], p) + r[3])
}

fn triangles(part: &CompiledPart) -> impl Iterator<Item = [[f64; 3]; 3]> + '_ {
    part.bodies.iter().flat_map(|body| {
        body.tri
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| [t[0], t[1], t[2]].map(|i| body.tri.positions[i as usize].map(f64::from)))
    })
}

#[test]
fn ridge_contacts_close_the_exported_roof_and_support_every_crown() -> Result<()> {
    for (bays, span_mm, depth_mm) in [(1, 2_400, 5_400), (1, 3_600, 3_600), (5, 5_400, 2_400)] {
        let layout = Layout::resolve(Parameters {
            bays,
            span_mm,
            depth_mm,
        })?;
        let assembly = scene::build(&layout)?;
        let compiled = PartCompiler::new().compile_parts(&assembly, &compile_policy())?;
        check_reports(&assembly, &compiled)?;
        let render = flatten(&assembly, &compiled);
        let named = |key: &str| -> &RenderItem {
            render
                .items
                .iter()
                .find(|item| assembly.instance(item.instance).unwrap().key() == key)
                .unwrap()
        };
        let solid = |item: &RenderItem| Solid::new(compiled.part(item.part).unwrap(), item.world);
        let section = RoofSection::new(&layout);
        let roof_width = layout.width + 1.2;
        let columns = interval_count(roof_width, 0.225);
        let pitch = roof_width / f64::from(columns);
        let column = columns / 2;
        let x0 = -roof_width * 0.5 + f64::from(column) * pitch;
        let z = section.layer(DECK_TOP)[4][1];
        let foot_item = named(&format!("ridge-foot-{column}"));
        let foot = solid(foot_item);
        let foot_part = compiled.part(foot_item.part).unwrap();
        let foot_samples = placed(&clay_samples(foot_part), foot_item.world);
        let mut lifted = foot_item.world;
        lifted.rows[2][3] += 0.003;
        let lifted = Solid::new(foot_part, lifted);
        for side in [-1.0, 1.0] {
            for (kind, lane, spacing, projection) in [
                ("cap", column, 0.245, CAP_EAVE_PROJECTION),
                ("cap", column + 1, 0.245, CAP_EAVE_PROJECTION),
                ("pan", column, 0.230, PAN_EAVE_PROJECTION),
            ] {
                let last = courses(&section, spacing, projection).len() - 1;
                let item = named(&format!("{kind}-{side}-{lane}-{last}"));
                let part = compiled.part(item.part).unwrap();
                let tile = solid(item);
                let mut contacts = 0;
                for triangle in triangles(part) {
                    let n = cross(sub(triangle[1], triangle[0]), sub(triangle[2], triangle[0]));
                    if n[2] / norm(n) < 0.15 {
                        continue;
                    }
                    for weights in [[0.6, 0.2, 0.2], [0.2, 0.6, 0.2], [0.2, 0.2, 0.6]] {
                        let p: [f64; 3] = core::array::from_fn(|i| {
                            (0..3).map(|j| triangle[j][i] * weights[j]).sum()
                        });
                        if !(0.240..0.330).contains(&p[1]) {
                            continue;
                        }
                        let world = point(item.world, p);
                        let across = world[0] - x0;
                        if !(0.002..pitch - 0.002).contains(&across)
                            || (kind == "pan"
                                && !(CAP_RADIUS + 0.002..pitch - CAP_RADIUS - 0.002)
                                    .contains(&across))
                        {
                            continue;
                        }
                        let below = point(item.world, [p[0], p[1], p[2] - PROBE]);
                        let above = point(item.world, [p[0], p[1], p[2] + PROBE]);
                        assert!(
                            tile.contains(below) && !foot.contains(below),
                            "ridge foot penetrates {kind}, {side}, {span_mm}/{depth_mm}"
                        );
                        assert!(
                            foot.contains(above) && !tile.contains(above),
                            "ridge foot floats above {kind}, {side}, {span_mm}/{depth_mm}"
                        );
                        assert!(
                            !lifted.contains(above),
                            "contact witness must reject a floating replacement"
                        );
                        contacts += 1;
                    }
                }
                assert!(contacts >= 3, "real {kind} contact coverage on side {side}");
                for p in placed(&clay_samples(part), item.world) {
                    assert!(
                        !foot.contains(p),
                        "roof clay intersects the ridge foot at {p:?}"
                    );
                }
                for &p in &foot_samples {
                    assert!(
                        !tile.contains(p),
                        "ridge foot intersects roof clay at {p:?}"
                    );
                }
            }
        }
        // The fillet's full rectangular underside fits within the continuous
        // horizontal bed. Include near-corner witnesses on both roof halves.
        let fillet = solid(named(&format!("ridge-fillet-{column}")));
        for across in [
            PROBE,
            pitch * 0.25,
            pitch * 0.5,
            pitch * 0.75,
            pitch - PROBE,
        ] {
            for y in [-0.180 + PROBE, -0.090, -PROBE, PROBE, 0.090, 0.180 - PROBE] {
                let below = [x0 + across, y, z + ridge::FOOT_TOP - PROBE];
                let above = [x0 + across, y, z + ridge::FOOT_TOP + PROBE];
                assert!(
                    foot.contains(below) && !fillet.contains(below),
                    "fillet lacks its ridge-foot bearing at {below:?}"
                );
                assert!(
                    fillet.contains(above) && !foot.contains(above),
                    "fillet is separated from its ridge-foot bed at {above:?}"
                );
            }
        }
        // Each cut cap is a complete rectangular half-bed, wider than the
        // fillet base. Area and boundary extents supplement the point witnesses.
        for body in &foot_part.bodies {
            let mut area = 0.0;
            let mut low = [f64::INFINITY; 2];
            let mut high = [f64::NEG_INFINITY; 2];
            for indices in body.tri.indices.as_chunks::<3>().0.iter() {
                let t = [indices[0], indices[1], indices[2]]
                    .map(|i| body.tri.positions[i as usize].map(f64::from));
                if t.iter().any(|p| (p[2] - ridge::FOOT_TOP).abs() > 1.0e-6) {
                    continue;
                }
                let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
                assert!(n[2] > 0.0);
                area += n[2] * 0.5;
                for p in t {
                    for axis in 0..2 {
                        low[axis] = low[axis].min(p[axis]);
                        high[axis] = high[axis].max(p[axis]);
                    }
                }
            }
            let rectangle = (high[0] - low[0]) * (high[1] - low[1]);
            assert!((area - rectangle).abs() < 1.0e-7);
            assert!((high[0] - low[0] - pitch).abs() < 1.0e-6);
            assert!(high[1] - low[1] > 0.180);
        }
        // Include every fillet/end extension and both end plates, rather than
        // inferring their support from the regular repeated unit.
        for item in render.items.iter().filter(|item| item.body == 0) {
            let key = assembly.instance(item.instance).unwrap().key();
            if !key.starts_with("ridge-fillet-") {
                continue;
            }
            let foot = solid(named(&key.replacen("ridge-fillet-", "ridge-foot-", 1)));
            let fillet = solid(item);
            for t in triangles(compiled.part(item.part).unwrap()) {
                if t.iter().any(|p| (p[2] - ridge::FOOT_TOP).abs() > 1.0e-6) {
                    continue;
                }
                for weights in [
                    [1.0 / 3.0; 3],
                    [0.9, 0.05, 0.05],
                    [0.05, 0.9, 0.05],
                    [0.05, 0.05, 0.9],
                ] {
                    let p = core::array::from_fn(|i| (0..3).map(|j| t[j][i] * weights[j]).sum());
                    let p = point(item.world, p);
                    assert!(
                        foot.contains([p[0], p[1], p[2] - PROBE]),
                        "unsupported {key} at {p:?}"
                    );
                    assert!(fillet.contains([p[0], p[1], p[2] + PROBE]));
                }
            }
        }
        let fillets: Vec<_> = render
            .items
            .iter()
            .filter(|item| {
                item.body == 0
                    && assembly
                        .part(item.part)
                        .unwrap()
                        .key()
                        .starts_with("ridge-fillet")
            })
            .map(solid)
            .collect();
        let crowns: Vec<_> = render
            .items
            .iter()
            .filter(|item| {
                item.body == 0 && assembly.part(item.part).unwrap().key() == "ridge-tile"
            })
            .collect();
        let mut bearing_faces = 0;
        for item in &crowns {
            let own = solid(item);
            for t in triangles(compiled.part(item.part).unwrap()) {
                let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
                if n[2] / norm(n) > -0.99 || t.iter().any(|p| p[2].abs() > 1.0e-6) {
                    continue;
                }
                bearing_faces += 1;
                for weights in [
                    [1.0 / 3.0; 3],
                    [0.9, 0.05, 0.05],
                    [0.05, 0.9, 0.05],
                    [0.05, 0.05, 0.9],
                ] {
                    let p = core::array::from_fn(|i| (0..3).map(|j| t[j][i] * weights[j]).sum());
                    let p = point(item.world, p);
                    let below = [p[0], p[1], p[2] - PROBE];
                    let above = [p[0], p[1], p[2] + PROBE];
                    assert!(
                        fillets.iter().any(|f| f.contains(below)),
                        "unsupported crown foot at {p:?}"
                    );
                    assert!(own.contains(above), "crown contact samples actual clay");
                    assert!(
                        !own.contains(below) && fillets.iter().all(|f| !f.contains(above)),
                        "crown and fillet intersect"
                    );
                }
            }
        }
        assert_eq!(
            bearing_faces,
            crowns.len() * 4,
            "both complete crown feet are checked"
        );
        for side in [-1, 1] {
            let item = named(&format!("ridge-end-{side}"));
            let own = solid(item);
            let mut bearing_faces = 0;
            for t in triangles(compiled.part(item.part).unwrap()) {
                let t = t.map(|p| point(item.world, p));
                if t.iter().any(|p| (p[2] - z - RIDGE_LIFT).abs() > 1.0e-6) {
                    continue;
                }
                bearing_faces += 1;
                for weights in [
                    [1.0 / 3.0; 3],
                    [0.9, 0.05, 0.05],
                    [0.05, 0.9, 0.05],
                    [0.05, 0.05, 0.9],
                ] {
                    let p: [f64; 3] =
                        core::array::from_fn(|i| (0..3).map(|j| t[j][i] * weights[j]).sum());
                    assert!(
                        fillets
                            .iter()
                            .any(|f| f.contains([p[0], p[1], p[2] - PROBE])),
                        "unsupported ridge end plate"
                    );
                    assert!(own.contains([p[0], p[1], p[2] + PROBE]));
                }
            }
            assert_eq!(bearing_faces, 2);
        }
    }
    Ok(())
}
