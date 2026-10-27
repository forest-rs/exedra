// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    LengthUnit, SCHEMA, StepCorrespondence, StepDocument, StepEntityId, StepError, StepOptions,
    StepReport, preflight::Prepared,
};
use alloc::{string::String, vec, vec::Vec};
use core::fmt::{self, Write};
use exedra_analytic::{
    body::{Body, Sense},
    geometry::{Curve, Surface},
};
use exedra_math::{Placement3, norm};

struct Buffer {
    text: String,
    next: u32,
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
        if self.next > self.entities {
            return Err(StepError::Budget {
                resource: "entities",
                required: self.next as usize,
                maximum: self.entities as usize,
            });
        }
        let id = StepEntityId(self.next);
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
        let curve = match &edge.curve {
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
        };
        map.edges[i] = Some(out.entity(format_args!(
            "EDGE_CURVE('',{},{},{},.T.)",
            Id(map.vertices[edge.start.index()].expect("used")),
            Id(map.vertices[edge.end.index()].expect("used")),
            Id(curve)
        ))?);
    }
    let mut refs = Vec::new();
    for (i, (&active, ring)) in prepared.loops.iter().zip(body.loops()).enumerate() {
        if !active {
            continue;
        }
        refs.clear();
        refs.reserve(ring.coedges.len());
        for c in &ring.coedges {
            refs.push(out.entity(format_args!(
                "ORIENTED_EDGE('',*,*,{},{})",
                Id(map.edges[c.edge.index()].expect("used")),
                boolean(!c.reversed)
            ))?);
        }
        map.loops[i] = Some(out.entity(format_args!("EDGE_LOOP('',{})", Refs(&refs)))?);
    }
    for (i, face) in body.faces().iter().enumerate() {
        let frame = out.frame(placement * face.surface.frame())?;
        let surface = match &face.surface {
            Surface::Plane(_) => out.entity(format_args!("PLANE('',{})", Id(frame)))?,
            Surface::Cylinder(c) => out.entity(format_args!(
                "CYLINDRICAL_SURFACE('',{},{})",
                Id(frame),
                Real(c.radius())
            ))?,
            _ => unreachable!("preflight checked surface family"),
        };
        refs.clear();
        refs.reserve(face.loops.len());
        for (j, &loop_id) in face.loops.iter().enumerate() {
            let kind = if j == 0 {
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
    prepared.counts.entities = out.next - 1;
    let report = StepReport {
        schema: SCHEMA,
        units: options.units,
        declared_uncertainty: options.tolerance,
        max_native_support_gap: prepared.gap,
        frame_roundoff_bound: prepared.allowance,
        max_deviation: prepared.gap + prepared.allowance,
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
