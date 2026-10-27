// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_compile::{CompilePolicy, PartCompiler};
use exedra_scene::{SceneEvaluator, SceneSnapshot, composition::*, register_schemas, schemas};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    sync::Arc,
};

pub(crate) const ROOT: LayerId = LayerId(1);
pub(crate) const BASE: LayerId = LayerId(2);
pub(crate) const OVERRIDES: LayerId = LayerId(3);
pub(crate) const GENERATED: LayerId = LayerId(4);

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug)]
pub(crate) struct SourceSummary {
    pub(crate) volume: f64,
    pub(crate) area: f64,
    pub(crate) bounds: [[f64; 3]; 2],
    pub(crate) fingerprint: String,
}

/// Application-owned stage; the adapter borrows this one authority at sync.
#[derive(Debug)]
pub(crate) struct Backend {
    pub(crate) store: InMemoryStore,
    pub(crate) live: LiveStage,
    pub(crate) evaluator: SceneEvaluator,
    pub(crate) compiler: PartCompiler,
    pub(crate) policy: CompilePolicy,
    pub(crate) snapshot: Option<SceneSnapshot>,
    pub(crate) placements: Vec<PathId>,
    pub(crate) undo: BTreeMap<u64, (Transaction, bool)>,
    pub(crate) next_receipt: u64,
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
        target.property(PropertyPath::new(prim, store.tokens.intern(name))),
        spec,
    );
}

impl Backend {
    pub(crate) fn new(instances: u32) -> Result<Self> {
        if !(1..=10_000).contains(&instances) {
            return Err("instance count must be between 1 and 10000".into());
        }
        let mut store = InMemoryStore::default();
        let mut registry = SchemaRegistry::builder();
        schemas::register(
            &mut registry,
            &[schemas::Domain::UsdGeom, schemas::Domain::UsdProc],
            &store.tokens,
        )?;
        register_schemas(&mut registry, &mut store.tokens);
        native_scene::register_panel_schema(&mut registry, &mut store.tokens);
        let registry = Arc::new(registry.build(&store.tokens));
        let world = store.path("/World");
        let asset = store.path("/World/PanelAsset");
        let placement_root = store.path("/World/Placements");
        let mut root = Layer::new(ROOT);
        *root.sublayers_mut() = vec![
            SublayerEntry::with_asset(OVERRIDES, "overrides.usda", LayerOffset::IDENTITY),
            SublayerEntry::with_asset(BASE, "base.usda", LayerOffset::IDENTITY),
            SublayerEntry::with_asset(GENERATED, "generated.usda", LayerOffset::IDENTITY),
        ];
        root.set_default_prim(Some(store.tokens.intern("World")));
        root.metadata_fields_mut().extend([
            FieldEntry {
                name: store.tokens.intern("metersPerUnit"),
                value: FieldValue::Value(Value::Double(0.001)),
            },
            FieldEntry {
                name: store.tokens.intern("upAxis"),
                value: FieldValue::Value(Value::Token(store.tokens.intern("Z"))),
            },
            FieldEntry {
                name: store.tokens.intern("fallbackPrimTypes"),
                value: FieldValue::Value(Value::Dictionary(vec![(
                    native_scene::PANEL_SCHEMA.into(),
                    Value::TypedArray(TypedArray::Token(
                        vec![store.tokens.intern("GenerativeProcedural")].into(),
                    )),
                )])),
            },
        ]);
        store.insert_layer(root);
        for id in [BASE, OVERRIDES, GENERATED] {
            store.insert_layer(Layer::new(id));
        }
        let mut live = LiveStage::compose(
            &mut store,
            ROOT,
            StageOptions {
                schemas: Some(registry),
                with_provenance: true,
                ..StageOptions::default()
            },
        );
        let base = EditTarget::for_layer(BASE);
        let root_target = EditTarget::for_layer(ROOT);
        let mut transaction = Transaction::new();
        for path in [world, placement_root] {
            transaction.create_prim(
                if path == world {
                    root_target.prim(path)
                } else {
                    base.prim(path)
                },
                Specifier::Def,
                Some(store.tokens.intern("Xform")),
            );
        }
        transaction.create_prim(
            base.prim(asset),
            Specifier::Def,
            Some(store.tokens.intern(native_scene::PANEL_SCHEMA)),
        );
        let system = store.tokens.intern(native_scene::PANEL_SYSTEM);
        for (name, ty, value) in [
            ("proceduralSystem", "token", Value::Token(system)),
            (native_scene::PANEL_WIDTH, "double", Value::Double(120.0)),
            (native_scene::PANEL_DEPTH, "double", Value::Double(5.0)),
        ] {
            attribute(&mut store, &mut transaction, &base, asset, name, ty, value);
        }
        let translate = store.tokens.intern("xformOp:translate");
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
                Reference::new(ROOT, asset),
                ListPosition::BackOfPrepend,
            );
            attribute(
                &mut store,
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
                &mut store,
                &mut transaction,
                &base,
                path,
                "xformOpOrder",
                "token[]",
                Value::TypedArray(TypedArray::Token(vec![translate].into())),
            );
        }
        live.apply(&mut store, &transaction)?;
        let mut evaluator = SceneEvaluator::new(&store, world, EditTarget::for_layer(GENERATED));
        native_scene::register_panel_generator(&mut evaluator)?;
        let mut policy = CompilePolicy::default();
        policy.native.tolerance = 0.2;
        Ok(Self {
            store,
            live,
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
            undo: BTreeMap::new(),
            next_receipt: 1,
            source_revision: 0,
            source_summary: None,
            last_mesh: None,
        })
    }
}
