// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_compile::{CompilePolicy, PartCompiler};
use exedra_scene::{SceneEvaluator, SceneSnapshot, composition::*, schemas};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
};

pub(crate) const ROOT_NAME: &str = "scene.usda";
pub(crate) const ROOT_BYTES: &[u8] = br#"#usda 1.0
(
    defaultPrim = "World"
    metersPerUnit = 0.001
    upAxis = "Z"
    subLayers = [@overrides.usda@, @base.usda@, @generated.usda@]
    fallbackPrimTypes = {
        token[] ExampleNativePanel = ["GenerativeProcedural"]
    }
)
"#;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Layers {
    pub(crate) root: LayerId,
    pub(crate) base: LayerId,
    pub(crate) generated: LayerId,
}

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug)]
pub(crate) struct SourceSummary {
    pub(crate) volume: f64,
    pub(crate) area: f64,
    pub(crate) bounds: [[f64; 3]; 2],
    pub(crate) fingerprint: String,
}

/// Exedra owns geometry and transfer caches; the official binding owns the stage.
#[derive(Debug)]
pub(crate) struct State {
    identity: StoreIdentity,
    pub(crate) layers: Layers,
    pub(crate) cursor: ChangeCursor,
    pub(crate) evaluator: SceneEvaluator,
    pub(crate) compiler: PartCompiler,
    pub(crate) policy: CompilePolicy,
    pub(crate) snapshot: Option<SceneSnapshot>,
    pub(crate) placements: Vec<PathId>,
    pub(crate) source_revision: u64,
    pub(crate) source_summary: Option<SourceSummary>,
    pub(crate) last_mesh: Option<[TypedArray; 5]>,
    pub(crate) pending_placements: Vec<PathId>,
    pub(crate) placement_set: BTreeSet<PathId>,
    pub(crate) reset_instances: bool,
    pub(crate) last_transforms: BTreeMap<PathId, [[f64; 4]; 4]>,
    pub(crate) display_abs: [f64; 3],
    pub(crate) verified_instances: Option<(u64, u64)>,
}

pub(crate) fn attribute(
    store: &mut InMemoryStore,
    transaction: &mut Transaction,
    target: &EditTarget,
    prim: PathId,
    name: &str,
    ty: &str,
    value: Value,
) {
    let mut spec = PropertySpec::typed_attribute(PropertyType::parse(ty).expect("known USD type"))
        .with_default(value);
    if name == "xformOpOrder" {
        spec = spec.uniform();
    }
    transaction.create_property(
        target.property(PropertyPath::new(prim, store.tokens().intern(name))),
        spec,
    );
}

impl State {
    pub(crate) fn new(stage: &mut layerstack_wasm::Stage, instances: u32) -> Result<Self> {
        if !(1..=10_000).contains(&instances) {
            return Err("instance count must be between 1 and 10000".into());
        }
        if !stage.document().stage().options().with_provenance {
            return Err(crate::error::refusal(
                "scene.provenance",
                "open the official stage with provenance: true for native correspondence",
            ));
        }
        let document = stage.document_mut();
        if document.storage().bytes(ROOT_NAME) != Some(ROOT_BYTES)
            || document.store().layer_ids().len() != 1
        {
            return Err(crate::error::refusal(
                "scene.seed",
                "create the scene on a fresh official stage opened from scene_root()",
            ));
        }
        for name in ["base.usda", "overrides.usda", "generated.usda"] {
            document.storage_mut().supply(name, b"#usda 1.0\n".to_vec());
        }
        document.load_requested_assets()?;
        let find = |name| {
            document
                .store()
                .layer_ids()
                .into_iter()
                .find(|&id| document.identifier(id) == Some(name))
                .ok_or("missing named scene layer")
        };
        let layers = Layers {
            root: document.root_layer(),
            base: find("base.usda")?,
            generated: find("generated.usda")?,
        };
        let (store, live) = document.parts_mut();
        let world = store.path("/World");
        let asset = store.path("/World/PanelAsset");
        let placement_root = store.path("/World/Placements");
        let base = EditTarget::for_layer(layers.base);
        let root_target = EditTarget::for_layer(layers.root);
        let mut transaction = Transaction::new();
        for path in [world, placement_root] {
            transaction.create_prim(
                if path == world {
                    root_target.prim(path)
                } else {
                    base.prim(path)
                },
                Specifier::Def,
                Some(store.tokens().intern("Xform")),
            );
        }
        transaction.create_prim(
            base.prim(asset),
            Specifier::Def,
            Some(store.tokens().intern(native_scene::PANEL_SCHEMA)),
        );
        let system = store.tokens().intern(native_scene::PANEL_SYSTEM);
        for (name, ty, value) in [
            ("proceduralSystem", "token", Value::Token(system)),
            (native_scene::PANEL_WIDTH, "double", Value::Double(120.0)),
            (native_scene::PANEL_DEPTH, "double", Value::Double(5.0)),
        ] {
            attribute(store, &mut transaction, &base, asset, name, ty, value);
        }
        let translate = store.tokens().intern("xformOp:translate");
        let mut placements = Vec::with_capacity(instances as usize);
        let floor = instances.isqrt();
        let columns = if floor * floor < instances {
            floor + 1
        } else {
            floor
        };
        for index in 0..instances {
            let path = store.path(&format!("/World/Placements/P{index:04}"));
            placements.push(path);
            transaction.create_prim(base.prim(path), Specifier::Def, None);
            transaction.set_instanceable(base.prim(path), true);
            transaction.add_reference(
                base.prim(path),
                // An internal reference names its authoring layer. Composition
                // still resolves the target in the complete stage layer stack.
                Reference::new(layers.base, asset),
                ListPosition::BackOfPrepend,
            );
            attribute(
                store,
                &mut transaction,
                &base,
                path,
                "xformOp:translate",
                "double3",
                Value::Vec3d([
                    160.0 + f64::from(index % columns) * 140.0,
                    f64::from(index / columns) * 100.0,
                    0.0,
                ]),
            );
            attribute(
                store,
                &mut transaction,
                &base,
                path,
                "xformOpOrder",
                "token[]",
                Value::TypedArray(TypedArray::Token(vec![translate].into())),
            );
        }
        live.apply(store, &transaction)?;
        let mut evaluator =
            SceneEvaluator::new(store, world, EditTarget::for_layer(layers.generated));
        native_scene::register_panel_generator(&mut evaluator)?;
        let mut policy = CompilePolicy::default();
        policy.native.tolerance = 0.2;
        Ok(Self {
            identity: store.identity(),
            layers,
            cursor: live.change_cursor(),
            evaluator,
            compiler: PartCompiler::with_capacity(16),
            policy,
            snapshot: None,
            pending_placements: placements.clone(),
            placement_set: placements.iter().copied().collect(),
            reset_instances: true,
            last_transforms: BTreeMap::new(),
            display_abs: [0.0; 3],
            verified_instances: None,
            placements,
            source_revision: 0,
            source_summary: None,
            last_mesh: None,
        })
    }
}

