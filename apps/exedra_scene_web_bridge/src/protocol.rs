// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Layerstack's bounded version-one native stage transport, in one application memory.

use crate::backend::{BASE, Backend, GENERATED, OVERRIDES, ROOT, Result};
use exedra_scene::{Time, composition::*, schemas};
use serde_json::{Value as Json, json};

#[derive(Debug)]
pub(crate) struct Refusal {
    pub(crate) code: &'static str,
    message: String,
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Refusal {}
pub(crate) fn refusal(
    code: &'static str,
    message: impl Into<String>,
) -> Box<dyn std::error::Error> {
    Box::new(Refusal {
        code,
        message: message.into(),
    })
}

pub(crate) fn text<'a>(value: &'a Json, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Json::as_str)
        .ok_or_else(|| refusal("request.field", format!("missing string field {key}")))
}
pub(crate) fn stage_id(realm: &str) -> String {
    format!("{realm}/stage")
}
pub(crate) fn store_id(realm: &str) -> String {
    format!("{realm}/store")
}
pub(crate) fn layer_handle(realm: &str, id: LayerId) -> String {
    format!("{realm}/layer/{}", id.0)
}
fn receipt_handle(realm: &str, id: u64) -> String {
    format!("{realm}/undo/{id}")
}
fn parse_handle(realm: &str, kind: &str, value: &str) -> Result<u64> {
    value
        .strip_prefix(&format!("{realm}/{kind}/"))
        .ok_or_else(|| refusal("handle.affinity", "foreign backend handle"))?
        .parse()
        .map_err(Into::into)
}
fn layer_id(b: &Backend, realm: &str, handle: &str) -> Result<LayerId> {
    let id = LayerId(parse_handle(realm, "layer", handle)?);
    if b.store.layer(id).is_none() {
        return Err("unknown layer handle".into());
    }
    Ok(id)
}
fn path(b: &mut Backend, value: &str) -> Result<PathId> {
    Ok(b.store
        .paths
        .intern(Path::parse_absolute(value, &b.store.tokens)?))
}
fn display(b: &Backend, path: PathId) -> String {
    b.store.paths.display(path, &b.store.tokens)
}
fn layer_name(id: LayerId) -> &'static str {
    match id {
        ROOT => "scene.usda",
        BASE => "base.usda",
        OVERRIDES => "overrides.usda",
        GENERATED => "generated.usda",
        _ => "layer.usda",
    }
}
fn changes(b: &Backend, value: &Changes) -> Json {
    let paths = |ids: &[PathId]| ids.iter().map(|&p| display(b, p)).collect::<Vec<_>>();
    json!({"created":paths(&value.created),"removed":paths(&value.removed),"resynced":paths(&value.resynced),"changedInfoOnly":paths(&value.changed_info_only)})
}
pub(crate) fn inventory(b: &Backend, realm: &str) -> Json {
    let mut schema_types = Vec::new();
    let mut schema_issues = Vec::new();
    if let Some(registry) = b.live.options().schemas.as_ref() {
        schema_types.extend(
            registry
                .schemas()
                .map(|s| b.store.tokens.resolve(s.name).to_owned()),
        );
        schema_issues.extend(registry.issues().iter().map(|i| format!("{i:?}")));
    }
    schema_types.sort();
    let layers = [ROOT,OVERRIDES,BASE,GENERATED].map(|id|json!({"id":layer_handle(realm,id),"name":layer_name(id),"muted":b.live.options().muted_layers.contains(&id)}));
    json!({"rootLayer":layer_handle(realm,ROOT),"layers":layers,
        "schemaTypes":schema_types,"schemaIssues":schema_issues,"compositionErrors":b.live.stage().composition_errors().iter().map(|e|format!("{e:?}")).collect::<Vec<_>>(),
        "metersPerUnit":0.001,"upAxis":"Z","capabilities":["inspect","query","children","transaction","undo","mute","layerText","dispose","exedra.scene.sync","exedra.scene.pack","exedra.scene.pick"]})
}

