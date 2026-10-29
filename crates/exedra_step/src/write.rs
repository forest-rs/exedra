// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    LengthUnit, SCHEMA, StepCorrespondence, StepDocument, StepEdgeEntity, StepEntityId, StepError,
    StepOptions, StepReport, preflight::Prepared,
};
use alloc::{string::String, vec, vec::Vec};
use core::fmt::{self, Write};
use exedra_analytic::{
    body::{Body, Sense},
    geometry::{Curve, Surface},
};
use exedra_math::{Placement3, norm};

// Empirical reader reconstruction mitigation, not a validity/accuracy certificate
// for a reader. The fixed cap bounds serialization work for every input scale.
const ELLIPSE_BASE_FRAGMENTS: u32 = 8;
const ELLIPSE_MAX_FRAGMENTS: u32 = 128;
const ELLIPSE_REFERENCE_MM: f64 = 2.0;

struct Buffer {
    text: String,
    next: u64,
    entities: u32,
    bytes: usize,
    required: usize,
}
impl Write for Buffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let required = self.text.len().saturating_add(s.len());
        if required > self.bytes {
            self.required = required;
            return Err(fmt::Error);
        }
        self.text.push_str(s);
        Ok(())
    }
}
impl Buffer {
    fn append(&mut self, args: fmt::Arguments<'_>) -> Result<(), StepError> {
        self.write_fmt(args).map_err(|_| StepError::Budget {
            resource: "bytes",
            required: self.required,
            maximum: self.bytes,
        })
    }
    fn entity(&mut self, args: fmt::Arguments<'_>) -> Result<StepEntityId, StepError> {
        if self.next > u64::from(self.entities) {
            return Err(StepError::Budget {
                resource: "entities",
                required: usize::try_from(self.next).unwrap_or(usize::MAX),
                maximum: self.entities as usize,
            });
        }
        let id = StepEntityId(u32::try_from(self.next).expect("entity budget"));
        self.next += 1;
        self.append(format_args!("{}={};\n", Id(id), args))?;
        Ok(id)
    }
    fn point(&mut self, p: [f64; 3]) -> Result<StepEntityId, StepError> {
        self.entity(format_args!(
            "CARTESIAN_POINT('',({},{},{}))",
            Real(p[0]),
            Real(p[1]),
            Real(p[2])
        ))
    }
    fn direction(&mut self, p: [f64; 3]) -> Result<StepEntityId, StepError> {
        self.entity(format_args!(
            "DIRECTION('',({},{},{}))",
            Real(p[0]),
            Real(p[1]),
            Real(p[2])
        ))
    }
    fn oriented_edge(
        &mut self,
        curve: StepEntityId,
        forward: bool,
    ) -> Result<StepEntityId, StepError> {
        self.entity(format_args!(
            "ORIENTED_EDGE('',*,*,{},{})",
            Id(curve),
            boolean(forward)
        ))
    }
    fn frame(&mut self, frame: Placement3) -> Result<StepEntityId, StepError> {
        let origin = self.point(frame.rows.map(|r| r[3]))?;
        let axis = self.direction(frame.rows.map(|r| r[2]))?;
        let reference = self.direction(frame.rows.map(|r| r[0]))?;
        self.entity(format_args!(
            "AXIS2_PLACEMENT_3D('',{},{},{})",
            Id(origin),
            Id(axis),
            Id(reference)
        ))
    }
}
struct Id(StepEntityId);
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0.0)
    }
}
struct Refs<'a>(&'a [StepEntityId]);
impl fmt::Display for Refs<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        for (i, &id) in self.0.iter().enumerate() {
            if i != 0 {
                f.write_str(",")?;
            }
            write!(f, "{}", Id(id))?;
        }
        f.write_str(")")
    }
}
struct Real(f64);
impl fmt::Display for Real {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Always a STEP REAL, including a decimal point and uppercase exponent.
        // Eighteen significant decimal digits round-trip finite binary64 values.
        write!(f, "{:.17E}", self.0)
    }
}
struct Quoted<'a>(&'a str);
impl fmt::Display for Quoted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("'")?;
        for c in self.0.chars() {
            match c {
                '\'' => f.write_str("''")?,
                '\\' => f.write_str("\\\\")?,
                ' '..='~' => write!(f, "{c}")?,
                _ if u32::from(c) <= 0xffff => write!(f, "\\X2\\{:04X}\\X0\\", u32::from(c))?,
                _ => write!(f, "\\X4\\{:08X}\\X0\\", u32::from(c))?,
            }
        }
        f.write_str("'")
    }
}
fn boolean(b: bool) -> &'static str {
    if b { ".T." } else { ".F." }
}

