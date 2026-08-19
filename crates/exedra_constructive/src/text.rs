// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `constructive-ir-v1`: a deterministic, human-diffable text rendering of
//! a recipe, with a round-trip parser.
//!
//! The format follows the workspace golden-file conventions: one header
//! line, ordered sections, every `f64` as a 16-digit uppercase hex bit
//! pattern (bit-exact, no shortest-float ambiguity), and no maps anywhere.
//! Parsing rebuilds through [`RecipeBuilder`], so a parsed recipe re-runs
//! full validation and recomputes fingerprints — round-trip fingerprint
//! equality is the format's correctness oracle.
//!
//! This is the goldens/review/debugging format. The serde interchange
//! format (JSON, for external frontends) is a separate surface.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use kurbo::Point;

use crate::ir::{
    CapMode, CsgOp, FramePolicy, LoftPolicy, NodeId, NodeKind, Path3, Placement3, Plane3,
    PrimitiveSpec, ProfileId, Recipe, RecipeBuilder, RecipeError,
};
use crate::profile::{Loop2, Profile2, ProfileError, Seg2, SegKind, SegTag};

/// Header line of the format.
pub const HEADER: &str = "constructive-ir-v1";

fn hex(v: f64) -> String {
    format!("{:016X}", v.to_bits())
}

fn put_point(out: &mut String, p: Point) {
    let _ = write!(out, "{} {}", hex(p.x), hex(p.y));
}

fn put_placement(out: &mut String, p: &Placement3) {
    for row in &p.rows {
        for &v in row {
            let _ = write!(out, " {}", hex(v));
        }
    }
}

fn put_caps(out: &mut String, caps: CapMode) {
    let name = match caps {
        CapMode::Both => "both",
        CapMode::Start => "start",
        CapMode::End => "end",
        CapMode::None => "none",
    };
    let _ = write!(out, "{name}");
}

/// Renders a recipe as `constructive-ir-v1` text.
#[must_use]
pub fn dump_recipe(recipe: &Recipe) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{HEADER}");
    let _ = writeln!(out, "schema {}", crate::EVAL_SCHEMA_VERSION);

    let _ = writeln!(out, "sources {}", recipe.sources().len());
    for (index, source) in recipe.sources().iter().enumerate() {
        let _ = writeln!(out, "  source {index} {source:?}");
    }
    let _ = writeln!(out, "slots {}", recipe.slots().len());
    for (index, slot) in recipe.slots().iter().enumerate() {
        let _ = writeln!(out, "  slot {index} {slot:?}");
    }

    let _ = writeln!(out, "profiles {}", recipe.profiles().len());
    for (index, profile) in recipe.profiles().iter().enumerate() {
        let _ = writeln!(out, "  profile {index} holes {}", profile.holes().len());
        dump_loop(&mut out, profile.outer());
        for hole in profile.holes() {
            dump_loop(&mut out, hole);
        }
    }

    let _ = writeln!(out, "nodes {}", recipe.nodes().len());
    for (index, node) in recipe.nodes().iter().enumerate() {
        let mut line = format!("  node {index} ");
        dump_kind(&mut line, &node.kind);
        match node.source {
            None => line.push_str(" source -"),
            Some(id) => {
                let _ = write!(line, " source {}", id.0);
            }
        }
        match node.material {
            None => line.push_str(" material -"),
            Some(id) => {
                let _ = write!(line, " material {}", id.0);
            }
        }
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out, "root {}", recipe.root().0);
    out
}

fn dump_loop(out: &mut String, source: &Loop2) {
    let _ = writeln!(out, "    loop {}", source.segs().len());
    for seg in source.segs() {
        let mut line = String::from("      ");
        match seg.kind {
            SegKind::Line => line.push_str("line to "),
            SegKind::Arc { bulge } => {
                let _ = write!(line, "arc {} to ", hex(bulge));
            }
            SegKind::Cubic { c1, c2 } => {
                line.push_str("cubic ");
                put_point(&mut line, c1);
                line.push(' ');
                put_point(&mut line, c2);
                line.push_str(" to ");
            }
        }
        put_point(&mut line, seg.to);
        match seg.tag {
            None => line.push_str(" tag -"),
            Some(SegTag(tag)) => {
                let _ = write!(line, " tag {tag}");
            }
        }
        let _ = writeln!(out, "{line}");
    }
}

