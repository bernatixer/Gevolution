//! T01-T08: compiler and semantic acceptance tests.

mod common;
use common::*;
use serde_json::json;
use sim_core::compiler;
use sim_core::schema::Budgets;

#[test]
fn t01_reject_adding_water_to_temperature_and_name_ports() {
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "bad",
            "cells",
            json!([
                { "id": "w", "op": "read_state", "field": "surface_water" },
                { "id": "t", "op": "read_state", "field": "temperature" },
                { "id": "sum", "op": "add", "inputs": ["w", "t"] }
            ]),
        )],
    ));
    let e = d.iter().find(|x| x.code == "E_UNIT").expect("unit error");
    assert_eq!(e.rule.as_deref(), Some("bad"));
    assert_eq!(e.node.as_deref(), Some("sum"));
    assert!(
        e.message.contains("inputs[0]") && e.message.contains("inputs[1]"),
        "{}",
        e.message
    );
    assert!(
        e.message.contains("kg") && e.message.contains("[K]"),
        "{}",
        e.message
    );
}

#[test]
fn t01b_reject_same_unit_different_quantity() {
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "bad",
            "cells",
            json!([
                { "id": "w", "op": "read_state", "field": "surface_water" },
                { "id": "n", "op": "read_state", "field": "soil_nutrients" },
                { "id": "sum", "op": "add", "inputs": ["w", "n"] }
            ]),
        )],
    ));
    assert!(has_code(&d, "E_QUANTITY"));
    // A water amount cannot silently fund a nutrient transfer either.
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "bad2",
            "cells",
            json!([
                { "id": "w", "op": "read_state", "field": "surface_water" },
                { "id": "k", "op": "const", "value": 0.001, "unit": "1/s" },
                { "id": "r", "op": "mul", "inputs": ["w", "k"] },
                { "id": "x", "op": "transfer", "resource": "nutrient", "from": "soil_nutrients", "to": "plant_nutrients", "rate": "r" }
            ]),
        )],
    ));
    assert!(has_code(&d, "E_QUANTITY"), "{d:?}");
}

#[test]
fn t02_reject_domain_and_grid_mixing() {
    // Cell field mixed with an entity column without an explicit domain operation.
    let mut p = base();
    p["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!("core.entities.v1"));
    p["archetypes"] = json!([{
        "id": "blob", "fields": [{ "id": "store", "unit": "kg", "policy": { "kind": "reservoir", "resource": "water" } }],
        "traits": [], "brain": { "backend": "mlp.tanh.v1", "inputs": 1, "hidden": 1, "outputs": 1, "weight_bound": 1.0, "init_scale": 0.1 },
        "max_speed": 1.0, "max_turn_rate": 1.0,
        "death_disposal": [{ "field": "store", "to": "cell.surface_water" }],
        "mutation": { "probability": 0.0, "scale": 0.0 }, "reproduction_cooldown": 1.0
    }]);
    p["rules"] = json!([{
        "rule_id": "mix", "domain": { "kind": "entities", "archetype": "blob" },
        "nodes": [
            { "id": "cell_w", "op": "read_state", "field": "surface_water" },
            { "id": "own", "op": "read_state", "field": "self.store" },
            { "id": "sum", "op": "add", "inputs": ["cell_w", "own"] }
        ]
    }]);
    let d = compile_err(p.clone());
    let e = d
        .iter()
        .find(|x| x.code == "E_DOMAIN")
        .expect("domain error");
    assert!(
        e.message.contains("cells") && e.message.contains("entities"),
        "{}",
        e.message
    );
    // The explicit `sample` operation makes the same expression valid.
    p["rules"][0]["nodes"] = json!([
        { "id": "cell_w", "op": "read_state", "field": "surface_water" },
        { "id": "here", "op": "sample", "inputs": ["cell_w"] },
        { "id": "own", "op": "read_state", "field": "self.store" },
        { "id": "sum", "op": "add", "inputs": ["here", "own"] }
    ]);
    compile(p).expect("sampled expression compiles");

    // Two grids: a field on another grid cannot mix with the surface grid.
    let mut p = base();
    p["fields"].as_array_mut().unwrap().push(json!({ "id": "coarse_w", "grid": "world.coarse", "unit": "kg", "policy": { "kind": "parameter" } }));
    p["rules"] = json!([rule(
        "grids",
        "cells",
        json!([
            { "id": "a", "op": "read_state", "field": "surface_water" },
            { "id": "b", "op": "read_state", "field": "coarse_w" },
            { "id": "sum", "op": "add", "inputs": ["a", "b"] }
        ])
    )]);
    let mut e2 = env(4, 4);
    e2.grids.push("world.coarse".into());
    let d = compiler::compile(&[pkg(p)], &e2).unwrap_err();
    let e = d
        .iter()
        .find(|x| x.code == "E_DOMAIN")
        .expect("grid identity error");
    assert!(e.message.contains("world.coarse"), "{}", e.message);
}

#[test]
fn t03_cycles_rejected_and_explicit_feedback_accepted() {
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "loop",
            "cells",
            json!([
                { "id": "a", "op": "add", "inputs": ["b", "c"] },
                { "id": "b", "op": "mul", "inputs": ["a", "c"] },
                { "id": "c", "op": "const", "value": 1.0, "unit": "1" }
            ]),
        )],
    ));
    let e = d
        .iter()
        .find(|x| x.code == "E_CYCLE")
        .expect("cycle diagnostic");
    assert!(
        e.message.contains("loop/a") && e.message.contains("loop/b"),
        "{}",
        e.message
    );
    // Feedback through stored state is fine: memory_next = memory + surface_water.
    compile(with_rules(
        base(),
        vec![rule(
            "feedback",
            "cells",
            json!([
                { "id": "m", "op": "read_state", "field": "memory" },
                { "id": "w", "op": "read_state", "field": "surface_water" },
                { "id": "w2", "op": "as_quantity", "inputs": ["w"] },
                { "id": "next", "op": "add", "inputs": ["m", "w2"] },
                { "id": "store", "op": "next_value", "field": "memory", "value": "next" }
            ]),
        )],
    ))
    .expect("stored-state feedback compiles");
}