fn decode_value(b: &mut Backend, value: &Json) -> Result<Value> {
    let kind = text(value, "type")?;
    let raw = value.get("value").ok_or("missing typed value")?;
    let number = || {
        raw.as_f64()
            .filter(|v| v.is_finite())
            .ok_or("expected finite number")
    };
    Ok(match kind {
        "bool" => Value::Bool(raw.as_bool().ok_or("expected boolean")?),
        "int" => Value::Int(i32::try_from(
            raw.as_i64().ok_or("expected signed integer")?,
        )?),
        "uint" => Value::UInt(u32::try_from(
            raw.as_u64().ok_or("expected unsigned integer")?,
        )?),
        "int64" => Value::Int64(
            raw.as_str()
                .ok_or("int64 requires an exact decimal string")?
                .parse()?,
        ),
        "uint64" => Value::UInt64(
            raw.as_str()
                .ok_or("uint64 requires an exact decimal string")?
                .parse()?,
        ),
        "double" => Value::Double(number()?),
        "float" => {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the native float result is checked for representability"
            )]
            let value = number()? as f32;
            if !value.is_finite() {
                return Err("float value exceeds native range".into());
            }
            Value::Float(value)
        }
        "string" => Value::String(raw.as_str().ok_or("expected string")?.into()),
        "asset" => Value::Asset(raw.as_str().ok_or("expected asset path")?.into()),
        "token" => Value::Token(b.store.tokens.intern(raw.as_str().ok_or("expected token")?)),
        "double3" => {
            let values = raw
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or("expected three finite coordinates")?;
            let mut vector = [0.0; 3];
            for (out, value) in vector.iter_mut().zip(values) {
                *out = value
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or("expected finite coordinate")?;
            }
            Value::Vec3d(vector)
        }
        "token[]" => {
            let values = raw.as_array().ok_or("expected token array")?;
            if values.len() > 256 {
                return Err("token array exceeds request budget".into());
            }
            let tokens = values
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|s| b.store.tokens.intern(s))
                        .ok_or("expected token")
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Value::TypedArray(TypedArray::Token(tokens.into()))
        }
        _ => return Err(format!("unsupported native value encoding {kind}").into()),
    })
}
fn encode_value(b: &Backend, value: &Value) -> Option<Json> {
    let (kind, value) = match value {
        Value::Bool(v) => ("bool", json!(v)),
        Value::Int(v) => ("int", json!(v)),
        Value::UInt(v) => ("uint", json!(v)),
        Value::Int64(v) => ("int64", json!(v.to_string())),
        Value::UInt64(v) => ("uint64", json!(v.to_string())),
        Value::Float(v) if v.is_finite() => ("float", json!(v)),
        Value::Double(v) if v.is_finite() => ("double", json!(v)),
        Value::String(v) => ("string", json!(v.as_ref())),
        Value::Asset(v) => ("asset", json!(v.as_ref())),
        Value::Token(v) => ("token", json!(b.store.tokens.resolve(*v))),
        Value::Vec3d(v) if v.iter().all(|v| v.is_finite()) => ("double3", json!(v)),
        Value::TypedArray(TypedArray::Token(v)) => (
            "token[]",
            json!(
                v.iter()
                    .map(|t| b.store.tokens.resolve(*t))
                    .collect::<Vec<_>>()
            ),
        ),
        _ => return None,
    };
    Some(json!({"type":kind,"value":value}))
}
fn refresh_placements(b: &mut Backend, changed: &Changes, membership: bool) {
    if membership {
        let before = std::mem::take(&mut b.placements);
        // Discover actual native instance roots under the caller's placement namespace.
        if let Ok(root) = Path::parse_absolute("/World/Placements", &b.store.tokens)
            && let Some(root) = b.store.paths.lookup(&root)
        {
            b.placements = b
                .live
                .stage()
                .traverse(root)
                .filter(|&p| {
                    b.live
                        .stage()
                        .view(&b.store)
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
                .any(|field| schemas::is_transform_property(b.store.tokens.resolve(field.name)))
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
        let parent = display(b, path);
        // Edits outside the placement namespace cannot alter its transforms.
        if parent != "/" && parent != "/World" && !parent.starts_with("/World/Placements") {
            continue;
        }
        for &placement in &b.placements {
            let spelling = display(b, placement);
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
fn applied(
    b: &mut Backend,
    realm: &str,
    transaction: &Transaction,
    membership: bool,
) -> Result<Json> {
    let next = b
        .next_receipt
        .checked_add(1)
        .ok_or("undo receipt budget exhausted")?;
    if b.undo.len() >= 256 {
        return Err("release or consume old undo receipts before retaining more than256".into());
    }
    let applied = b.live.apply(&mut b.store, transaction)?;
    refresh_placements(b, &applied.changes, membership);
    let result = json!({"receipt":receipt_handle(realm,b.next_receipt),"changes":changes(b,&applied.changes)});
    b.undo.insert(b.next_receipt, (applied.inverse, membership));
    b.next_receipt = next;
    Ok(result)
}

pub(crate) fn dispatch(b: &mut Backend, realm: &str, operation: &Json) -> Result<Json> {
    Ok(match text(operation, "type")? {
        "describe" | "inspect" => inventory(b, realm),
        "query" => {
            let path = path(b, text(operation, "path")?)?;
            let prim = b
                .live
                .stage()
                .view(&b.store)
                .prim(path)
                .ok_or("prim does not exist")?;
            let names = operation
                .get("attributes")
                .and_then(Json::as_array)
                .ok_or("expected attribute names")?;
            if names.len() > 64 {
                return Err("attribute query exceeds64names".into());
            }
            let mut attributes = serde_json::Map::new();
            for name in names {
                let name = name.as_str().ok_or("expected attribute name")?;
                if let Some(attr) = prim.attribute(name) {
                    let value = attr.get(Time::Default).map(|v| v.value);
                    let encoded = value.as_ref().and_then(|v| encode_value(b, v));
                    let unsupported = value.is_some() && encoded.is_none();
                    attributes.insert(name.to_owned(),json!({"typeName":attr.type_name(),"variability":if attr.variability()==Variability::Uniform {"uniform"} else {"varying"},"value":encoded,"unsupportedEncoding":unsupported.then_some("outside scalar query vocabulary")}));
                }
            }
            let scene = schemas::Scene::new(b.live.stage(), &b.store);
            let mut transforms = schemas::XformCache::new(Time::Default);
            let world = transforms.local_to_world(&scene, path);
            if world
                .as_ref()
                .is_some_and(|m| m.iter().flatten().any(|v| !v.is_finite()))
            {
                return Err(refusal(
                    "query.numericLimit",
                    "composed world transform exceeds finite f64 range",
                ));
            }
            json!({"path":display(b,path),"typeName":prim.type_name(),"isInstance":prim.is_instance(),"isInstanceProxy":prim.is_instance_proxy(),"propertyNames":prim.property_names(),"attributes":attributes,
                "worldTransform":world.map(|m|m.into_iter().flatten().collect::<Vec<_>>())})
        }
        "children" => {
            let path = path(b, text(operation, "path")?)?;
            let prim = b
                .live
                .stage()
                .view(&b.store)
                .prim(path)
                .ok_or("prim does not exist")?;
            json!(
                prim.children()
                    .map(|p| display(b, p.path()))
                    .collect::<Vec<_>>()
            )
        }
        "transaction" => {
            let target_layer = layer_id(b, realm, text(operation, "target")?)?;
            let target = EditTarget::for_layer(target_layer);
            let edits = operation
                .get("edits")
                .and_then(Json::as_array)
                .ok_or("expected edits")?;
            if edits.len() > 256 {
                return Err("transaction exceeds256operations".into());
            }
            let mut transaction = Transaction::new();
            let mut membership = false;
            for edit in edits {
                let prim = path(b, text(edit, "path")?)?;
                match text(edit, "type")? {
                    "definePrim" => {
                        membership = true;
                        let kind = edit
                            .get("typeName")
                            .and_then(Json::as_str)
                            .map(|s| b.store.tokens.intern(s));
                        transaction.create_prim(target.prim(prim), Specifier::Def, kind);
                    }
                    "createAttribute" | "setAttribute" => {
                        let name = b.store.tokens.intern(text(edit, "name")?);
                        let property = target.property(PropertyPath::new(prim, name));
                        let value =
                            decode_value(b, edit.get("value").ok_or("missing typed value")?)?;
                        if text(edit, "type")? == "createAttribute" {
                            let ty = PropertyType::parse(text(edit, "typeName")?)?;
                            let mut spec = PropertySpec::typed_attribute(ty).with_default(value);
                            if edit.get("variability").and_then(Json::as_str) == Some("uniform") {
                                spec = spec.uniform();
                            }
                            transaction.create_property(property, spec);
                        } else {
                            transaction.set_default(property, value);
                        }
                    }
                    "addReference" => {
                        membership = true;
                        let layer = layer_id(b, realm, text(edit, "layer")?)?;
                        let referenced = path(b, text(edit, "primPath")?)?;
                        if layer != ROOT && layer != target_layer {
                            return Err(refusal(
                                "edit.referenceAsset",
                                "this transport supports stage-internal references through the root or authoring layer; external references require an authored asset path",
                            ));
                        }
                        transaction.add_reference(
                            target.prim(prim),
                            Reference::new(target_layer, referenced),
                            ListPosition::BackOfPrepend,
                        );
                    }
                    "setInstanceable" => {
                        membership = true;
                        transaction.set_instanceable(
                            target.prim(prim),
                            edit.get("value")
                                .and_then(Json::as_bool)
                                .ok_or("expected instanceable boolean")?,
                        );
                    }
                    unknown => {
                        return Err(
                            format!("unsupported native transaction operation {unknown}").into(),
                        );
                    }
                }
            }
            applied(b, realm, &transaction, membership)?
        }
        "undo" => {
            let id = parse_handle(realm, "undo", text(operation, "receipt")?)?;
            let (inverse, membership) = b
                .undo
                .get(&id)
                .ok_or("unknown or consumed undo receipt")?
                .clone();
            // Make room for the inverse receipt; restore the original on refusal.
            b.undo.remove(&id);
            match applied(b, realm, &inverse, membership) {
                Ok(result) => result,
                Err(error) => {
                    b.undo.insert(id, (inverse, membership));
                    return Err(error);
                }
            }
        }
        "releaseReceipt" => {
            let id = parse_handle(realm, "undo", text(operation, "receipt")?)?;
            b.undo
                .remove(&id)
                .ok_or("unknown or consumed undo receipt")?;
            json!({})
        }
        "mute" => {
            let id = layer_id(b, realm, text(operation, "layer")?)?;
            if operation
                .get("muted")
                .and_then(Json::as_bool)
                .ok_or("expected mute boolean")?
            {
                b.live.mute_layer(id)?;
            } else {
                b.live.unmute_layer(id);
            }
            let changed = b.live.recompose_changes(&mut b.store);
            refresh_placements(b, &changed, true);
            json!({"changes":changes(b,&changed)})
        }
        "layerText" => {
            let id = layer_id(b, realm, text(operation, "layer")?)?;
            json!({"name":layer_name(id),"text":layerstack_usda::save::save_usda(b.store.layer(id).expect("checked layer"),&b.store.tokens,&b.store.paths)?})
        }
        unknown => return Err(format!("unsupported stage operation {unknown}").into()),
    })
}