fn dump_kind(line: &mut String, kind: &NodeKind) {
    match kind {
        NodeKind::Extrude {
            profile,
            placement,
            height,
            caps,
        } => {
            let _ = write!(
                line,
                "extrude profile {} height {} caps ",
                profile.0,
                hex(*height)
            );
            put_caps(line, *caps);
            line.push_str(" placement");
            put_placement(line, placement);
        }
        NodeKind::Revolve {
            profile,
            placement,
            sweep,
            caps,
        } => {
            let _ = write!(
                line,
                "revolve profile {} sweep {} caps ",
                profile.0,
                hex(*sweep)
            );
            put_caps(line, *caps);
            line.push_str(" placement");
            put_placement(line, placement);
        }
        NodeKind::Loft {
            sections,
            policy,
            caps,
        } => {
            let LoftPolicy::Ruled = policy;
            let _ = write!(line, "loft ruled sections {} caps ", sections.len());
            put_caps(line, *caps);
            for (placement, profile) in sections {
                let _ = write!(line, " section {} placement", profile.0);
                put_placement(line, placement);
            }
        }
        NodeKind::Sweep {
            profile,
            path,
            caps,
        } => {
            let Path3::Polyline { points, frame } = path;
            let FramePolicy::RotationMinimizing = frame;
            let _ = write!(
                line,
                "sweep profile {} frame rmf points {} caps ",
                profile.0,
                points.len()
            );
            put_caps(line, *caps);
            for p in points {
                let _ = write!(line, " {} {} {}", hex(p[0]), hex(p[1]), hex(p[2]));
            }
        }
        NodeKind::PlanarFace { profile, placement } => {
            let _ = write!(line, "planar_face profile {} placement", profile.0);
            put_placement(line, placement);
        }
        NodeKind::Primitive { spec, placement } => {
            match spec {
                PrimitiveSpec::Box { size } => {
                    let _ = write!(
                        line,
                        "primitive box {} {} {}",
                        hex(size[0]),
                        hex(size[1]),
                        hex(size[2])
                    );
                }
                PrimitiveSpec::Cylinder {
                    radius,
                    height,
                    segments,
                } => {
                    let _ = write!(
                        line,
                        "primitive cylinder {} {} {segments}",
                        hex(*radius),
                        hex(*height)
                    );
                }
            }
            line.push_str(" placement");
            put_placement(line, placement);
        }
        NodeKind::Csg { op, operands } => {
            let name = match op {
                CsgOp::Union => "union",
                CsgOp::Difference => "difference",
                CsgOp::Intersection => "intersection",
            };
            let _ = write!(line, "csg {name} operands {}", operands.len());
            for operand in operands {
                let _ = write!(line, " {}", operand.0);
            }
        }
        NodeKind::Transform { child, xf } => {
            let _ = write!(line, "transform child {} placement", child.0);
            put_placement(line, xf);
        }
        NodeKind::Mirror { child, plane } => {
            let _ = write!(
                line,
                "mirror child {} plane {} {} {} {}",
                child.0,
                hex(plane.normal[0]),
                hex(plane.normal[1]),
                hex(plane.normal[2]),
                hex(plane.distance)
            );
        }
        NodeKind::Instance { of, placement } => {
            let _ = write!(line, "instance of {} placement", of.0);
            put_placement(line, placement);
        }
        NodeKind::Group { children } => {
            let _ = write!(line, "group children {}", children.len());
            for child in children {
                let _ = write!(line, " {}", child.0);
            }
        }
        NodeKind::Stretch {
            child,
            plane,
            length,
        } => {
            let _ = write!(
                line,
                "stretch child {} plane {} {} {} {} length {}",
                child.0,
                hex(plane.normal[0]),
                hex(plane.normal[1]),
                hex(plane.normal[2]),
                hex(plane.distance),
                hex(*length)
            );
        }
    }
}

