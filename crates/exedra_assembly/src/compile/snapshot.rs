// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{CompiledParts, PartFingerprint, PolicyFingerprint, part_fingerprint};
use crate::{Assembly, PartId};

/// Compiled geometry does not correspond to an assembly's current part records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompilationMismatch {
    /// The assembly and compilation have different part counts.
    PartCount {
        /// Number of parts registered in the assembly.
        expected: usize,
        /// Number of compiled entries supplied.
        actual: usize,
    },
    /// A local handle refers to different source geometry.
    PartContent {
        /// The mismatched assembly-local handle.
        part: PartId,
        /// Fingerprint of the current assembly part source.
        expected: PartFingerprint,
        /// Fingerprint of the supplied compiled entry.
        actual: PartFingerprint,
    },
}

impl core::fmt::Display for CompilationMismatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PartCount { expected, actual } => write!(
                f,
                "assembly has {expected} parts but compilation has {actual}"
            ),
            Self::PartContent { part, .. } => {
                write!(f, "compiled content does not match assembly part {part:?}")
            }
        }
    }
}

impl core::error::Error for CompilationMismatch {}

impl CompiledParts {
    /// Identity of the evaluation and extraction policy used for this snapshot.
    #[must_use]
    pub fn policy_fingerprint(&self) -> PolicyFingerprint {
        self.policy
    }

