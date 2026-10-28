// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{ArchiveError, ArchiveLimits, NativeArchive};
use alloc::vec::Vec;
use exedra_analytic::{
    body::{
        Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, FaceId, LoopId, LoopTrims,
        Sense, TrimCurve, ValidationPolicy, VertexId,
    },
    geometry::{Curve, CurveSpan, ParameterDomain, ParameterRange, Surface},
};
use exedra_compile::NativeSource;
use exedra_constructive::{ir::Placement3, profile::SegTag};

struct Encoder {
    words: Vec<u32>,
    scalars: Vec<f64>,
}
impl Encoder {
    fn count(&mut self, count: usize) -> Result<(), ArchiveError> {
        self.words
            .push(u32::try_from(count).map_err(|_| ArchiveError::Budget("u32 table length"))?);
        Ok(())
    }
    fn frame(&mut self, frame: Placement3) {
        self.scalars.extend(frame.rows.into_iter().flatten());
    }
    fn curve(&mut self, curve: &Curve) {
        match curve {
            Curve::Line(v) => {
                self.words.push(0);
                self.scalars.extend(v.origin());
                self.scalars.extend(v.direction());
            }
            Curve::Circle(v) => {
                self.words.push(1);
                self.frame(v.frame());
                self.scalars.push(v.radius());
            }
            Curve::Ellipse(v) => {
                self.words.push(2);
                self.frame(v.frame());
                self.scalars.extend([v.semi_major(), v.semi_minor()]);
            }
        }
    }
    fn surface(&mut self, surface: &Surface) {
        match surface {
            Surface::Plane(v) => {
                self.words.push(0);
                self.frame(v.frame());
            }
            Surface::Cylinder(v) => {
                self.words.push(1);
                self.frame(v.frame());
                self.scalars.push(v.radius());
            }
            Surface::Cone(v) => {
                self.words.push(2);
                self.frame(v.frame());
                self.scalars.extend([v.radius(), v.slope()]);
            }
            Surface::Sphere(v) => {
                self.words.push(3);
                self.frame(v.frame());
                self.scalars.push(v.radius());
            }
            Surface::Torus(v) => {
                self.words.push(4);
                self.frame(v.frame());
                self.scalars.extend([v.major_radius(), v.minor_radius()]);
            }
            Surface::Ruled(v) => {
                self.words.push(5);
                for span in v.curves() {
                    self.curve(span.curve());
                    self.words.push(u32::from(span.reversed()));
                    self.scalars.extend(span.range().bounds());
                }
            }
        }
    }
}
pub(super) fn capture(
    source: &NativeSource,
    limits: &ArchiveLimits,
) -> Result<NativeArchive, ArchiveError> {
    use exedra_compile::NativeGeometryRef;
    match source.geometry() {
        NativeGeometryRef::Extrusion(v) => capture_body(v.body(), source, limits),
        NativeGeometryRef::Revolution(v) => capture_body(v.body(), source, limits),
        NativeGeometryRef::Sweep(v) => capture_body(v.body(), source, limits),
        NativeGeometryRef::Loft(v) => capture_body(v.body(), source, limits),
        NativeGeometryRef::BoredPlate(v) => capture_body(v.body(), source, limits),
        NativeGeometryRef::Body { body, .. } => capture_body(body, source, limits),
    }
}
fn curve_size(curve: &Curve) -> usize {
    match curve {
        Curve::Line(_) => 6,
        Curve::Circle(_) => 13,
        Curve::Ellipse(_) => 14,
    }
}
fn surface_size(surface: &Surface) -> (usize, usize) {
    match surface {
        Surface::Plane(_) => (1, 12),
        Surface::Cylinder(_) | Surface::Sphere(_) => (1, 13),
        Surface::Cone(_) | Surface::Torus(_) => (1, 14),
        Surface::Ruled(v) => (
            5,
            v.curves()
                .iter()
                .map(|span| curve_size(span.curve()) + 2)
                .sum(),
        ),
    }
}
fn preflight<F, E>(
    body: &Body<F, E>,
    source: &NativeSource,
    limits: &ArchiveLimits,
) -> Result<(usize, usize), ArchiveError> {
    // Count borrowed records before cloning geometry or allocating output buffers.
    let mut words = 5_usize;
    let mut boundary_checks = 0;
    let mut scalars = 12_usize.saturating_add(body.vertices().len().saturating_mul(3));
    let mut items = body
        .vertices()
        .len()
        .saturating_add(body.edges().len())
        .saturating_add(body.loops().len())
        .saturating_add(body.faces().len())
        .saturating_add(body.trims().len());
    for edge in body.edges() {
        words = words.saturating_add(3);
        if let Some((curve, _)) = edge.regular() {
            words = words.saturating_add(1);
            scalars = scalars.saturating_add(curve_size(curve) + 2);
        }
    }
    for ring in body.loops() {
        words = words
            .saturating_add(1)
            .saturating_add(ring.coedges.len().saturating_mul(2));
        items = items.saturating_add(ring.coedges.len());
    }
    for (index, face) in body.faces().iter().enumerate() {
        for loop_id in &face.loops {
            charge_boundary_checks(
                &mut boundary_checks,
                body.loops()[loop_id.index()].coedges.len(),
                limits,
            )?;
        }
        let tags = source
            .face_tags(FaceId(
                u32::try_from(index).map_err(|_| ArchiveError::Budget("u32 face identity"))?,
            ))
            .expect("iterated native face")
            .count();
        let (surface_words, surface_scalars) = surface_size(&face.surface);
        words = words
            .saturating_add(surface_words + 3)
            .saturating_add(face.loops.len())
            .saturating_add(tags);
        scalars = scalars.saturating_add(surface_scalars + 4);
        items = items.saturating_add(face.loops.len()).saturating_add(tags);
    }
    for trims in body.trims() {
        words = words.saturating_add(2).saturating_add(trims.curves.len());
        items = items.saturating_add(trims.curves.len());
        for curve in &trims.curves {
            scalars = scalars.saturating_add(match curve {
                TrimCurve::Line { .. } => 4,
                TrimCurve::Harmonic { .. } => 10,
            });
        }
    }
    if words > limits.topology_words {
        return Err(ArchiveError::Budget("topology words"));
    }
    if scalars > limits.geometry_values {
        return Err(ArchiveError::Budget("geometry values"));
    }
    if items > limits.decoded_items {
        return Err(ArchiveError::Budget("decoded items"));
    }
    Ok((words, scalars))
}
fn charge_boundary_checks(
    checks: &mut usize,
    coedges: usize,
    limits: &ArchiveLimits,
) -> Result<(), ArchiveError> {
    *checks = checks
        .checked_add(coedges)
        .filter(|&total| total <= limits.max_boundary_checks)
        .ok_or(ArchiveError::Budget("boundary checks"))?;
    Ok(())
}
fn capture_body<F, E>(
    body: &Body<F, E>,
    source: &NativeSource,
    limits: &ArchiveLimits,
) -> Result<NativeArchive, ArchiveError> {
    let (words, scalars) = preflight(body, source, limits)?;
    let mut out = Encoder {
        words: Vec::with_capacity(words),
        scalars: Vec::with_capacity(scalars),
    };
    out.frame(source.frame());
    out.count(body.vertices().len())?;
    for &point in body.vertices() {
        out.scalars.extend(point);
    }
    out.count(body.edges().len())?;
    for edge in body.edges() {
        out.words.extend([edge.start.0, edge.end.0]);
        if let Some((curve, range)) = edge.regular() {
            out.words.push(1);
            out.curve(curve);
            out.scalars.extend(range.bounds());
        } else {
            out.words.push(0);
        }
    }
    out.count(body.loops().len())?;
    for ring in body.loops() {
        out.count(ring.coedges.len())?;
        for c in &ring.coedges {
            out.words.extend([c.edge.0, u32::from(c.reversed)]);
        }
    }
    out.count(body.faces().len())?;
    for (index, face) in body.faces().iter().enumerate() {
        out.surface(&face.surface);
        out.scalars.extend(face.domain.u.bounds());
        out.scalars.extend(face.domain.v.bounds());
        out.words.push(u32::from(face.sense == Sense::Reverse));
        out.count(face.loops.len())?;
        out.words.extend(face.loops.iter().map(|id| id.0));
        let face_id = FaceId(u32::try_from(index).expect("body face IDs fit u32"));
        out.count(
            source
                .face_tags(face_id)
                .expect("iterated native face")
                .count(),
        )?;
        out.words.extend(
            source
                .face_tags(face_id)
                .expect("iterated native face")
                .map(|tag| tag.0),
        );
    }
    out.count(body.trims().len())?;
    for trims in body.trims() {
        out.words.push(trims.loop_id.0);
        out.count(trims.curves.len())?;
        for curve in &trims.curves {
            match curve {
                TrimCurve::Line { start, end } => {
                    out.words.push(0);
                    out.scalars.extend(*start);
                    out.scalars.extend(*end);
                }
                TrimCurve::Harmonic {
                    origin,
                    linear,
                    cosine,
                    sine,
                    range,
                } => {
                    out.words.push(1);
                    for values in [*origin, *linear, *cosine, *sine, range.bounds()] {
                        out.scalars.extend(values);
                    }
                }
            }
        }
    }
    debug_assert_eq!(
        out.words.len(),
        words,
        "preflight integer count must match encoder output"
    );
    debug_assert_eq!(
        out.scalars.len(),
        scalars,
        "preflight scalar count must match encoder output"
    );
    let archive = NativeArchive {
        version: 1,
        topology: out.words,
        geometry: out.scalars,
    };
    check_limits(&archive, limits)?;
    Ok(archive)
}
fn check_limits(archive: &NativeArchive, limits: &ArchiveLimits) -> Result<(), ArchiveError> {
    if archive.version != 1 {
        return Err(ArchiveError::Version(archive.version));
    }
    if archive.topology.len() > limits.topology_words {
        return Err(ArchiveError::Budget("topology words"));
    }
    if archive.geometry.len() > limits.geometry_values {
        return Err(ArchiveError::Budget("geometry values"));
    }
    Ok(())
}
struct Decoder<'a> {
    words: &'a [u32],
    scalars: &'a [f64],
    word: usize,
    scalar: usize,
    items: usize,
    limit: usize,
    record: &'static str,
    index: usize,
}
impl Decoder<'_> {
    fn geometry(&self, error: exedra_analytic::geometry::GeometryError) -> ArchiveError {
        ArchiveError::Geometry {
            record: self.record,
            index: self.index,
            word: self.word,
            scalar: self.scalar,
            error,
        }
    }
    fn error(&self, record: &'static str) -> ArchiveError {
        ArchiveError::Encoding {
            record,
            table: self.record,
            index: self.index,
            word: self.word,
            scalar: self.scalar,
        }
    }
    fn word(&mut self, record: &'static str) -> Result<u32, ArchiveError> {
        let value = *self
            .words
            .get(self.word)
            .ok_or_else(|| self.error(record))?;
        self.word += 1;
        Ok(value)
    }
    fn boolean(&mut self, record: &'static str) -> Result<bool, ArchiveError> {
        match self.word(record)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.error(record)),
        }
    }
    fn count(
        &mut self,
        record: &'static str,
        words: usize,
        scalars: usize,
    ) -> Result<usize, ArchiveError> {
        let count = self.word(record)? as usize;
        self.items = self
            .items
            .checked_add(count)
            .ok_or(ArchiveError::Budget("decoded items"))?;
        if self.items > self.limit {
            return Err(ArchiveError::Budget("decoded items"));
        }
        if count
            .checked_mul(words)
            .is_none_or(|n| n > self.words.len().saturating_sub(self.word))
            || count
                .checked_mul(scalars)
                .is_none_or(|n| n > self.scalars.len().saturating_sub(self.scalar))
        {
            return Err(self.error(record));
        }
        Ok(count)
    }
    fn values<const N: usize>(&mut self, record: &'static str) -> Result<[f64; N], ArchiveError> {
        let end = self
            .scalar
            .checked_add(N)
            .ok_or_else(|| self.error(record))?;
        let values = self
            .scalars
            .get(self.scalar..end)
            .ok_or_else(|| self.error(record))?;
        let array = values.try_into().expect("slice has N elements");
        self.scalar = end;
        Ok(array)
    }
    fn frame(&mut self) -> Result<Placement3, ArchiveError> {
        Ok(Placement3 {
            rows: [
                self.values("frame")?,
                self.values("frame")?,
                self.values("frame")?,
            ],
        })
    }
    fn range(&mut self) -> Result<ParameterRange, ArchiveError> {
        let [min, max] = self.values("parameter range")?;
        ParameterRange::new(min, max).map_err(|error| self.geometry(error))
    }
    fn curve(&mut self) -> Result<Curve, ArchiveError> {
        match self.word("curve kind")? {
            0 => Curve::line(self.values("line origin")?, self.values("line direction")?)
                .map_err(|error| self.geometry(error)),
            1 => {
                let frame = self.frame()?;
                let [radius] = self.values("circle radius")?;
                Curve::circle(frame, radius).map_err(|error| self.geometry(error))
            }
            2 => {
                let frame = self.frame()?;
                let [major, minor] = self.values("ellipse radii")?;
                Curve::ellipse(frame, major, minor).map_err(|error| self.geometry(error))
            }
            _ => Err(self.error("curve kind")),
        }
    }
    fn surface(&mut self) -> Result<Surface, ArchiveError> {
        let kind = self.word("surface kind")?;
        if kind == 5 {
            let mut span = || {
                let curve = self.curve()?;
                let reversed = self.boolean("ruled traversal")?;
                let range = self.range()?;
                CurveSpan::new(curve, range, reversed).map_err(|error| self.geometry(error))
            };
            return Surface::ruled(span()?, span()?).map_err(|error| self.geometry(error));
        }
        if kind > 4 {
            return Err(self.error("surface kind"));
        }
        let frame = self.frame()?;
        match kind {
            0 => Surface::plane(frame),
            1 => {
                let [radius] = self.values("cylinder radius")?;
                Surface::cylinder(frame, radius)
            }
            2 => {
                let [radius, slope] = self.values("cone radius/slope")?;
                Surface::cone(frame, radius, slope)
            }
            3 => {
                let [radius] = self.values("sphere radius")?;
                Surface::sphere(frame, radius)
            }
            4 => {
                let [major, minor] = self.values("torus radii")?;
                Surface::torus(frame, major, minor)
            }
            _ => unreachable!("surface kind checked"),
        }
        .map_err(|error| self.geometry(error))
    }
}
pub(super) fn restore(
    archive: &NativeArchive,
    policy: &ValidationPolicy,
    limits: &ArchiveLimits,
) -> Result<NativeSource, ArchiveError> {
    restore_streams(
        archive.version,
        &archive.topology,
        &archive.geometry,
        policy,
        limits,
    )
}
pub(super) fn restore_streams(
    version: u32,
    words: &[u32],
    scalars: &[f64],
    policy: &ValidationPolicy,
    limits: &ArchiveLimits,
) -> Result<NativeSource, ArchiveError> {
    if version != 1 {
        return Err(ArchiveError::Version(version));
    }
    if words.len() > limits.topology_words {
        return Err(ArchiveError::Budget("topology words"));
    }
    if scalars.len() > limits.geometry_values {
        return Err(ArchiveError::Budget("geometry values"));
    }
    let mut input = Decoder {
        words,
        scalars,
        word: 0,
        scalar: 0,
        items: 0,
        limit: limits.decoded_items,
        record: "occurrence",
        index: 0,
    };
    let frame = input.frame()?;
    let column = |i| frame.rows.map(|r| r[i]);
    Placement3::try_from_orthonormal_axes(
        column(0),
        column(1),
        column(2),
        column(3),
        exedra_analytic::geometry::FRAME_TOLERANCE,
    )
    .map_err(ArchiveError::Placement)?;
    input.record = "vertex";
    let vertices = (0..input.count("vertex count", 0, 3)?)
        .map(|index| {
            input.index = index;
            input.values("vertex")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut edges = Vec::new();
    input.record = "edge";
    input.index = 0;
    for index in 0..input.count("edge count", 3, 0)? {
        input.record = "edge";
        input.index = index;
        let start = VertexId(input.word("edge start")?);
        let end = VertexId(input.word("edge end")?);
        for vertex in [start, end] {
            if vertex.index() >= vertices.len() {
                return Err(ArchiveError::Boundary(
                    exedra_analytic::body::BodyError::MissingVertex {
                        edge: EdgeId(u32::try_from(index).expect("encoded count fits u32")),
                        vertex,
                    },
                ));
            }
        }
        edges.push(if input.boolean("edge geometry")? {
            let curve = input.curve()?;
            let range = input.range()?;
            Edge::new(curve, range, start, end, ())
        } else {
            if start != end {
                return Err(input.error("collapsed edge endpoints"));
            }
            Edge::collapsed(start, ())
        });
    }
    let mut loops = Vec::new();
    input.record = "loop";
    for index in 0..input.count("loop count", 1, 0)? {
        input.index = index;
        let mut coedges = Vec::new();
        for occurrence in 0..input.count("coedge count", 2, 0)? {
            let edge = EdgeId(input.word("coedge edge")?);
            if edge.index() >= edges.len() {
                return Err(ArchiveError::Boundary(
                    exedra_analytic::body::BodyError::MissingEdge {
                        loop_id: LoopId(u32::try_from(index).expect("encoded count fits u32")),
                        coedge: occurrence,
                        edge,
                    },
                ));
            }
            coedges.push(Coedge {
                edge,
                reversed: input.boolean("coedge traversal")?,
            });
        }
        loops.push(BoundaryLoop { coedges });
    }
    let mut faces = Vec::new();
    let mut boundary_checks = 0;
    input.record = "face";
    input.index = 0;
    for index in 0..input.count("face count", 4, 16)? {
        input.record = "face";
        input.index = index;
        let surface = input.surface()?;
        let domain = ParameterDomain {
            u: input.range()?,
            v: input.range()?,
        };
        let sense = if input.boolean("face sense")? {
            Sense::Reverse
        } else {
            Sense::Forward
        };
        let face_loops = (0..input.count("face loop count", 1, 0)?)
            .map(|_| {
                let loop_id = LoopId(input.word("face loop")?);
                if loop_id.index() >= loops.len() {
                    return Err(ArchiveError::Boundary(
                        exedra_analytic::body::BodyError::MissingLoop {
                            face: FaceId(u32::try_from(index).expect("encoded count fits u32")),
                            loop_id,
                        },
                    ));
                }
                charge_boundary_checks(
                    &mut boundary_checks,
                    loops[loop_id.index()].coedges.len(),
                    limits,
                )?;
                Ok(loop_id)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let role = (0..input.count("face tag count", 1, 0)?)
            .map(|_| input.word("face tag").map(SegTag))
            .collect::<Result<Vec<_>, _>>()?;
        faces.push(Face {
            surface,
            domain,
            sense,
            loops: face_loops,
            role,
        });
    }
    let mut trims = Vec::new();
    input.record = "trim";
    input.index = 0;
    for index in 0..input.count("trim record count", 2, 0)? {
        input.record = "trim";
        input.index = index;
        let loop_id = LoopId(input.word("trim loop")?);
        if loop_id.index() >= loops.len() {
            return Err(ArchiveError::Boundary(
                exedra_analytic::body::BodyError::TrimStructure {
                    loop_id,
                    reason: "missing loop",
                },
            ));
        }
        let mut curves = Vec::new();
        for _ in 0..input.count("trim curve count", 1, 4)? {
            curves.push(match input.word("trim curve kind")? {
                0 => TrimCurve::Line {
                    start: input.values("trim start")?,
                    end: input.values("trim end")?,
                },
                1 => TrimCurve::Harmonic {
                    origin: input.values("trim origin")?,
                    linear: input.values("trim linear")?,
                    cosine: input.values("trim cosine")?,
                    sine: input.values("trim sine")?,
                    range: input.range()?,
                },
                _ => return Err(input.error("trim curve kind")),
            });
        }
        trims.push(LoopTrims { loop_id, curves });
    }
    if input.word != input.words.len() || input.scalar != input.scalars.len() {
        return Err(input.error("surplus data"));
    }
    let body = Body::try_from_parts(
        BodyParts {
            vertices,
            edges,
            loops,
            faces,
            trims,
        },
        policy,
    )
    .map_err(ArchiveError::Boundary)?;
    NativeSource::from_body(body, frame).map_err(ArchiveError::Placement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn decoder<'a>(words: &'a [u32], scalars: &'a [f64]) -> Decoder<'a> {
        Decoder {
            words,
            scalars,
            word: 0,
            scalar: 0,
            items: 0,
            limit: 1000,
            record: "test",
            index: 0,
        }
    }
    #[test]
    fn every_curve_and_surface_preserves_exact_scalar_bits() {
        let frame = Placement3::rotate_y_then_translate(0.37, -0.0, 3.25, -7.123456789012345);
        let curves = vec![
            Curve::line([-0.0, 1.0, 2.0], [0.3, 0.7, -0.0]).unwrap(),
            Curve::circle(frame, 1.23456789012345).unwrap(),
            Curve::ellipse(frame, 3.12345678901234, 1.23456789012345).unwrap(),
        ];
        let a = CurveSpan::new(
            Curve::line([-0.0, 0.0, 0.0], [2.0, 0.0, 0.0]).unwrap(),
            ParameterRange::new(-0.2, 0.8).unwrap(),
            false,
        )
        .unwrap();
        let b = CurveSpan::new(
            Curve::line([-0.0, 0.0, 3.0], [2.0, 0.0, 0.0]).unwrap(),
            ParameterRange::new(-0.2, 0.8).unwrap(),
            false,
        )
        .unwrap();
        let surfaces = vec![
            Surface::plane(frame).unwrap(),
            Surface::cylinder(frame, 1.23456789012345).unwrap(),
            Surface::cone(frame, 2.34567890123456, -0.34567890123456).unwrap(),
            Surface::sphere(frame, 3.45678901234567).unwrap(),
            Surface::torus(frame, 8.34567890123456, 1.23456789012345).unwrap(),
            Surface::ruled(a, b).unwrap(),
        ];
        let mut original = Encoder {
            words: vec![],
            scalars: vec![],
        };
        for c in &curves {
            original.curve(c);
        }
        for s in &surfaces {
            original.surface(s);
        }
        let mut input = decoder(&original.words, &original.scalars);
        let mut again = Encoder {
            words: vec![],
            scalars: vec![],
        };
        for expected in curves {
            let actual = input.curve().unwrap();
            assert_eq!(actual, expected);
            again.curve(&actual);
        }
        for expected in surfaces {
            let actual = input.surface().unwrap();
            assert_eq!(actual, expected);
            again.surface(&actual);
        }
        assert_eq!(again.words, original.words);
        assert_eq!(
            again
                .scalars
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            original
                .scalars
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn malformed_geometry_reports_owning_record_index_and_cursors() {
        let mut input = decoder(&[0], &[0.0; 6]);
        input.record = "edge";
        input.index = 23;
        assert!(matches!(
            input.curve(),
            Err(ArchiveError::Geometry {
                record: "edge",
                index: 23,
                word: 1,
                scalar: 6,
                error: exedra_analytic::geometry::GeometryError::ZeroDirection
            })
        ));
        let mut input = decoder(&[99], &[]);
        input.record = "face";
        input.index = 4;
        assert!(matches!(
            input.surface(),
            Err(ArchiveError::Encoding {
                table: "face",
                index: 4,
                record: "surface kind",
                word: 1,
                scalar: 0
            })
        ));
    }
    #[test]
    fn counts_require_record_specific_remaining_storage_before_work() {
        let mut input = decoder(&[100, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0.0; 12]);
        assert!(matches!(
            input.count("vertex count", 0, 3),
            Err(ArchiveError::Encoding {
                record: "vertex count",
                word: 1,
                scalar: 0,
                ..
            })
        ));
        let mut input = decoder(&[u32::MAX], &[]);
        input.limit = usize::MAX;
        assert!(matches!(
            input.count("coedge count", 2, 0),
            Err(ArchiveError::Encoding { .. })
        ));
    }
}