/// Typed parse failure with a 1-based line number.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TextError {
    /// The header or schema line is missing or wrong.
    BadHeader,
    /// A line failed to parse.
    Malformed {
        /// 1-based line number.
        line: usize,
    },
    /// The rebuilt profile failed validation.
    Profile(ProfileError),
    /// The rebuilt recipe failed validation.
    Recipe(RecipeError),
}

impl core::fmt::Display for TextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadHeader => write!(f, "missing or unsupported header"),
            Self::Malformed { line } => write!(f, "malformed line {line}"),
            Self::Profile(e) => write!(f, "profile validation failed: {e}"),
            Self::Recipe(e) => write!(f, "recipe validation failed: {e}"),
        }
    }
}

impl core::error::Error for TextError {}

struct Lines<'a> {
    iter: core::iter::Enumerate<core::str::Lines<'a>>,
}

impl<'a> Lines<'a> {
    fn next(&mut self) -> Result<(usize, &'a str), TextError> {
        self.iter
            .next()
            .map(|(index, line)| (index + 1, line.trim()))
            .ok_or(TextError::Malformed { line: usize::MAX })
    }
}

fn parse_f64(token: &str, line: usize) -> Result<f64, TextError> {
    u64::from_str_radix(token, 16)
        .map(f64::from_bits)
        .map_err(|_| TextError::Malformed { line })
}

fn parse_usize(token: &str, line: usize) -> Result<usize, TextError> {
    token.parse().map_err(|_| TextError::Malformed { line })
}

fn parse_u32(token: &str, line: usize) -> Result<u32, TextError> {
    token.parse().map_err(|_| TextError::Malformed { line })
}

fn parse_placement(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<Placement3, TextError> {
    let mut rows = [[0.0; 4]; 3];
    for row in &mut rows {
        for cell in row.iter_mut() {
            let token = tokens.next().ok_or(TextError::Malformed { line })?;
            *cell = parse_f64(token, line)?;
        }
    }
    Ok(Placement3 { rows })
}

fn parse_caps(token: &str, line: usize) -> Result<CapMode, TextError> {
    match token {
        "both" => Ok(CapMode::Both),
        "start" => Ok(CapMode::Start),
        "end" => Ok(CapMode::End),
        "none" => Ok(CapMode::None),
        _ => Err(TextError::Malformed { line }),
    }
}

/// Parses `constructive-ir-v1` text back into a validated recipe.
///
/// # Errors
///
/// Returns a typed [`TextError`]; the rebuilt recipe re-runs full builder
/// validation, so malformed geometry is caught the same way direct
/// construction catches it.
pub fn parse_recipe(text: &str) -> Result<Recipe, TextError> {
    let mut lines = Lines {
        iter: text.lines().enumerate(),
    };
    let (_, header) = lines.next()?;
    if header != HEADER {
        return Err(TextError::BadHeader);
    }
    let (line, schema) = lines.next()?;
    let schema_value = schema
        .strip_prefix("schema ")
        .ok_or(TextError::Malformed { line })?;
    if parse_u32(schema_value, line)? != crate::EVAL_SCHEMA_VERSION {
        return Err(TextError::BadHeader);
    }

    let mut builder = RecipeBuilder::new();

    // Sources and slots intern in file order, so ids are stable.
    let (line, header) = lines.next()?;
    let count = section_count(header, "sources", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let rest = entry
            .strip_prefix("source ")
            .ok_or(TextError::Malformed { line })?;
        let (_, quoted) = rest.split_once(' ').ok_or(TextError::Malformed { line })?;
        let value = unquote(quoted).ok_or(TextError::Malformed { line })?;
        builder.source_ref(&value);
    }
    let (line, header) = lines.next()?;
    let count = section_count(header, "slots", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let rest = entry
            .strip_prefix("slot ")
            .ok_or(TextError::Malformed { line })?;
        let (_, quoted) = rest.split_once(' ').ok_or(TextError::Malformed { line })?;
        let value = unquote(quoted).ok_or(TextError::Malformed { line })?;
        builder.material_slot(&value);
    }

    let (line, header) = lines.next()?;
    let count = section_count(header, "profiles", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let holes: usize = entry
            .strip_prefix("profile ")
            .and_then(|rest| rest.split_once(" holes "))
            .map(|(_, holes)| holes)
            .ok_or(TextError::Malformed { line })
            .and_then(|token| parse_usize(token, line))?;
        let outer = parse_loop(&mut lines)?;
        let hole_loops = (0..holes)
            .map(|_| parse_loop(&mut lines))
            .collect::<Result<Vec<_>, _>>()?;
        let profile = Profile2::new(outer, hole_loops).map_err(TextError::Profile)?;
        builder.add_profile(profile);
    }

    let (line, header) = lines.next()?;
    let count = section_count(header, "nodes", line)?;
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let rest = entry
            .strip_prefix("node ")
            .ok_or(TextError::Malformed { line })?;
        let (_, body) = rest.split_once(' ').ok_or(TextError::Malformed { line })?;
        parse_node(&mut builder, body, line)?;
    }

    let (line, root_line) = lines.next()?;
    let root = root_line
        .strip_prefix("root ")
        .ok_or(TextError::Malformed { line })
        .and_then(|token| parse_u32(token, line))?;
    builder.finish(NodeId(root)).map_err(TextError::Recipe)
}

