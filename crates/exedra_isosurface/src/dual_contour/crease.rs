// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Restoring creases where a field's pieces meet.
//!
//! Where two surfaces meet at an acute angle, the solid between them tapers
//! below the grid, and sampling stops a cell or so short of the edge: the
//! mesh bridges from one surface to the other in a sawtooth. A field that
//! knows its pieces, through [`ScalarField::eval_crease`], says where the
//! nearest two meet: the crease is where the field and the tie between them
//! are both zero, and the tie's sign says which piece's side a point is on.
//!
//! First, Newton steps move each vertex near a crease onto it, within
//! `reach`, where no face around the vertex turns over. Then each edge still
//! bridging from one side to the other is split where it crosses the tie,
//! the new vertex moves onto the crease, and the faces either side are
//! divided along it, so every face keeps to one piece and the crease runs
//! through vertices. A face whose division would turn a triangle over keeps
//! its bridges.

use alloc::vec::Vec;
use exedra_math::{dot, lerp};
use exedra_mesh::{FaceId, Mesh, PropagatePolicy, VertexId, op};
use hashbrown::{HashMap, HashSet};

use super::control::RunContext;
use super::{DualContourError, newell_normal, triangle_is_nondegenerate};
use crate::ScalarField;

/// Newton steps before a point is left where it is.
const NEWTON_STEPS: usize = 8;

/// Sweeps over moves refused because a neighbor had not moved yet.
const PASSES: usize = 4;

/// A vertex within this fraction of the splitting reach of a crease is
/// taken to be on it: splitting its edges would put a vertex beside it.
const ON_CREASE: f64 = 64.0;

/// What restoring creases did.
pub(super) struct CreaseWork {
    /// Vertices moved onto a crease.
    pub(super) moved: usize,
    /// Edges split where they crossed a crease.
    pub(super) split: usize,
}

/// Moves vertices within `reach` of a crease onto it, then splits edges
/// that still bridge it, within twice `reach`.
pub(super) fn restore_creases<F: ScalarField>(
    field: &F,
    mesh: &mut Mesh,
    reach: f32,
    run: &RunContext,
) -> Result<CreaseWork, DualContourError> {
    let mut rows = HashMap::new();
    for vertex in mesh.vertices().collect::<Vec<_>>() {
        let Some(&position) = mesh.vertex_position(vertex) else {
            continue;
        };
        let answer = field.eval_crease(position);
        run.check()?;
        if let Some(answer) = answer {
            rows.insert(vertex, answer);
        }
    }
    let moved = project(field, mesh, &rows, reach, run)?;
    let split = split_bridges(field, mesh, &rows, &moved, 2.0 * reach, run)?;
    Ok(CreaseWork {
        moved: moved.len(),
        split,
    })
}

/// Whether the crease named by `rows` is within `reach`, by the first
/// Newton step toward it.
///
/// Where the pieces meet at a grazing angle the tie changes slowly, so its
/// own distance overstates how far the crease is; the step does not.
fn near(rows: [[f32; 4]; 2], reach: f32) -> bool {
    step(rows[0], rows[1]).is_some_and(|step| norm(step) <= 2.0 * f64::from(reach))
}