/// A temporary borrow of the official Stage; no handles or pointers are retained.
#[derive(Debug)]
pub(crate) struct Backend<'a> {
    pub(crate) store: &'a mut InMemoryStore,
    pub(crate) live: &'a mut LiveStage,
    pub(crate) state: &'a mut State,
}

impl State {
    pub(crate) fn borrow<'a>(
        &'a mut self,
        stage: &'a mut layerstack_wasm::Stage,
    ) -> Result<Backend<'a>> {
        if stage.document().store().identity() != self.identity {
            return Err(crate::error::refusal(
                "scene.stage_affinity",
                "this geometry kernel belongs to a different official stage",
            ));
        }
        stage.document_mut().synchronize();
        let (store, live) = stage.document_mut().parts_mut();
        let latest = live.change_cursor();
        match live.changes_since(&mut self.cursor) {
            Ok(changes) => {
                for changed in changes {
                    refresh_placements(
                        self,
                        store,
                        live,
                        changed,
                        !changed.created.is_empty()
                            || !changed.removed.is_empty()
                            || !changed.resynced.is_empty(),
                    );
                }
            }
            Err(_) => {
                self.cursor = latest;
                refresh_placements(self, store, live, &Changes::default(), true);
                self.pending_placements.clone_from(&self.placements);
            }
        }
        Ok(Backend {
            store,
            live,
            state: self,
        })
    }
}

fn refresh_placements(
    b: &mut State,
    store: &InMemoryStore,
    live: &LiveStage,
    changed: &Changes,
    membership: bool,
) {
    if membership {
        let before = std::mem::take(&mut b.placements);
        // Discover actual native instance roots under the caller's placement namespace.
        if let Ok(root) = Path::parse_absolute("/World/Placements", store.tokens())
            && let Some(root) = store.paths().lookup(&root)
        {
            b.placements = live
                .stage()
                .traverse(root)
                .filter(|&p| {
                    live.stage()
                        .view(store)
                        .prim(p)
                        .is_some_and(|p| p.is_instance())
                })
                .collect();
        }
        if before != b.placements {
            b.placement_set = b.placements.iter().copied().collect();
            b.pending_placements.clone_from(&b.placements);
            b.reset_instances = true;
            return;
        }
    }
    let transform_info = changed.changed_info_only.iter().filter(|&&path| {
        changed.properties_for(path).is_none_or(|fields| {
            fields
                .iter()
                .any(|field| schemas::is_transform_property(store.tokens().resolve(field.name)))
        })
    });
    for &path in transform_info
        .chain(&changed.resynced)
        .chain(&changed.created)
        .chain(&changed.removed)
    {
        if b.placement_set.contains(&path) {
            if !b.pending_placements.contains(&path) {
                b.pending_placements.push(path);
            }
            continue;
        }
        let parent = store.paths().display(path, store.tokens());
        // Edits outside the placement namespace cannot alter its transforms.
        if parent != "/" && parent != "/World" && !parent.starts_with("/World/Placements") {
            continue;
        }
        for &placement in &b.placements {
            let spelling = store.paths().display(placement, store.tokens());
            if (parent == "/"
                || spelling
                    .strip_prefix(&parent)
                    .is_some_and(|rest| rest.starts_with('/')))
                && !b.pending_placements.contains(&placement)
            {
                b.pending_placements.push(placement);
            }
        }
    }
}