#[test]
fn t04_exclusive_writers_rejected_rate_contributions_combine() {
    let writer = |id: &str| {
        rule(
            id,
            "cells",
            json!([
                { "id": "v", "op": "const", "value": 1.0, "unit": "kg" },
                { "id": "w", "op": "next_value", "field": "memory", "value": "v" }
            ]),
        )
    };
    let d = compile_err(with_rules(base(), vec![writer("one"), writer("two")]));
    assert!(has_code(&d, "E_MULTIPLE_WRITERS"));
    let rate = |id: &str| {
        rule(
            id,
            "cells",
            json!([
                { "id": "r", "op": "const", "value": 0.1, "unit": "dK/s" },
                { "id": "w", "op": "rate_contribution", "field": "temperature", "rate": "r" }
            ]),
        )
    };
    compile(with_rules(base(), vec![rate("one"), rate("two")]))
        .expect("rate contributions combine");
    // A conserved reservoir cannot take an arbitrary delta.
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "delta",
            "cells",
            json!([
                { "id": "r", "op": "const", "value": 1.0, "unit": "kg/s" },
                { "id": "w", "op": "rate_contribution", "field": "surface_water", "rate": "r" }
            ]),
        )],
    ));
    assert!(has_code(&d, "E_WRITER"));
}

#[test]
fn t05_reject_negative_requests_and_undeclared_external_creation() {
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "neg",
            "cells",
            json!([
                { "id": "r", "op": "const", "value": -1.0, "unit": "kg/s" },
                { "id": "x", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "r" }
            ]),
        )],
    ));
    assert!(has_code(&d, "E_NEGATIVE_REQUEST"));
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "undeclared",
            "cells",
            json!([
                { "id": "r", "op": "const", "value": 1.0, "unit": "kg/s" },
                { "id": "x", "op": "external_source", "resource": "water", "account": "external.miracle", "to": "surface_water", "rate": "r" }
            ]),
        )],
    ));
    assert!(has_code(&d, "E_UNDECLARED_EXTERNAL"));
    // Using a sink as a source is also undeclared creation.
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "wrongway",
            "cells",
            json!([
                { "id": "r", "op": "const", "value": 1.0, "unit": "kg/s" },
                { "id": "x", "op": "external_source", "resource": "water", "account": "external.atmosphere", "to": "surface_water", "rate": "r" }
            ]),
        )],
    ));
    assert!(has_code(&d, "E_UNDECLARED_EXTERNAL"));
    // A source account requires the sandbox capability.
    let mut p = base();
    p["capabilities"] = json!(["core.cells.v1"]);
    assert!(has_code(&compile_err(p), "E_EXTERNAL_SOURCE"));
}