fn unquote(token: &str) -> Option<String> {
    // Values are Rust debug-quoted; parse the escape-free common case and
    // reject anything else (source refs are opaque identifiers by
    // convention).
    let inner = token.strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains('\\') {
        return None;
    }
    Some(String::from(inner))
}

fn section_count(header: &str, name: &str, line: usize) -> Result<usize, TextError> {
    header
        .strip_prefix(name)
        .map(str::trim)
        .ok_or(TextError::Malformed { line })
        .and_then(|token| parse_usize(token, line))
}

fn parse_loop(lines: &mut Lines<'_>) -> Result<Loop2, TextError> {
    let (line, header) = lines.next()?;
    let count = header
        .strip_prefix("loop ")
        .ok_or(TextError::Malformed { line })
        .and_then(|token| parse_usize(token, line))?;
    let mut segs = Vec::with_capacity(count);
    for _ in 0..count {
        let (line, entry) = lines.next()?;
        let mut tokens = entry.split_whitespace();
        let kind = tokens.next().ok_or(TextError::Malformed { line })?;
        let seg = match kind {
            "line" => {
                expect(&mut tokens, "to", line)?;
                let to = parse_point(&mut tokens, line)?;
                Seg2::line(to)
            }
            "arc" => {
                let bulge = next_f64(&mut tokens, line)?;
                expect(&mut tokens, "to", line)?;
                let to = parse_point(&mut tokens, line)?;
                Seg2::arc(to, bulge)
            }
            "cubic" => {
                let c1 = parse_point(&mut tokens, line)?;
                let c2 = parse_point(&mut tokens, line)?;
                expect(&mut tokens, "to", line)?;
                let to = parse_point(&mut tokens, line)?;
                Seg2::cubic(to, c1, c2)
            }
            _ => return Err(TextError::Malformed { line }),
        };
        expect(&mut tokens, "tag", line)?;
        let tag = tokens.next().ok_or(TextError::Malformed { line })?;
        let seg = if tag == "-" {
            seg
        } else {
            seg.tagged(SegTag(parse_u32(tag, line)?))
        };
        segs.push(seg);
    }
    Loop2::new(segs).map_err(TextError::Profile)
}

fn expect(
    tokens: &mut core::str::SplitWhitespace<'_>,
    expected: &str,
    line: usize,
) -> Result<(), TextError> {
    (tokens.next() == Some(expected))
        .then_some(())
        .ok_or(TextError::Malformed { line })
}