/// Moves vertices within `reach` of a crease onto it where no face around
/// them turns over. Returns the vertices moved.
fn project<F: ScalarField>(
    field: &F,
    mesh: &mut Mesh,
    rows: &HashMap<VertexId, [[f32; 4]; 2]>,
    reach: f32,
    run: &RunContext,
) -> Result<HashSet<VertexId>, DualContourError> {
    let mut targets = Vec::new();
    let mut candidates = rows
        .iter()
        .filter(|(_, rows)| near(**rows, reach))
        .map(|(&vertex, _)| vertex)
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    for vertex in candidates {
        let start = *mesh
            .vertex_position(vertex)
            .expect("vertices with rows are live");
        if let Some(target) = crease_point(field, start, reach, run)? {
            targets.push((vertex, target));
        }
    }
    if targets.is_empty() {
        return Ok(HashSet::new());
    }

    let faces_of = faces_of(mesh);
    let before = targets
        .iter()
        .flat_map(|(vertex, _)| faces_of.get(vertex).into_iter().flatten())
        .map(|&face| (face, face_normal(mesh, face, &HashMap::new())))
        .collect::<HashMap<_, _>>();
    // Moves are checked against the positions placed so far, then applied
    // together. A move that turns a face over may be fine once a neighbor
    // has moved too, so refused moves are tried again.
    let mut placed = HashMap::<VertexId, [f32; 3]>::new();
    let mut pending = targets;
    for _ in 0..PASSES {
        let placed_before = placed.len();
        let mut refused = Vec::new();
        for (vertex, target) in pending {
            let previous = placed.insert(vertex, target);
            let keeps_sides = faces_of.get(&vertex).into_iter().flatten().all(|face| {
                before
                    .get(face)
                    .is_some_and(|&normal| dot(face_normal(mesh, *face, &placed), normal) > 0.0)
            });
            if !keeps_sides {
                match previous {
                    Some(position) => placed.insert(vertex, position),
                    None => placed.remove(&vertex),
                };
                refused.push((vertex, target));
            }
        }
        pending = refused;
        if pending.is_empty() || placed.len() == placed_before {
            break;
        }
    }
    let mut session = mesh.edit();
    for (&vertex, &target) in &placed {
        op::set_vertex_position(&mut session, vertex, target)
            .expect("a vertex of a live face is live");
    }
    let _: () = session.finish();
    Ok(placed.into_keys().collect())
}

/// A corner of a face divided along a crease: one of its vertices, or the
/// point where one of its edges crosses the crease.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
enum Corner {
    Vertex(VertexId),
    Crossing(VertexId, VertexId),
}