pub(super) fn export<F, E>(
    body: &Body<F, E>,
    placement: Placement3,
    options: &StepOptions<'_>,
    mut prepared: Prepared,
) -> Result<StepDocument, StepError> {
    let estimate =
        (body.vertices().len() + body.edges().len() + body.faces().len()).saturating_mul(256);
    let mut out = Buffer {
        text: String::with_capacity(estimate.min(options.max_bytes)),
        next: 1,
        entities: options.max_entities,
        bytes: options.max_bytes,
        required: 0,
    };
    out.append(format_args!("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('Exedra retained analytic boundary'),'2;1');\nFILE_NAME({},{},(''),(''),'Exedra','Exedra','');\nFILE_SCHEMA(('{}'));\nENDSEC;\nDATA;\n",Quoted(options.name),Quoted(options.timestamp),SCHEMA))?;
    let mut map = StepCorrespondence {
        vertices: vec![None; body.vertices().len()],
        edges: vec![None; body.edges().len()],
        loops: vec![None; body.loops().len()],
        faces: vec![None; body.faces().len()],
    };
    for (i, (&active, &p)) in prepared.vertices.iter().zip(body.vertices()).enumerate() {
        if !active {
            continue;
        }
        let point = out.point(placement.transform_point(p))?;
        map.vertices[i] = Some(out.entity(format_args!("VERTEX_POINT('',{})", Id(point)))?);
    }
    for (i, (&active, edge)) in prepared.edges.iter().zip(body.edges()).enumerate() {
        if !active {
            continue;
        }
        let Some((geometry, range)) = edge.regular() else {
            map.edges[i] = Some(StepEdgeEntity::CollapsedVertex(
                map.vertices[edge.start.index()].expect("used"),
            ));
            continue;
        };
        let curve = match geometry {
            Curve::Line(line) => {
                let origin = out.point(placement.transform_point(line.origin()))?;
                let direction = placement.transform_vector(line.direction());
                let magnitude = norm(direction);
                if !magnitude.is_finite() || magnitude == 0.0 {
                    return Err(StepError::Numeric);
                }
                let direction = out.direction(direction)?;
                let vector = out.entity(format_args!(
                    "VECTOR('',{},{})",
                    Id(direction),
                    Real(magnitude)
                ))?;
                out.entity(format_args!("LINE('',{},{})", Id(origin), Id(vector)))?
            }
            Curve::Circle(circle) => {
                let frame = out.frame(placement * circle.frame())?;
                out.entity(format_args!(
                    "CIRCLE('',{},{})",
                    Id(frame),
                    Real(circle.radius())
                ))?
            }
            Curve::Ellipse(ellipse) => {
                let frame = out.frame(placement * ellipse.frame())?;
                if ellipse.semi_major() == ellipse.semi_minor() {
                    // Exact identity, not tolerance-based snapping: an equal-axis
                    // native ellipse is the same circle with the same parameters.
                    out.entity(format_args!(
                        "CIRCLE('',{},{})",
                        Id(frame),
                        Real(ellipse.semi_major())
                    ))?
                } else {
                    out.entity(format_args!(
                        "ELLIPSE('',{},{},{})",
                        Id(frame),
                        Real(ellipse.semi_major()),
                        Real(ellipse.semi_minor())
                    ))?
                }
            }
        };
        // Some readers reconstruct a closed ellipse's surface parameter curve
        // outside the declared tolerance when its seam is rephased. Short exact
        // elliptic arcs bound that reconstruction work without changing any
        // supporting geometry or duplicating an edge between its two faces.
        let count = match geometry {
            Curve::Ellipse(ellipse) if ellipse.semi_major() != ellipse.semi_minor() => {
                // OCCT reconstructs trims with a fixed physical confusion floor
                // in millimetres, even for metre-valued STEP coordinates. Scale
                // this empirically verified reconstruction guard with physical
                // axial excursion and aspect ratio; it is not an interval certificate
                // for a reader.
                // Fourth-power growth matches the observed cubic interpolation
                // sensitivity while powers of two retain a simple bounded policy.
                let axial_excursion =
                    exedra_math::Real::sqrt(ellipse.semi_major() - ellipse.semi_minor())
                        * exedra_math::Real::sqrt(ellipse.semi_major() + ellipse.semi_minor());
                let axial_excursion_mm = axial_excursion * options.units.metres_per_unit() * 1000.0;
                let aspect_ratio = ellipse.semi_major() / ellipse.semi_minor();
                let trim_scale_mm = axial_excursion_mm * aspect_ratio;
                if !trim_scale_mm.is_finite() {
                    return Err(StepError::Numeric);
                }
                let mut full = ELLIPSE_BASE_FRAGMENTS;
                while full < ELLIPSE_MAX_FRAGMENTS {
                    let ratio = f64::from(full) / f64::from(ELLIPSE_BASE_FRAGMENTS);
                    if trim_scale_mm <= ELLIPSE_REFERENCE_MM * ratio * ratio * ratio * ratio {
                        break;
                    }
                    full *= 2;
                }
                let step = core::f64::consts::TAU / f64::from(full);
                (1..=full)
                    .find(|n| range.span() <= f64::from(*n) * step * (1.0 + 64.0 * f64::EPSILON))
                    .unwrap_or(full)
            }
            _ => 1,
        };
        let mut vertices = [StepEntityId(0); ELLIPSE_MAX_FRAGMENTS as usize + 1];
        vertices[0] = map.vertices[edge.start.index()].expect("used");
        for fragment in 1..count {
            let fraction = f64::from(fragment) / f64::from(count);
            let p = geometry
                .evaluate(range.at(fraction).ok_or(StepError::Numeric)?)
                .ok_or(StepError::Numeric)?
                .point;
            let p = placement.transform_point(p);
            if !p.into_iter().all(f64::is_finite) {
                return Err(StepError::Numeric);
            }
            let point = out.point(p)?;
            vertices[fragment as usize] =
                out.entity(format_args!("VERTEX_POINT('',{})", Id(point)))?;
        }
        vertices[count as usize] = map.vertices[edge.end.index()].expect("used");
        let mut first = None;
        for pair in vertices[..=count as usize].windows(2) {
            let id = out.entity(format_args!(
                "EDGE_CURVE('',{},{},{},.T.)",
                Id(pair[0]),
                Id(pair[1]),
                Id(curve)
            ))?;
            first.get_or_insert(id);
        }
        let first = first.expect("positive fragment count");
        map.edges[i] = Some(if count == 1 {
            StepEdgeEntity::Curve(first)
        } else {
            StepEdgeEntity::CurveChain { first, count }
        });
        prepared.counts.vertices = prepared
            .counts
            .vertices
            .checked_add(count - 1)
            .ok_or(StepError::Numeric)?;
        prepared.counts.edges = prepared
            .counts
            .edges
            .checked_add(count - 1)
            .ok_or(StepError::Numeric)?;
    }
    let mut refs = Vec::new();
    for (i, (&active, ring)) in prepared.loops.iter().zip(body.loops()).enumerate() {
        if !active {
            continue;
        }
        refs.clear();
        refs.reserve(ring.coedges.len());
        for c in &ring.coedges {
            let entity = map.edges[c.edge.index()].expect("used");
            let curves = entity.curve_entities();
            if c.reversed {
                for curve in curves.rev() {
                    refs.push(out.oriented_edge(curve, false)?);
                }
            } else {
                for curve in curves {
                    refs.push(out.oriented_edge(curve, true)?);
                }
            }
            if let StepEdgeEntity::CurveChain { count, .. } = entity {
                prepared.counts.coedges = prepared
                    .counts
                    .coedges
                    .checked_add(count - 1)
                    .ok_or(StepError::Numeric)?;
            }
        }
        map.loops[i] = Some(if refs.is_empty() {
            let edge = &body.edges()[ring.coedges[0].edge.index()];
            out.entity(format_args!(
                "VERTEX_LOOP('',{})",
                Id(map.vertices[edge.start.index()].expect("used"))
            ))?
        } else {
            out.entity(format_args!("EDGE_LOOP('',{})", Refs(&refs)))?
        });
    }
    for (i, face) in body.faces().iter().enumerate() {
        let native_frame = face.surface.frame();
        // Canonical positive cone angle also works with readers that clamp a
        // negative STEP angle. Flipping Y and Z maps (u,v) to (-u,-v): two
        // parameter reversals preserve the outward normal and native face sense.
        let surface_frame = if matches!(&face.surface, Surface::Cone(c) if c.slope() < 0.0) {
            Placement3::from_axes(
                native_frame.rows.map(|r| r[0]),
                native_frame.rows.map(|r| -r[1]),
                native_frame.rows.map(|r| -r[2]),
                native_frame.rows.map(|r| r[3]),
            )
        } else {
            native_frame
        };
        let frame = out.frame(placement * surface_frame)?;
        let surface = match &face.surface {
            Surface::Plane(_) => out.entity(format_args!("PLANE('',{})", Id(frame)))?,
            Surface::Cylinder(c) => out.entity(format_args!(
                "CYLINDRICAL_SURFACE('',{},{})",
                Id(frame),
                Real(c.radius())
            ))?,
            Surface::Cone(c) if c.slope() == 0.0 => out.entity(format_args!(
                "CYLINDRICAL_SURFACE('',{},{})",
                Id(frame),
                Real(c.radius())
            ))?,
            Surface::Cone(c) => out.entity(format_args!(
                "CONICAL_SURFACE('',{},{},{})",
                Id(frame),
                Real(c.radius()),
                Real(c.semi_angle().abs())
            ))?,
            Surface::Sphere(s) => out.entity(format_args!(
                "SPHERICAL_SURFACE('',{},{})",
                Id(frame),
                Real(s.radius())
            ))?,
            Surface::Torus(t) => out.entity(format_args!(
                "TOROIDAL_SURFACE('',{},{},{})",
                Id(frame),
                Real(t.major_radius()),
                Real(t.minor_radius())
            ))?,
            _ => unreachable!("preflight checked surface family"),
        };
        refs.clear();
        refs.reserve(face.loops.len());
        for (j, &loop_id) in face.loops.iter().enumerate() {
            let kind = if j == 0
                && !matches!(face.surface, Surface::Sphere(_))
                && body.loops()[loop_id.index()]
                    .coedges
                    .iter()
                    .any(|c| !body.edges()[c.edge.index()].is_collapsed())
            {
                "FACE_OUTER_BOUND"
            } else {
                "FACE_BOUND"
            };
            refs.push(out.entity(format_args!(
                "{}('',{},.T.)",
                kind,
                Id(map.loops[loop_id.index()].expect("used"))
            ))?);
        }
        map.faces[i] = Some(out.entity(format_args!(
            "ADVANCED_FACE('',{},{},{})",
            Refs(&refs),
            Id(surface),
            boolean(face.sense == Sense::Forward)
        ))?);
    }
    refs.clear();
    refs.extend(map.faces.iter().map(|&id| id.expect("all faces used")));
    let shell = out.entity(format_args!("CLOSED_SHELL('',{})", Refs(&refs)))?;
    let solid = out.entity(format_args!(
        "MANIFOLD_SOLID_BREP({},{})",
        Quoted(options.name),
        Id(shell)
    ))?;
    let prefix = match options.units {
        LengthUnit::Millimetre => ".MILLI.",
        LengthUnit::Metre => "$",
    };
    let length = out.entity(format_args!(
        "(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT({},.METRE.))",
        prefix
    ))?;
    let angle = out.entity(format_args!(
        "(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.))"
    ))?;
    let solid_angle = out.entity(format_args!(
        "(NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT())"
    ))?;
    let uncertainty=out.entity(format_args!("UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE({}),{},'distance_accuracy_value','native support and export allowance')",Real(options.tolerance),Id(length)))?;
    let context=out.entity(format_args!("(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT(({})) GLOBAL_UNIT_ASSIGNED_CONTEXT(({},{},{})) REPRESENTATION_CONTEXT('','3D'))",Id(uncertainty),Id(length),Id(angle),Id(solid_angle)))?;
    let representation = out.entity(format_args!(
        "ADVANCED_BREP_SHAPE_REPRESENTATION({},({}),{})",
        Quoted(options.name),
        Id(solid),
        Id(context)
    ))?;
    let application = out.entity(format_args!("APPLICATION_CONTEXT('automotive design')"))?;
    out.entity(format_args!(
        "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,{})",
        Id(application)
    ))?;
    let product_context = out.entity(format_args!(
        "PRODUCT_CONTEXT('',{},'mechanical')",
        Id(application)
    ))?;
    let product = out.entity(format_args!(
        "PRODUCT({},{},'',({}))",
        Quoted(options.name),
        Quoted(options.name),
        Id(product_context)
    ))?;
    let formation = out.entity(format_args!(
        "PRODUCT_DEFINITION_FORMATION('','',{})",
        Id(product)
    ))?;
    let definition_context = out.entity(format_args!(
        "PRODUCT_DEFINITION_CONTEXT('part definition',{},'design')",
        Id(application)
    ))?;
    let definition = out.entity(format_args!(
        "PRODUCT_DEFINITION('design','',{},{})",
        Id(formation),
        Id(definition_context)
    ))?;
    let shape = out.entity(format_args!(
        "PRODUCT_DEFINITION_SHAPE('','',{})",
        Id(definition)
    ))?;
    out.entity(format_args!(
        "SHAPE_DEFINITION_REPRESENTATION({},{})",
        Id(shape),
        Id(representation)
    ))?;
    out.append(format_args!("ENDSEC;\nEND-ISO-10303-21;\n"))?;
    prepared.counts.entities = u32::try_from(out.next - 1).expect("entity budget");
    let report = StepReport {
        schema: SCHEMA,
        units: options.units,
        declared_uncertainty: options.tolerance,
        max_native_support_gap: prepared.gap,
        frame_roundoff_bound: prepared.allowance,
        parameter_roundoff_bound: prepared.parameter_allowance,
        max_deviation: prepared.gap + prepared.allowance + prepared.parameter_allowance,
        counts: prepared.counts,
        correspondence: map,
    };
    Ok(StepDocument {
        text: out.text,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    #[test]
    fn maximum_entity_id_is_emitted_without_counter_overflow() {
        let mut buffer = Buffer {
            text: String::new(),
            next: u64::from(u32::MAX),
            entities: u32::MAX,
            bytes: 256,
            required: 0,
        };
        assert_eq!(
            buffer.entity(format_args!("DUMMY()")).unwrap(),
            StepEntityId(u32::MAX)
        );
        assert!(matches!(
            buffer.entity(format_args!("DUMMY()")),
            Err(StepError::Budget {
                resource: "entities",
                ..
            })
        ));
    }
    #[test]
    fn step_reals_round_trip_binary64_and_always_have_real_lexical_form() {
        for value in [
            0.0,
            -0.0,
            0.1,
            1.0,
            -1e-100,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
        ] {
            let s = format!("{}", Real(value));
            assert!(s.contains('.') && s.contains('E'));
            assert_eq!(s.parse::<f64>().unwrap().to_bits(), value.to_bits());
        }
    }
    #[test]
    fn step_strings_escape_quotes_reverse_solidus_and_unicode_without_raw_control_text() {
        let s = format!("{}", Quoted("Panel's \\ é Σ 😀\n"));
        assert_eq!(
            s,
            "'Panel''s \\\\ \\X2\\00E9\\X0\\ \\X2\\03A3\\X0\\ \\X4\\0001F600\\X0\\\\X2\\000A\\X0\\'"
        );
        assert!(s.is_ascii());
    }
}