fn next_f64(tokens: &mut core::str::SplitWhitespace<'_>, line: usize) -> Result<f64, TextError> {
    let token = tokens.next().ok_or(TextError::Malformed { line })?;
    parse_f64(token, line)
}

fn next_u32(tokens: &mut core::str::SplitWhitespace<'_>, line: usize) -> Result<u32, TextError> {
    let token = tokens.next().ok_or(TextError::Malformed { line })?;
    parse_u32(token, line)
}

fn parse_point(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<Point, TextError> {
    let x = next_f64(tokens, line)?;
    let y = next_f64(tokens, line)?;
    Ok(Point::new(x, y))
}

fn parse_plane(
    tokens: &mut core::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<Plane3, TextError> {
    Ok(Plane3 {
        normal: [
            next_f64(tokens, line)?,
            next_f64(tokens, line)?,
            next_f64(tokens, line)?,
        ],
        distance: next_f64(tokens, line)?,
    })
}

fn parse_node(builder: &mut RecipeBuilder, body: &str, line: usize) -> Result<(), TextError> {
    // Split off the trailing `source X material Y` suffix.
    let (body, material) = split_suffix(body, "material", line)?;
    let (body, source) = split_suffix(&body, "source", line)?;
    if let Some(source) = parse_opt_index(&source, line)? {
        builder.with_source(crate::ir::SourceId(source));
    }
    if let Some(material) = parse_opt_index(&material, line)? {
        builder.with_material(crate::ir::SlotId(material));
    }

    let mut tokens = body.split_whitespace();
    let kind_name = tokens.next().ok_or(TextError::Malformed { line })?;
    let kind = match kind_name {
        "extrude" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "height", line)?;
            let height = next_f64(&mut tokens, line)?;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::Extrude {
                profile,
                placement,
                height,
                caps,
            }
        }
        "revolve" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "sweep", line)?;
            let sweep = next_f64(&mut tokens, line)?;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::Revolve {
                profile,
                placement,
                sweep,
                caps,
            }
        }
        "loft" => {
            expect(&mut tokens, "ruled", line)?;
            expect(&mut tokens, "sections", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            let mut sections = Vec::with_capacity(count);
            for _ in 0..count {
                expect(&mut tokens, "section", line)?;
                let profile = ProfileId(next_u32(&mut tokens, line)?);
                expect(&mut tokens, "placement", line)?;
                let placement = parse_placement(&mut tokens, line)?;
                sections.push((placement, profile));
            }
            NodeKind::Loft {
                sections,
                policy: LoftPolicy::Ruled,
                caps,
            }
        }
        "sweep" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "frame", line)?;
            expect(&mut tokens, "rmf", line)?;
            expect(&mut tokens, "points", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            expect(&mut tokens, "caps", line)?;
            let caps = parse_caps(tokens.next().ok_or(TextError::Malformed { line })?, line)?;
            let mut points = Vec::with_capacity(count);
            for _ in 0..count {
                points.push([
                    next_f64(&mut tokens, line)?,
                    next_f64(&mut tokens, line)?,
                    next_f64(&mut tokens, line)?,
                ]);
            }
            NodeKind::Sweep {
                profile,
                path: Path3::Polyline {
                    points,
                    frame: FramePolicy::RotationMinimizing,
                },
                caps,
            }
        }
        "planar_face" => {
            expect(&mut tokens, "profile", line)?;
            let profile = ProfileId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::PlanarFace { profile, placement }
        }
        "primitive" => {
            let which = tokens.next().ok_or(TextError::Malformed { line })?;
            let spec = match which {
                "box" => PrimitiveSpec::Box {
                    size: [
                        next_f64(&mut tokens, line)?,
                        next_f64(&mut tokens, line)?,
                        next_f64(&mut tokens, line)?,
                    ],
                },
                "cylinder" => PrimitiveSpec::Cylinder {
                    radius: next_f64(&mut tokens, line)?,
                    height: next_f64(&mut tokens, line)?,
                    segments: next_u32(&mut tokens, line)?,
                },
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::Primitive { spec, placement }
        }
        "csg" => {
            let op = match tokens.next().ok_or(TextError::Malformed { line })? {
                "union" => CsgOp::Union,
                "difference" => CsgOp::Difference,
                "intersection" => CsgOp::Intersection,
                _ => return Err(TextError::Malformed { line }),
            };
            expect(&mut tokens, "operands", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            let operands = (0..count)
                .map(|_| Ok(NodeId(next_u32(&mut tokens, line)?)))
                .collect::<Result<Vec<_>, TextError>>()?;
            NodeKind::Csg { op, operands }
        }
        "transform" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "placement", line)?;
            let xf = parse_placement(&mut tokens, line)?;
            NodeKind::Transform { child, xf }
        }
        "mirror" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "plane", line)?;
            let plane = parse_plane(&mut tokens, line)?;
            NodeKind::Mirror { child, plane }
        }
        "instance" => {
            expect(&mut tokens, "of", line)?;
            let of = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "placement", line)?;
            let placement = parse_placement(&mut tokens, line)?;
            NodeKind::Instance { of, placement }
        }
        "group" => {
            expect(&mut tokens, "children", line)?;
            let count = next_u32(&mut tokens, line)? as usize;
            let children = (0..count)
                .map(|_| Ok(NodeId(next_u32(&mut tokens, line)?)))
                .collect::<Result<Vec<_>, TextError>>()?;
            NodeKind::Group { children }
        }
        "stretch" => {
            expect(&mut tokens, "child", line)?;
            let child = NodeId(next_u32(&mut tokens, line)?);
            expect(&mut tokens, "plane", line)?;
            let plane = parse_plane(&mut tokens, line)?;
            expect(&mut tokens, "length", line)?;
            let length = next_f64(&mut tokens, line)?;
            NodeKind::Stretch {
                child,
                plane,
                length,
            }
        }
        _ => return Err(TextError::Malformed { line }),
    };
    builder.add(kind).map_err(TextError::Recipe)?;
    Ok(())
}

