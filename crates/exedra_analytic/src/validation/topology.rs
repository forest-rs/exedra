// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{BoundaryShell, BoundaryValidationError as Error};
use crate::body::{Body, Coedge, EdgeId, FaceId};
use alloc::{vec, vec::Vec};

struct Sets(Vec<usize>);
impl Sets {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    fn root(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }
    fn join(&mut self, a: usize, b: usize) {
        let a = self.root(a);
        let b = self.root(b);
        self.0[a] = b;
    }
}
fn port(coedge: Coedge, end: bool) -> usize {
    2 * coedge.edge.index() + usize::from(coedge.reversed != end)
}

pub(super) fn check<F, E>(body: &Body<F, E>) -> Result<Vec<BoundaryShell>, Error> {
    if body.faces().is_empty() {
        return Err(Error::Empty);
    }
    let mut owners = vec![0_usize; body.loops().len()];
    let mut uses = vec![[0_usize; 2]; body.edges().len()];
    let mut edge_face = vec![None; body.edges().len()];
    let mut face_sets = Sets::new(body.faces().len());
    let mut links = Sets::new(2 * body.edges().len());
    for (f, face) in body.faces().iter().enumerate() {
        for &loop_id in &face.loops {
            owners[loop_id.index()] += 1;
            if owners[loop_id.index()] > 1 {
                return Err(Error::LoopOwnership {
                    loop_id,
                    owners: owners[loop_id.index()],
                });
            }
            let ring = &body.loops()[loop_id.index()];
            let mut regular = ring
                .coedges
                .iter()
                .copied()
                .filter(|co| !body.edges()[co.edge.index()].is_collapsed());
            let Some(first) = regular.next() else {
                return Err(Error::CollapsedLoop {
                    face: FaceId(crate::usize_to_u32(f)),
                    loop_id,
                });
            };
            let mut previous = first;
            for co in regular {
                links.join(port(previous, true), port(co, false));
                previous = co;
            }
            links.join(port(previous, true), port(first, false));
            for co in &ring.coedges {
                uses[co.edge.index()][usize::from(co.reversed)] += 1;
                if !body.edges()[co.edge.index()].is_collapsed() {
                    if let Some(other) = edge_face[co.edge.index()] {
                        face_sets.join(f, other);
                    } else {
                        edge_face[co.edge.index()] = Some(f);
                    }
                }
            }
        }
    }
    let mut first_port = vec![None; body.vertices().len()];
    for (i, (edge, uses)) in body.edges().iter().zip(&uses).enumerate() {
        if *uses == [0, 0] {
            continue;
        }
        let valid = if edge.is_collapsed() {
            uses[0] + uses[1] == 1
        } else {
            *uses == [1, 1]
        };
        if !valid {
            return Err(Error::EdgeIncidence {
                edge: EdgeId(crate::usize_to_u32(i)),
                uses: *uses,
            });
        }
        if edge.is_collapsed() {
            continue;
        }
        for (end, vertex) in [edge.start, edge.end].into_iter().enumerate() {
            let p = 2 * i + end;
            if let Some(first) = first_port[vertex.index()] {
                if links.root(first) != links.root(p) {
                    return Err(Error::DisconnectedVertexLink { vertex });
                }
            } else {
                first_port[vertex.index()] = Some(p);
            }
        }
    }
    let mut shell_for_root = vec![None; body.faces().len()];
    let mut shells: Vec<BoundaryShell> = Vec::new();
    for (f, face) in body.faces().iter().enumerate() {
        let root = face_sets.root(f);
        let shell = *shell_for_root[root].get_or_insert_with(|| {
            let i = shells.len();
            shells.push(BoundaryShell {
                faces: Vec::new(),
                vertices: 0,
                regular_edges: 0,
                euler_characteristic: 0,
            });
            i
        });
        shells[shell].faces.push(FaceId(crate::usize_to_u32(f)));
        // A face with h inner loops is a disk with h holes, not h+1 disks.
        shells[shell].euler_characteristic +=
            2 - i64::try_from(face.loops.len()).expect("u32 native table");
    }
    for face in edge_face.iter().copied().flatten() {
        let shell = shell_for_root[face_sets.root(face)].expect("used face root");
        shells[shell].regular_edges += 1;
        shells[shell].euler_characteristic -= 1;
    }
    for port in first_port.into_iter().flatten() {
        let face = edge_face[port / 2].expect("used vertex port");
        let shell = shell_for_root[face_sets.root(face)].expect("used face root");
        shells[shell].vertices += 1;
        shells[shell].euler_characteristic += 1;
    }
    Ok(shells)
}