#[test]
fn t07_reject_receipt_feedback_into_requests() {
    // Direct cycle: a transfer's request depends on its own receipt.
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "self_feedback",
            "cells",
            json!([
                { "id": "k", "op": "const", "value": 0.5, "unit": "1" },
                { "id": "r", "op": "mul", "inputs": ["x.accepted_rate", "k"] },
                { "id": "x", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "r" }
            ]),
        )],
    ));
    assert!(has_code(&d, "E_RECEIPT_FEEDBACK"), "{d:?}");
    // Cross-effect: one transfer's receipt funds another transfer's request.
    let d = compile_err(with_rules(
        base(),
        vec![rule(
            "cross",
            "cells",
            json!([
                { "id": "r", "op": "const", "value": 0.5, "unit": "kg/s" },
                { "id": "a", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "r" },
                { "id": "b", "op": "transfer", "resource": "water", "from": "soil_water", "to": "surface_water", "rate": "a.accepted_rate" }
            ]),
        )],
    ));
    assert!(has_code(&d, "E_RECEIPT_FEEDBACK"), "{d:?}");
    // Receipts may feed non-resource outcomes.
    compile(with_rules(
        base(),
        vec![rule(
            "ok",
            "cells",
            json!([
                { "id": "r", "op": "const", "value": 0.5, "unit": "kg/s" },
                { "id": "a", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "r" },
                { "id": "f", "op": "mul", "inputs": ["a.fraction", "q"] },
                { "id": "q", "op": "const", "value": 0.1, "unit": "dK/s" },
                { "id": "t", "op": "rate_contribution", "field": "temperature", "rate": "f" }
            ]),
        )],
    ))
    .expect("receipt into outcome compiles");
}

#[test]
fn t08_budgets() {
    let big: Vec<serde_json::Value> = (0..20)
        .map(|i| json!({ "id": format!("n{i}"), "op": "const", "value": 1.0, "unit": "1" }))
        .collect();
    let p = with_rules(base(), vec![rule("big", "cells", json!(big))]);
    let mut e = env(4, 4);
    e.budgets = Budgets {
        max_nodes: 10,
        ..Budgets::default()
    };
    assert!(has_code(
        &compiler::compile(&[pkg(p.clone())], &e).unwrap_err(),
        "E_BUDGET_NODES"
    ));
    let p = with_rules(
        base(),
        vec![rule(
            "wide",
            "cells",
            json!([
                { "id": "w", "op": "read_state", "field": "surface_water" },
                { "id": "s", "op": "neighbor_sum", "inputs": ["w"], "radius": 9 }
            ]),
        )],
    );
    assert!(has_code(&compile_err(p), "E_BUDGET_RADIUS"));
    let p = with_rules(
        base(),
        vec![rule(
            "heavy",
            "cells",
            json!([
                { "id": "w", "op": "read_state", "field": "surface_water" },
                { "id": "s", "op": "neighbor_sum", "inputs": ["w"], "radius": 4 }
            ]),
        )],
    );
    let mut e = env(512, 512);
    e.budgets = Budgets {
        max_element_ops: 1_000_000,
        max_transient_bytes: 1 << 20,
        ..Budgets::default()
    };
    let d = compiler::compile(&[pkg(p)], &e).unwrap_err();
    assert!(
        has_code(&d, "E_BUDGET_OPS") && has_code(&d, "E_BUDGET_MEMORY"),
        "{d:?}"
    );
}

#[test]
fn stability_limits_are_checked_at_compile_time() {
    let mut p = with_rules(
        base(),
        vec![rule(
            "fast",
            "cells",
            json!([
                { "id": "w", "op": "read_state", "field": "surface_water" },
                { "id": "k", "op": "parameter", "name": "k" },
                { "id": "r", "op": "mul", "inputs": ["k", "w"] },
                { "id": "x", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "r" }
            ]),
        )],
    );
    p["rules"][0]["parameters"] = json!({ "k": { "value": 8.0, "unit": "1/s", "min": 0.0, "max": 10.0, "stability": { "kind": "first_order" } } });
    assert!(has_code(&compile_err(p), "E_NUMERIC_RISK"));
}

#[test]
fn standard_pack_compiles_without_warnings() {
    let root = sim_core::assets::asset_root();
    let (sc, pkgs) =
        sim_core::assets::load_scenario(&root.join("scenarios/seasonal_river.json")).unwrap();
    let env = sim_core::world::compile_env(&sc, sc.regions.iter().map(|r| r.id.clone()).collect());
    let plan = compiler::compile(&pkgs, &env)
        .unwrap_or_else(|d| panic!("{}", sim_core::world::format_diagnostics(&d)));
    let warnings: Vec<String> = plan.warnings.iter().map(|w| w.to_string()).collect();
    assert!(warnings.is_empty(), "{warnings:#?}");
}