    /// Checks that every compiled entry corresponds to the assembly's part source.
    ///
    /// Placements, frame instances, keys, metadata and material bindings may change
    /// without invalidating geometry. Reordered handles or replaced source content
    /// require a matching compilation. The supplied snapshot determines the policy;
    /// callers can compare [`Self::policy_fingerprint`] with their intended policy.
    ///
    /// This recomputes source content fingerprints, including baked mesh attributes.
    /// It does not tessellate or measure placed bounds. Use it at snapshot/export
    /// boundaries; reading local poses directly needs no geometry validation walk.
    /// Geometry reports remain accessible through [`Self::report`], including reports
    /// for partial evaluations. Matching content alone does not certify completeness.
    ///
    /// # Errors
    ///
    /// Reports a part-count mismatch first, then the first mismatched part source.
    pub fn validate_for(&self, assembly: &Assembly) -> Result<(), CompilationMismatch> {
        if assembly.parts().len() != self.parts.len() {
            return Err(CompilationMismatch::PartCount {
                expected: assembly.parts().len(),
                actual: self.parts.len(),
            });
        }
        for (index, (def, compiled)) in assembly.parts().iter().zip(&self.parts).enumerate() {
            let expected = part_fingerprint(def.source());
            if expected != compiled.fingerprint {
                return Err(CompilationMismatch::PartContent {
                    part: PartId(crate::len_u32(index)),
                    expected,
                    actual: compiled.fingerprint,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompileError, CompilePolicy, CompiledPart, PartCompiler};
    use alloc::format;
    use alloc::sync::Arc;
    use exedra_constructive::ir::{NodeKind, Placement3, PrimitiveSpec, RecipeBuilder};

    fn assembly(sizes: &[f64]) -> Assembly {
        let mut assembly = Assembly::new();
        for (i, &size) in sizes.iter().enumerate() {
            let mut recipe = RecipeBuilder::new();
            let slot = recipe.material_slot("surface");
            let root = recipe
                .with_material(slot)
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box { size: [size; 3] },
                    placement: Placement3::IDENTITY,
                })
                .unwrap();
            assembly
                .add_recipe_part(&format!("part-{i}"), recipe.finish(root).unwrap())
                .unwrap();
        }
        assembly
    }

    #[test]
    fn correspondence_uses_source_content_not_pose_materials_or_reused_handles() {
        let mut source = assembly(&[1.0, 2.0]);
        let policy = CompilePolicy::default();
        let compiled = PartCompiler::new().compile_parts(&source, &policy).unwrap();
        assert_eq!(compiled.validate_for(&source), Ok(()));
        assert_eq!(
            compiled.policy_fingerprint(),
            super::super::policy_fingerprint(&policy)
        );
        let frame = source
            .add_frame(None, "frame", Placement3::translate(5.0, 0.0, 0.0))
            .unwrap();
        let placed = source
            .add_instance(Some(frame), "placed", PartId(0), Placement3::IDENTITY)
            .unwrap();
        source
            .bind_material(placed, "surface", "caller:paint")
            .unwrap();
        assert_eq!(compiled.validate_for(&source), Ok(()));
        assert!(matches!(
            compiled.validate_for(&assembly(&[2.0, 1.0])),
            Err(CompilationMismatch::PartContent {
                part: PartId(0),
                ..
            })
        ));
        assert!(matches!(
            compiled.validate_for(&assembly(&[1.0, 3.0])),
            Err(CompilationMismatch::PartContent {
                part: PartId(1),
                ..
            })
        ));
        assert_eq!(
            compiled.validate_for(&assembly(&[1.0])),
            Err(CompilationMismatch::PartCount {
                expected: 1,
                actual: 2
            })
        );
        assert_eq!(
            compiled.validate_for(&assembly(&[1.0, 2.0, 3.0])),
            Err(CompilationMismatch::PartCount {
                expected: 3,
                actual: 2
            })
        );
    }

    #[test]
    fn invalidation_tracks_cache_hits_and_the_latest_successful_handle_assignment() {
        let policy = CompilePolicy::default();
        let mut compiler = PartCompiler::new();
        compiler
            .compile_parts(&assembly(&[1.0, 1.0]), &policy)
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 1);
        compiler.mark_part_changed(PartId(1)); // This alias was originally a hit.
        compiler
            .compile_parts(&assembly(&[1.0, 1.0]), &policy)
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);
        assert_eq!(compiler.counters().cache_evictions, 1);

        let resized = assembly(&[2.0, 1.0]);
        let before = compiler.compile_parts(&resized, &policy).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 3);
        compiler.mark_part_changed(PartId(0));
        let after = compiler.compile_parts(&resized, &policy).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 4);
        assert!(
            !Arc::ptr_eq(
                before.part(PartId(0)).unwrap(),
                after.part(PartId(0)).unwrap()
            ),
            "the current content associated with the handle is evicted"
        );
        assert!(
            Arc::ptr_eq(
                before.part(PartId(1)).unwrap(),
                after.part(PartId(1)).unwrap()
            ),
            "reused IDs must not evict their historical content instead"
        );
    }

    #[test]
    fn releasing_cache_retains_counters_and_old_snapshot_geometry_and_reports() {
        let source = assembly(&[1.0, 2.0]);
        let policy = CompilePolicy::default();
        let mut compiler = PartCompiler::new();
        let before = compiler.compile_parts(&source, &policy).unwrap();
        let report = before.report(PartId(0)).unwrap().clone();
        compiler.mark_part_changed(PartId(0));
        compiler.clear_cache();
        assert_eq!(compiler.cached_entries(), 0);
        assert_eq!(compiler.counters().cache_evictions, 2);
        assert_eq!(compiler.counters().parts_compiled, 2);
        assert_eq!(before.validate_for(&source), Ok(()));
        assert_eq!(before.part(PartId(0)).unwrap().triangle_count(), 12);
        assert_eq!(before.report(PartId(0)), Some(&report));
        let after = compiler.compile_parts(&source, &policy).unwrap();
        assert_eq!(
            compiler.counters().cache_evictions,
            2,
            "pending invalidations were cleared"
        );
        assert_eq!(compiler.counters().parts_compiled, 4);
        assert!(
            !Arc::ptr_eq(
                before.part(PartId(0)).unwrap(),
                after.part(PartId(0)).unwrap()
            ),
            "released geometry is recompiled into a new allocation"
        );
        assert_eq!(before.report(PartId(0)), after.report(PartId(0)));
    }

    #[test]
    fn compiled_ownership_supports_thread_transfer_and_shared_reads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompiledParts>();
        assert_send_sync::<PartCompiler>();
        assert_send_sync::<CompiledPart>();
        assert_send_sync::<CompileError>();
    }

    #[cfg(all(feature = "std", not(target_arch = "wasm32")))]
    #[test]
    fn populated_compiler_and_snapshots_cross_threads_without_copying_geometry_or_reports() {
        use exedra_constructive::evaluate::Severity;

        let source = assembly(&[1.0, 1.0]);
        let policy = CompilePolicy::default();
        let mut compiler = PartCompiler::new();
        let first = compiler.compile_parts(&source, &policy).unwrap();
        let (first, published) = std::thread::spawn(move || {
            let next = compiler
                .compile_parts(&assembly(&[1.0, 1.0]), &policy)
                .unwrap();
            assert_eq!(compiler.counters().parts_compiled, 1);
            let published = next.clone();
            compiler.clear_cache();
            drop(compiler);
            (first, published)
        })
        .join()
        .expect("geometry worker completed");
        assert!(Arc::ptr_eq(
            first.part(PartId(0)).unwrap(),
            published.part(PartId(1)).unwrap()
        ));
        assert!(core::ptr::eq(
            first.report(PartId(0)).unwrap(),
            published.report(PartId(1)).unwrap()
        ));
        drop(first);
        assert_eq!(published.validate_for(&source), Ok(()));
        assert_eq!(published.part(PartId(0)).unwrap().triangle_count(), 12);
        assert!(
            published
                .report(PartId(0))
                .unwrap()
                .clean_at(Severity::Error)
        );
    }
}