/// Splits edges within `reach` of a crease whose ends lie on opposite sides
/// of it, and divides their faces along it. Returns the edges split.
fn split_bridges<F: ScalarField>(
    field: &F,
    mesh: &mut Mesh,
    rows: &HashMap<VertexId, [[f32; 4]; 2]>,
    on_crease: &HashSet<VertexId>,
    reach: f32,
    run: &RunContext,
) -> Result<usize, DualContourError> {
    // The side of its nearest tie each vertex near a crease lies on: none
    // for a vertex on it, moved there or sampled there already.
    let side = |vertex: VertexId| -> Option<i8> {
        if on_crease.contains(&vertex) {
            return Some(0);
        }
        let rows = *rows.get(&vertex)?;
        let distance = norm(step(rows[0], rows[1])?);
        if distance > 2.0 * f64::from(reach) {
            None
        } else if distance <= f64::from(reach) / ON_CREASE {
            Some(0)
        } else {
            Some(if rows[1][0] > 0.0 { 1 } else { -1 })
        }
    };
    let mut half_edges = HashMap::new();
    let mut crossings = HashMap::<Corner, [f32; 3]>::new();
    let mut edges = mesh
        .faces()
        .flat_map(|face| mesh.face_loop(face).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    edges.sort_unstable();
    for edge in edges {
        let (Some(from), Some(to)) = (mesh.from_vertex(edge), mesh.to_vertex(edge)) else {
            continue;
        };
        half_edges.insert((from, to), edge);
        let interior = mesh
            .twin(edge)
            .and_then(|twin| mesh.face(twin))
            .is_some_and(|face| face != FaceId::OUTSIDE);
        let opposite = matches!(
            (side(from), side(to)),
            (Some(1), Some(-1)) | (Some(-1), Some(1))
        );
        if from > to || !interior || !opposite {
            continue;
        }
        let [tie_from, tie_to] = [from, to].map(|vertex| split(rows[&vertex][1]));
        // Both ends must see the same tie: its gradients agree.
        let unit = |gradient: [f64; 3]| gradient.map(|g| g / norm(gradient));
        if dot(unit(tie_from.1), unit(tie_to.1)) < 0.5 {
            continue;
        }
        let along = tie_from.0 / (tie_from.0 - tie_to.0);
        let ends = [from, to].map(|vertex| *mesh.vertex_position(vertex).expect("live"));
        #[expect(clippy::cast_possible_truncation, reason = "positions are f32")]
        let start = lerp(ends[0], ends[1], along as f32);
        if let Some(point) = crease_point(field, start, reach, run)? {
            crossings.insert(Corner::Crossing(from, to), point);
        }
    }
    if crossings.is_empty() {
        return Ok(0);
    }

    // Plan every divided face first; a face whose plan would turn a
    // triangle over keeps its bridges, which may change its neighbor's plan.
    let faces_of = faces_of(mesh);
    let mut plans = HashMap::<FaceId, Vec<[Corner; 2]>>::new();
    loop {
        let mut touched = crossings
            .keys()
            .flat_map(|corner| {
                let Corner::Crossing(from, _) = *corner else {
                    unreachable!("crossings are edge crossings")
                };
                faces_of.get(&from).into_iter().flatten().copied()
            })
            .collect::<Vec<_>>();
        touched.sort_unstable();
        touched.dedup();
        plans.clear();
        let mut refused = Vec::new();
        for face in touched {
            let polygon = mesh
                .face_loop(face)
                .flat_map(|edge| {
                    let from = mesh.from_vertex(edge).expect("live");
                    let to = mesh.to_vertex(edge).expect("live");
                    let crossing = [Corner::Crossing(from, to), Corner::Crossing(to, from)]
                        .into_iter()
                        .find(|corner| crossings.contains_key(corner));
                    core::iter::once(Corner::Vertex(from)).chain(crossing)
                })
                .collect::<Vec<_>>();
            if polygon.len() == 3 {
                continue;
            }
            let position = |corner: &Corner| match corner {
                Corner::Vertex(vertex) => *mesh.vertex_position(*vertex).expect("live"),
                Corner::Crossing(..) => crossings[corner],
            };
            let class = |corner: &Corner| match corner {
                Corner::Vertex(vertex) => side(*vertex),
                Corner::Crossing(..) => Some(0),
            };
            let Some((diagonals, triangles)) = divide(&polygon, class, position) else {
                refused.extend(
                    polygon
                        .into_iter()
                        .filter(|c| matches!(c, Corner::Crossing(..))),
                );
                continue;
            };
            // Each triangle now lies on one piece, so it must face the way
            // the field rises on that piece, not the way the bridge did:
            // at its center, or at its corners off the crease, which
            // sampled their own piece. Near the crease, a thin triangle's
            // center may sample the other piece.
            let points = triangles
                .iter()
                .map(|triangle| triangle.map(|corner| position(&corner)))
                .collect::<Vec<_>>();
            let centers = points
                .iter()
                .map(|p| core::array::from_fn(|i| (p[0][i] + p[1][i] + p[2][i]) / 3.0))
                .collect::<Vec<[f32; 3]>>();
            let mut gradients = alloc::vec![[0.0_f32; 4]; centers.len()];
            field.eval_gradients(&centers, &mut gradients);
            run.check()?;
            let agrees = triangles.iter().zip(&points).zip(&gradients).all(
                |((triangle, &points), center)| {
                    let normal = newell_normal(&points);
                    let at_center = dot(normal, split(*center).1) > 0.0;
                    let at_corners = triangle.iter().all(|corner| match corner {
                        Corner::Vertex(vertex) if side(*vertex) != Some(0) => rows
                            .get(vertex)
                            .is_none_or(|rows| dot(normal, split(rows[0]).1) > 0.0),
                        _ => true,
                    });
                    triangle_is_nondegenerate(points) && (at_center || at_corners)
                },
            );
            if agrees {
                plans.insert(face, diagonals);
            } else {
                refused.extend(
                    polygon
                        .into_iter()
                        .filter(|c| matches!(c, Corner::Crossing(..))),
                );
            }
        }
        if refused.is_empty() {
            break;
        }
        for corner in refused {
            crossings.remove(&corner);
        }
        if crossings.is_empty() {
            return Ok(0);
        }
    }

    let mut session = mesh.edit();
    let mut inserted = HashMap::new();
    let mut ordered = crossings.into_iter().collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|(corner, _)| match corner {
        Corner::Crossing(from, to) => (*from, *to),
        Corner::Vertex(_) => unreachable!("crossings are edge crossings"),
    });
    for (corner, point) in ordered {
        let Corner::Crossing(from, to) = corner else {
            unreachable!("crossings are edge crossings")
        };
        let vertex = op::split_edge(
            &mut session,
            half_edges[&(from, to)],
            &PropagatePolicy::default(),
        )
        .expect("a live interior edge splits");
        op::set_vertex_position(&mut session, vertex, point).expect("the new vertex is live");
        inserted.insert(corner, vertex);
    }
    let vertex_of = |corner: Corner| match corner {
        Corner::Vertex(vertex) => vertex,
        crossing => inserted[&crossing],
    };
    let mut plans = plans.into_iter().collect::<Vec<_>>();
    plans.sort_unstable_by_key(|(face, _)| *face);
    for (face, diagonals) in plans {
        let mut pieces = alloc::vec![face];
        for [a, b] in diagonals {
            let (a, b) = (vertex_of(a), vertex_of(b));
            let found = pieces.iter().find_map(|&piece| {
                let mesh = session.mesh();
                let corner = |vertex| {
                    mesh.face_loop(piece)
                        .find(|&edge| mesh.to_vertex(edge) == Some(vertex))
                };
                Some((corner(a)?, corner(b)?))
            });
            let Some((corner_a, corner_b)) = found else {
                continue;
            };
            if let Ok(piece) = op::split_face(
                &mut session,
                corner_a,
                corner_b,
                &PropagatePolicy::default(),
            ) {
                pieces.push(piece);
            }
        }
    }
    let _: () = session.finish();
    Ok(inserted.len())
}