/// Splits `... <keyword> <token>` off the end of a node body.
fn split_suffix(body: &str, keyword: &str, line: usize) -> Result<(String, String), TextError> {
    let position = body
        .rfind(&format!(" {keyword} "))
        .ok_or(TextError::Malformed { line })?;
    let value = body[position + keyword.len() + 2..].trim();
    Ok((String::from(&body[..position]), String::from(value)))
}

fn parse_opt_index(token: &str, line: usize) -> Result<Option<u32>, TextError> {
    if token == "-" {
        Ok(None)
    } else {
        parse_u32(token, line).map(Some)
    }
}

/// Test-only fixture shared with the interchange tests.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::builders;
    use alloc::vec;

    /// A recipe exercising every node kind and segment kind.
    pub(crate) fn full_coverage_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let rect = b.add_profile(builders::rect(2.0, 1.0).expect("rect"));
        let rounded = b.add_profile(builders::rounded_rect(3.0, 2.0, 0.25).expect("rounded"));
        let ring = b.add_profile(builders::ring(1.0, 0.5).expect("ring"));
        let src = b.source_ref("text:demo");
        let slot = b.material_slot("front");

        let extrude = b
            .with_source(src)
            .with_material(slot)
            .add(NodeKind::Extrude {
                profile: rect,
                placement: Placement3::translate(0.5, 0.25, 0.0),
                height: 2.0,
                caps: CapMode::Both,
            })
            .expect("valid");
        let revolve = b
            .add(NodeKind::Revolve {
                profile: ring,
                placement: Placement3::translate(5.0, 0.0, 0.0),
                sweep: core::f64::consts::FRAC_PI_2,
                caps: CapMode::Start,
            })
            .expect("valid");
        let loft = b
            .add(NodeKind::Loft {
                sections: vec![
                    (Placement3::IDENTITY, rect),
                    (Placement3::translate(0.0, 0.0, 2.0), rect),
                ],
                policy: LoftPolicy::Ruled,
                caps: CapMode::None,
            })
            .expect("valid");
        let sweep = b
            .add(NodeKind::Sweep {
                profile: rounded,
                path: Path3::Polyline {
                    points: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [1.0, 0.0, 3.0]],
                    frame: FramePolicy::RotationMinimizing,
                },
                caps: CapMode::End,
            })
            .expect("valid");
        let face = b
            .add(NodeKind::PlanarFace {
                profile: rect,
                placement: Placement3::IDENTITY,
            })
            .expect("valid");
        let primitive = b
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Cylinder {
                    radius: 0.5,
                    height: 2.0,
                    segments: 16,
                },
                placement: Placement3::IDENTITY,
            })
            .expect("valid");
        let csg = b
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![extrude, primitive],
            })
            .expect("valid");
        let transform = b
            .add(NodeKind::Transform {
                child: csg,
                xf: Placement3::rotate_z_then_translate(0.3, 1.0, 2.0, 3.0),
            })
            .expect("valid");
        let mirror = b
            .add(NodeKind::Mirror {
                child: revolve,
                plane: Plane3 {
                    normal: [0.0, 1.0, 0.0],
                    distance: 0.5,
                },
            })
            .expect("valid");
        let instance = b
            .add(NodeKind::Instance {
                of: loft,
                placement: Placement3::translate(10.0, 0.0, 0.0),
            })
            .expect("valid");
        let stretch = b
            .add(NodeKind::Stretch {
                child: sweep,
                plane: Plane3 {
                    normal: [1.0, 0.0, 0.0],
                    distance: 1.0,
                },
                length: 0.5,
            })
            .expect("valid");
        let group = b
            .add(NodeKind::Group {
                children: vec![transform, mirror, instance, stretch, face],
            })
            .expect("valid");
        b.finish(group).expect("valid recipe")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use tests_support::full_coverage_recipe;

    #[test]
    fn round_trip_preserves_fingerprints() {
        let recipe = full_coverage_recipe();
        let text = dump_recipe(&recipe);
        let parsed = parse_recipe(&text).expect("parses");
        assert_eq!(
            recipe.recipe_fingerprint(),
            parsed.recipe_fingerprint(),
            "round trip must preserve content identity exactly"
        );
        // Second dump is byte-identical: the format is canonical.
        assert_eq!(text, dump_recipe(&parsed));
    }

    #[test]
    fn dump_is_deterministic_and_headed() {
        let recipe = full_coverage_recipe();
        let a = dump_recipe(&recipe);
        let b = dump_recipe(&recipe);
        assert_eq!(a, b);
        assert!(a.starts_with("constructive-ir-v1\nschema 1\n"));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(parse_recipe("nope"), Err(TextError::BadHeader));
        let mut text = String::from("constructive-ir-v1\nschema 1\nsources 0\nslots 0\n");
        text.push_str("profiles 0\nnodes 1\n  node 0 fancy thing source - material -\nroot 0\n");
        assert!(matches!(
            parse_recipe(&text),
            Err(TextError::Malformed { .. })
        ));
    }

    #[test]
    fn parse_revalidates_geometry() {
        // A structurally valid file with an invalid extrude height fails
        // through the builder, not silently.
        let recipe = {
            let mut b = RecipeBuilder::new();
            let p = b.add_profile(builders::rect(1.0, 1.0).expect("rect"));
            let n = b
                .add(NodeKind::Extrude {
                    profile: p,
                    placement: Placement3::IDENTITY,
                    height: 1.0,
                    caps: CapMode::Both,
                })
                .expect("valid");
            b.finish(n).expect("valid")
        };
        let text = dump_recipe(&recipe);
        let negative_height = hex(-1.0);
        let broken = text.replace(&hex(1.0), &negative_height);
        assert!(matches!(
            parse_recipe(&broken),
            Err(TextError::Recipe(_)) | Err(TextError::Profile(_))
        ));
    }
}
