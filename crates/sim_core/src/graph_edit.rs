//! Structural edits on rule graphs, shared by the editor and tests.

use crate::catalog::{self, Inputs};
use crate::schema::{Node, ParamDecl, Rule};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn is_effect(op: &str) -> bool {
    catalog::info(op).is_some_and(|i| i.effect)
}

/// Keep a rule's effects list equal to its effect nodes.
pub fn sync_effects(rule: &mut Rule) {
    rule.effects = rule.nodes.iter().filter(|n| is_effect(&n.op)).map(|n| n.id.clone()).collect();
}

/// Set the k-th input row (positional ports first, then named references).
pub fn set_input(n: &mut Node, k: usize, src: &str) {
    let info = catalog::info(&n.op);
    match info.map(|i| i.inputs) {
        Some(Inputs::Positional(names)) if k < names.len() => {
            let mut arr = n.args.get("inputs").and_then(|v| v.as_array().cloned()).unwrap_or_default();
            arr.resize(names.len(), json!(""));
            arr[k] = json!(src);
            n.args.insert("inputs".into(), Value::Array(arr));
        }
        Some(Inputs::Named(names)) if k < names.len() => {
            n.args.insert(names[k].to_string(), json!(src));
        }
        Some(Inputs::Variadic) => {
            let mut arr = n.args.get("inputs").and_then(|v| v.as_array().cloned()).unwrap_or_default();
            if k < arr.len() {
                arr[k] = json!(src);
            } else {
                arr.push(json!(src));
            }
            n.args.insert("inputs".into(), Value::Array(arr));
        }
        _ => {
            // Reaction/birth leg references: rows after the named ports.
            if let Some(Value::Array(legs)) = n.args.get_mut("legs") {
                let offset = k - info.map(|i| if let Inputs::Named(x) = i.inputs { x.len() } else { 0 }).unwrap_or(0);
                if let Some(l) = legs.get_mut(offset)
                    && let Some(o) = l.as_object_mut()
                {
                    let key = if o.contains_key("amount") { "amount" } else { "rate" };
                    o.insert(key.into(), json!(src));
                }
            }
        }
    }
}

pub fn rename_refs(args: &mut BTreeMap<String, Value>, from: &str, to: &str) {
    fn fix(v: &mut Value, from: &str, to: &str) {
        match v {
            Value::String(s) => {
                if s == from {
                    *s = to.to_string();
                } else if let Some(rest) = s.strip_prefix(&format!("{from}.")) {
                    *s = format!("{to}.{rest}");
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| fix(x, from, to)),
            Value::Object(o) => {
                for (k, x) in o.iter_mut() {
                    if matches!(k.as_str(), "rate" | "amount") {
                        fix(x, from, to);
                    }
                }
            }
            _ => {}
        }
    }
    for (k, v) in args.iter_mut() {
        if matches!(
            k.as_str(),
            "inputs" | "rate" | "amount" | "value" | "turn" | "speed" | "condition" | "legs"
        ) {
            fix(v, from, to);
        }
    }
}

/// Rewire input `port` of node `ni` through `mul(original, lerp(1, factor, region))`.
pub fn insert_blend(rule: &mut Rule, ni: usize, port: usize, region: &str) {
    let node = rule.nodes[ni].clone();
    let refs = catalog::node_refs(&node);
    let names: Vec<String> = match catalog::info(&node.op).map(|i| i.inputs) {
        Some(Inputs::Positional(n)) => n.iter().map(|s| s.to_string()).collect(),
        Some(Inputs::Named(n)) => n.iter().map(|s| s.to_string()).collect(),
        _ => return,
    };
    let Some(pname) = names.get(port) else { return };
    let original = refs.iter().find(|r| &r.0 == pname).map(|r| r.1.clone()).unwrap_or_default();
    let base = format!("{}_{}", node.id, region.replace('.', "_"));
    let pname_param = format!("{}_factor", region.replace('.', "_"));
    rule.parameters.entry(pname_param.clone()).or_insert(ParamDecl {
        value: 0.3,
        unit: "1".into(),
        quantity: None,
        min: 0.0,
        max: 5.0,
        label: format!("Multiplier inside region {region}"),
        stability: None,
    });
    let (px, py) = node
        .args
        .get("pos")
        .and_then(|p| p.as_array())
        .and_then(|a| Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)))
        .unwrap_or((0.0, 0.0));
    let at = |dx: f32, dy: f32| json!([px - 260.0 + dx, py + dy]);
    let mk = |id: &str, op: &str, mut args: BTreeMap<String, Value>, pos: Value| {
        args.insert("pos".into(), pos);
        Node {
            id: id.to_string(),
            op: op.to_string(),
            args,
        }
    };
    let obj = |v: Value| -> BTreeMap<String, Value> { serde_json::from_value(v).unwrap() };
    let new = vec![
        mk(
            &format!("{base}_mask"),
            "region",
            obj(json!({ "region": region })),
            at(-230.0, 140.0),
        ),
        mk(
            &format!("{base}_one"),
            "const",
            obj(json!({ "value": 1.0, "unit": "1" })),
            at(-230.0, 200.0),
        ),
        mk(
            &format!("{base}_factor"),
            "parameter",
            obj(json!({ "name": pname_param })),
            at(-230.0, 260.0),
        ),
        mk(
            &format!("{base}_blend"),
            "lerp",
            obj(json!({ "inputs": [format!("{base}_one"), format!("{base}_factor"), format!("{base}_mask")] })),
            at(0.0, 180.0),
        ),
        mk(
            &format!("{base}_scaled"),
            "mul",
            obj(json!({ "inputs": [original, format!("{base}_blend")] })),
            at(0.0, 60.0),
        ),
    ];
    for n in new {
        if !rule.nodes.iter().any(|x| x.id == n.id) {
            rule.nodes.push(n);
        }
    }
    set_input(&mut rule.nodes[ni], port, &format!("{base}_scaled"));
    sync_effects(rule);
}