/// Diagonals dividing `polygon` into triangles, each on one side of the
/// crease, and those triangles. `class` gives each corner's side: `0` on the
/// crease, `±1` off it, `None` unknown. Consecutive crease corners are
/// joined first; what remains is cut on its shorter diagonals, never
/// joining opposite sides. `None` for more than two crease corners.
#[expect(clippy::type_complexity, reason = "a private plan")]
fn divide(
    polygon: &[Corner],
    class: impl Fn(&Corner) -> Option<i8>,
    position: impl Fn(&Corner) -> [f32; 3],
) -> Option<(Vec<[Corner; 2]>, Vec<[Corner; 3]>)> {
    let marks = (0..polygon.len())
        .filter(|&i| class(&polygon[i]) == Some(0))
        .collect::<Vec<_>>();
    let mut parts = match marks.as_slice() {
        [first, second] if second - first > 1 && second - first < polygon.len() - 1 => {
            let (first, second) = (*first, *second);
            alloc::vec![
                polygon[first..=second].to_vec(),
                polygon[second..]
                    .iter()
                    .chain(&polygon[..=first])
                    .copied()
                    .collect(),
            ]
        }
        [_] | [_, _] => alloc::vec![polygon.to_vec()],
        _ => return None,
    };
    let mut diagonals = Vec::new();
    if parts.len() == 2 {
        diagonals.push([parts[0][0], *parts[0].last().expect("non-empty")]);
    }
    let opposite =
        |a: &Corner, b: &Corner| matches!((class(a), class(b)), (Some(x), Some(y)) if x * y < 0);
    let length = |a: &Corner, b: &Corner| {
        let (p, q) = (position(a), position(b));
        (0..3).map(|i| (p[i] - q[i]) * (p[i] - q[i])).sum::<f32>()
    };
    let mut triangles = Vec::new();
    while let Some(part) = parts.pop() {
        let n = part.len();
        if n == 3 {
            triangles.push([part[0], part[1], part[2]]);
            continue;
        }
        // The shortest diagonal that joins no opposite sides cuts off one
        // corner's triangle; the rest is cut again.
        let (cut, _) = (0..n)
            .filter(|&i| !opposite(&part[(i + n - 1) % n], &part[(i + 1) % n]))
            .map(|i| (i, length(&part[(i + n - 1) % n], &part[(i + 1) % n])))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let (before, after) = (part[(cut + n - 1) % n], part[(cut + 1) % n]);
        diagonals.push([before, after]);
        triangles.push([before, part[cut], after]);
        parts.push((0..n).filter(|&i| i != cut).map(|i| part[i]).collect());
    }
    Some((diagonals, triangles))
}

