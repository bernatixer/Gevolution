#![allow(dead_code)]
//! Test fixtures: small packages and scenarios built from JSON.

use serde_json::{Value, json};
use sim_core::compiler::{self, CompileEnv, Diagnostic, Plan};
use sim_core::schema::{Budgets, Package, Scenario};
use sim_core::{RunConfig, World};

pub fn pkg(v: Value) -> Package {
    serde_json::from_value(v).expect("fixture package parses")
}

/// Hydrology-only base package: water reservoirs, one open weather source and sinks.
pub fn base() -> Value {
    json!({
        "schema_version": 1,
        "package_id": "t.base",
        "capabilities": ["core.cells.v1", "core.edges.v1", "sandbox.external_source"],
        "resources": [{ "id": "water", "unit": "kg" }, { "id": "nutrient", "unit": "kg" }],
        "accounts": [
            { "id": "external.weather", "resource": "water", "direction": "source" },
            { "id": "external.atmosphere", "resource": "water", "direction": "sink" }
        ],
        "fields": [
            { "id": "elevation", "unit": "m", "policy": { "kind": "parameter" } },
            { "id": "temperature", "unit": "K", "policy": { "kind": "integrated" }, "default": 290.0 },
            { "id": "surface_water", "unit": "kg", "policy": { "kind": "reservoir", "resource": "water" } },
            { "id": "cap", "unit": "kg", "policy": { "kind": "parameter" }, "default": 10.0 },
            { "id": "soil_water", "unit": "kg", "policy": { "kind": "reservoir", "resource": "water", "capacity": "cap" } },
            { "id": "soil_nutrients", "unit": "kg", "policy": { "kind": "reservoir", "resource": "nutrient" } },
            { "id": "plant_nutrients", "unit": "kg", "policy": { "kind": "reservoir", "resource": "nutrient" } },
            { "id": "memory", "unit": "kg", "policy": { "kind": "next_value" } }
        ],
        "forcings": [{ "id": "rain_flux", "unit": "kg/(m^2*s)", "quantity": "water", "domain": "cells" }],
        "rules": []
    })
}

pub fn with_rules(mut p: Value, rules: Vec<Value>) -> Value {
    p["rules"] = Value::Array(rules);
    p
}

pub fn env(w: usize, h: usize) -> CompileEnv {
    CompileEnv {
        grids: vec!["world.surface".into()],
        width: w,
        height: h,
        cell_size: 10.0,
        dt: 0.25,
        regions: vec!["a".into(), "b".into()],
        budgets: Budgets::default(),
        population_cap: 100,
    }
}

pub fn compile(p: Value) -> Result<Plan, Vec<Diagnostic>> {
    compiler::compile(&[pkg(p)], &env(4, 4))
}

pub fn compile_err(p: Value) -> Vec<Diagnostic> {
    match compile(p) {
        Ok(_) => panic!("expected a compile error"),
        Err(d) => d,
    }
}

pub fn has_code(d: &[Diagnostic], code: &str) -> bool {
    d.iter().any(|x| x.code == code)
}

pub fn rule(id: &str, kind: &str, nodes: Value) -> Value {
    let effects: Vec<String> = nodes
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| {
            matches!(
                n["op"].as_str().unwrap(),
                "transfer"
                    | "external_source"
                    | "external_sink"
                    | "reaction"
                    | "edge_transfer"
                    | "rate_contribution"
                    | "next_value"
                    | "move"
                    | "death"
                    | "birth"
            )
        })
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect();
    json!({ "rule_id": id, "domain": { "kind": kind }, "nodes": nodes, "effects": effects })
}

pub fn scenario(w: usize, h: usize, initial: Value, regions: Value, rain: f64) -> Scenario {
    serde_json::from_value(json!({
        "schema_version": 1,
        "scenario_id": "t",
        "seed": 7,
        "grid": { "width": w, "height": h, "cell_size": 10.0, "chunk_size": 32 },
        "packages": [],
        "terrain": { "kind": "flat", "elevation": 0.0 },
        "initial": initial,
        "weather": { "signals": { "rain_flux": { "mean": rain } } },
        "regions": regions
    }))
    .expect("fixture scenario parses")
}

pub fn world(sc: Scenario, pkgs: Vec<Value>, config: RunConfig) -> World {
    World::new(sc, pkgs.into_iter().map(pkg).collect(), config)
        .unwrap_or_else(|e| panic!("world: {e}"))
}

pub fn field<'a>(w: &'a World, name: &str) -> &'a [f64] {
    &w.state.cells[w.schema().cell_field(name).unwrap()]
}

pub fn explicit(values: Vec<f64>) -> Value {
    json!({ "kind": "explicit", "values": values })
}

pub fn constant(v: f64) -> Value {
    json!({ "kind": "constant", "value": v })
}