/// The point where the crease crosses the plane through `start` across it,
/// by Newton steps on the field, the tie and that plane together, or `None`
/// when they do not meet within `reach`.
///
/// Where the pieces meet at a grazing angle, the crease's place along
/// itself is poorly determined, and steps free to slide along it carry
/// neighboring starts to one point. Holding each to its own cross-section
/// keeps them in order along the crease.
fn crease_point<F: ScalarField>(
    field: &F,
    start: [f32; 3],
    reach: f32,
    run: &RunContext,
) -> Result<Option<[f32; 3]>, DualContourError> {
    let tolerance = 1.0e-4 * f64::from(reach);
    let origin = start.map(f64::from);
    let mut point = origin;
    let mut along = None;
    for _ in 0..NEWTON_STEPS {
        #[expect(clippy::cast_possible_truncation, reason = "positions are f32")]
        let at = point.map(|c| c as f32);
        let rows = field.eval_crease(at);
        run.check()?;
        let Some([surface, tie]) = rows else {
            return Ok(None);
        };
        let (f, a) = split(surface);
        let (t, b) = split(tie);
        // The crease runs along both surfaces: across both gradients.
        let tangent = *along.get_or_insert(cross(a, b));
        let offset = core::array::from_fn::<_, 3, _>(|i| point[i] - origin[i]);
        let drift = dot(tangent, offset);
        if f.abs() <= tolerance * norm(a) && t.abs() <= tolerance * norm(b) {
            return Ok((norm(offset) <= f64::from(reach)).then_some(at));
        }
        let Some(step) = solve([a, b, tangent], [-f, -t, -drift]) else {
            return Ok(None);
        };
        point = core::array::from_fn(|i| point[i] + step[i]);
    }
    Ok(None)
}

/// The `x` with `rows · x = rhs`, or `None` when the rows are nearly
/// dependent: the pieces are tangent, with no transverse crease.
fn solve(rows: [[f64; 3]; 3], rhs: [f64; 3]) -> Option<[f64; 3]> {
    let determinant = dot(rows[0], cross(rows[1], rows[2]));
    let scale = norm(rows[0]) * norm(rows[1]) * norm(rows[2]);
    if determinant.is_nan() || determinant.abs() <= 1.0e-6 * scale {
        return None;
    }
    // Cramer's rule, by columns of the inverse.
    let inverse = [
        cross(rows[1], rows[2]),
        cross(rows[2], rows[0]),
        cross(rows[0], rows[1]),
    ];
    Some(core::array::from_fn(|i| {
        (0..3).map(|k| inverse[k][i] * rhs[k]).sum::<f64>() / determinant
    }))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    exedra_math::cross(a, b)
}

/// The least step along both gradients that zeroes the field and the tie
/// to first order, or `None` where the pieces are tangent: no transverse
/// crease.
fn step(surface: [f32; 4], tie: [f32; 4]) -> Option<[f64; 3]> {
    let (f, a) = split(surface);
    let (t, b) = split(tie);
    let (aa, bb, ab) = (dot(a, a), dot(b, b), dot(a, b));
    let determinant = aa * bb - ab * ab;
    if determinant.is_nan() || determinant <= 1.0e-6 * aa * bb {
        return None;
    }
    let along_a = (-f * bb + t * ab) / determinant;
    let along_b = (-t * aa + f * ab) / determinant;
    Some(core::array::from_fn(|i| along_a * a[i] + along_b * b[i]))
}

fn faces_of(mesh: &Mesh) -> HashMap<VertexId, Vec<FaceId>> {
    let mut faces_of = HashMap::<VertexId, Vec<FaceId>>::new();
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            if let Some(vertex) = mesh.to_vertex(corner) {
                faces_of.entry(vertex).or_default().push(face);
            }
        }
    }
    faces_of
}

/// The normal of `face` with vertices at their `placed` positions.
fn face_normal(mesh: &Mesh, face: FaceId, placed: &HashMap<VertexId, [f32; 3]>) -> [f64; 3] {
    let points = mesh
        .face_loop(face)
        .filter_map(|corner| {
            let vertex = mesh.to_vertex(corner)?;
            placed
                .get(&vertex)
                .or_else(|| mesh.vertex_position(vertex))
                .copied()
        })
        .collect::<Vec<_>>();
    newell_normal(&points)
}

fn split(row: [f32; 4]) -> (f64, [f64; 3]) {
    (f64::from(row[0]), [row[1], row[2], row[3]].map(f64::from))
}

fn norm(v: [f64; 3]) -> f64 {
    exedra_math::Real::sqrt(dot(v, v))
}
