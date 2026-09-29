//! M4 gates and T19-T26, T28-T30: organisms, lifecycle, workers, persistence.

mod common;
use common::*;
use serde_json::{Value, json};
use sim_core::commands::CommandKind;
use sim_core::eval::Mode;
use sim_core::{RunConfig, World, biology, persistence};

/// A minimal archetype with direct, brain-free rules for exact fixtures.
fn blob_pkg(rules: Vec<Value>, cooldown: f64) -> Value {
    let mut p = base();
    p["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!("core.entities.v1"));
    p["resources"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "energy", "unit": "J" }));
    p["accounts"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "external.heat", "resource": "energy", "direction": "sink" }));
    p["archetypes"] = json!([{
        "id": "blob",
        "fields": [
            { "id": "store", "unit": "kg", "policy": { "kind": "reservoir", "resource": "water" } },
            { "id": "energy", "unit": "J", "policy": { "kind": "reservoir", "resource": "energy" } },
            { "id": "flag", "unit": "1", "policy": { "kind": "parameter" } }
        ],
        "traits": [{ "id": "size", "unit": "1", "min": 0.5, "max": 2.0, "init_min": 1.0, "init_max": 1.0 }],
        "brain": { "backend": "mlp.tanh.v1", "inputs": 1, "hidden": 1, "outputs": 1, "weight_bound": 1.0, "init_scale": 0.5 },
        "max_speed": 10.0, "max_turn_rate": 1.0,
        "death_disposal": [{ "field": "store", "to": "cell.soil_water" }, { "field": "energy", "to": "external.heat" }],
        "mutation": { "probability": 0.5, "scale": 0.1 },
        "reproduction_cooldown": cooldown,
        "seed_state": { "store": 6.0, "energy": 1.0, "flag": 0.0 }
    }]);
    p["rules"] = Value::Array(rules);
    p
}

fn erule(id: &str, nodes: Value) -> Value {
    let mut r = rule(id, "entities", nodes);
    r["domain"] = json!({ "kind": "entities", "archetype": "blob" });
    r
}

fn blob_world(rules: Vec<Value>, initial: Value, count: usize, cooldown: f64) -> World {
    let mut sc = scenario(
        2,
        2,
        initial,
        json!([{ "id": "a", "shape": { "kind": "rect", "x0": 0, "z0": 0, "x1": 1, "z1": 1 } }]),
        0.0,
    );
    sc.populations =
        serde_json::from_value(json!([{ "archetype": "blob", "count": count, "region": "a" }]))
            .unwrap();
    world(sc, vec![blob_pkg(rules, cooldown)], RunConfig::reference())
}

fn col<'a>(w: &'a World, f: &str) -> &'a [f64] {
    let a = &w.schema().archetypes[0];
    &w.state.entities[0].fields[a.field(f).unwrap()]
}

#[test]
fn t19_organisms_and_environment_share_one_limited_inventory() {
    let drink = erule(
        "drink",
        json!([
            { "id": "a", "op": "const", "value": 4.0, "unit": "kg" },
            { "id": "x", "op": "transfer", "resource": "water", "from": "cell.surface_water", "to": "self.store", "amount": "a" }
        ]),
    );
    let evap = rule(
        "evap",
        "cells",
        json!([
            { "id": "a", "op": "const", "value": 4.0, "unit": "kg" },
            { "id": "x", "op": "external_sink", "resource": "water", "from": "surface_water", "account": "external.atmosphere", "amount": "a" }
        ]),
    );
    let mut w = blob_world(
        vec![drink, evap],
        json!({ "surface_water": explicit(vec![10.0, 0.0, 0.0, 0.0]) }),
        3,
        1.0,
    );
    let before: Vec<f64> = col(&w, "store").to_vec();
    w.step().unwrap();
    // 16 kg requested from 10 kg: every consumer receives 10/16 of its request.
    for (b, a) in before.iter().zip(col(&w, "store")) {
        assert_eq!(a - b, 2.5);
    }
    assert_eq!(field(&w, "surface_water")[0], 0.0);
    let atm = w.schema().account("external.atmosphere").unwrap();
    assert_eq!(w.state.account_out[atm], 2.5);
}

#[test]
fn t20_partially_funded_movement_and_single_location_eating() {
    let rules = vec![
        erule(
            "move",
            json!([
                { "id": "want", "op": "const", "value": 8.0, "unit": "m/s" },
                { "id": "cost", "op": "const", "value": 2.0, "unit": "J/m" },
                { "id": "p", "op": "mul", "inputs": ["want", "cost"] },
                { "id": "pay", "op": "external_sink", "resource": "energy", "from": "self.energy", "account": "external.heat", "rate": "p" },
                { "id": "v", "op": "mul", "inputs": ["want", "pay.fraction"] },
                { "id": "t", "op": "const", "value": 0.0, "unit": "1/s" },
                { "id": "m", "op": "move", "turn": "t", "speed": "v" }
            ]),
        ),
        erule(
            "graze",
            json!([
                { "id": "a", "op": "const", "value": 1.0, "unit": "kg" },
                { "id": "x", "op": "transfer", "resource": "water", "from": "cell.surface_water", "to": "self.store", "amount": "a" }
            ]),
        ),
    ];
    let mut w = blob_world(rules, json!({ "surface_water": constant(100.0) }), 1, 1.0);
    // Put the organism near the cell edge heading +x; one tick requests 8 m/s * 0.25 s = 2 m costing 4 J; it has 1 J.
    {
        let e = &mut w.state.entities[0];
        e.x[0] = 9.8;
        e.z[0] = 5.0;
        e.heading[0] = 0.0;
    }
    w.step().unwrap();
    let e = &w.state.entities[0];
    assert!(
        (e.x[0] - 10.3).abs() < 1e-9,
        "moved only the funded quarter: x = {}",
        e.x[0]
    );
    assert_eq!(col(&w, "energy")[0], 0.0);
    let sw = field(&w, "surface_water");
    assert_eq!(sw[0], 99.0);
    assert_eq!(sw[1], 100.0);
    // Next tick it is in cell 1 and grazes there only.
    w.step().unwrap();
    let sw = field(&w, "surface_water");
    assert_eq!((sw[0], sw[1]), (99.0, 99.0));
}

fn birth_rule(amount: f64, threshold: f64) -> Value {
    erule(
        "breed",
        json!([
            { "id": "yes", "op": "gt", "inputs": ["s", "z"] },
            { "id": "s", "op": "candidate_state", "field": "self.store" },
            { "id": "z", "op": "const", "value": threshold, "unit": "kg" },
            { "id": "give", "op": "const", "value": amount, "unit": "kg" },
            { "id": "b", "op": "birth", "condition": "yes", "legs": [{ "field": "store", "amount": "give" }] }
        ]),
    )
}

#[test]
fn t21_births_transfer_atomically_rejections_cost_nothing_newborns_wait() {
    let drink = erule(
        "drink",
        json!([
            { "id": "a", "op": "const", "value": 1.0, "unit": "kg" },
            { "id": "x", "op": "transfer", "resource": "water", "from": "cell.surface_water", "to": "self.store", "amount": "a" }
        ]),
    );
    let mut w = blob_world(
        vec![birth_rule(2.0, 6.5), drink.clone()],
        json!({ "surface_water": constant(100.0) }),
        1,
        1000.0,
    );
    let total = w.ledger[0].total;
    w.step().unwrap();
    assert_eq!(w.population(), 2);
    let s = col(&w, "store");
    assert_eq!(s[0], 6.0 + 1.0 - 2.0);
    assert_eq!(
        s[1], 2.0,
        "newborn holds exactly the transferred amount and has not drunk yet"
    );
    assert!((w.ledger[0].total - total).abs() < 1e-12);
    assert_eq!(
        w.state.entities[0].lineage[1].parent,
        w.state.entities[0].ids[0]
    );
    w.step().unwrap();
    assert_eq!(
        col(&w, "store")[1],
        3.0,
        "newborn acts from the following tick"
    );

    // A birth the parent cannot fund is rejected and costs nothing.
    let mut w = blob_world(vec![birth_rule(50.0, 0.0)], json!({}), 1, 1000.0);
    w.step().unwrap();
    assert_eq!(w.population(), 1);
    assert_eq!(col(&w, "store")[0], 6.0);
}

#[test]
fn t22_death_disposes_resources_and_precedes_birth() {
    let die = erule(
        "die",
        json!([
            { "id": "t", "op": "const", "value": 1.0, "unit": "1" },
            { "id": "z", "op": "const", "value": 0.0, "unit": "1" },
            { "id": "c", "op": "gt", "inputs": ["t", "z"] },
            { "id": "d", "op": "death", "condition": "c", "reason": "fixture" }
        ]),
    );
    let mut w = blob_world(vec![die, birth_rule(1.0, 0.0)], json!({}), 3, 1000.0);
    let water0 = w.ledger[0].total;
    w.step().unwrap();
    assert_eq!(w.population(), 0, "dying parents do not reproduce");
    assert_eq!(w.stats.births, 0);
    assert_eq!(
        field(&w, "soil_water")[0],
        18.0,
        "every organism's water is deposited in its cell"
    );
    assert!((w.ledger[0].total - water0).abs() < 1e-12);
    let heat = w.schema().account("external.heat").unwrap();
    assert_eq!(w.state.account_out[heat], 3.0);
    assert_eq!(w.stats.deaths_by_reason["fixture"], 3);
}

#[test]
fn t23_mutation_preserves_validity_varies_offspring_and_leaves_parents() {
    let root = sim_core::assets::asset_root();
    let (sc, pkgs) =
        sim_core::assets::load_scenario(&root.join("scenarios/seasonal_river.json")).unwrap();
    let env = sim_core::world::compile_env(&sc, vec!["upstream".into(), "basin".into()]);
    let plan = sim_core::compiler::compile(&pkgs, &env).unwrap();
    let a = &plan.schema.archetypes[0];
    assert_eq!(a.brain_params(), 310);
    let parent = biology::random_genome(a, 1, 1);
    let copy = parent.clone();
    let mut changed = 0;
    for child in 0..1000u64 {
        let g = biology::mutate(a, &parent, 1, 5, child + 10);
        assert!(biology::genome_valid(a, &g));
        changed += g.iter().zip(&parent).filter(|(x, y)| x != y).count();
    }
    assert_eq!(parent, copy);
    let expected = 1000.0 * a.genome_len() as f64 * a.mutation.probability;
    assert!(
        (changed as f64) > expected * 0.8 && (changed as f64) < expected * 1.2,
        "{changed} vs {expected}"
    );
}

/// Authored forager (a labeled control fixture): always eats, drinks, walks, and breeds.
fn forager(pref: f64, count: usize) -> Value {
    json!({
        "archetype": "herbivore", "count": count, "genome": "authored:forager",
        "traits": { "preferred_temperature": pref, "tolerance_breadth": 3.0, "body_size": 1.0, "water_conservation": 0.35, "locomotion_efficiency": 0.5 },
        "authored_links": [[6, 0, 2.0], [8, 0, 3.0]],
        "authored_bias": [0.0, 0.4, 3.0, 3.0, 3.0, -3.0]
    })
}

#[test]
fn t24_thermal_trade_offs_differ_between_hot_and_cool_fixtures() {
    let root = sim_core::assets::asset_root();
    let (base_sc, pkgs) =
        sim_core::assets::load_scenario(&root.join("scenarios/seasonal_river.json")).unwrap();
    let run = |ambient: f64, seed: u64| -> (f64, f64) {
        let mut sc = base_sc.clone();
        sc.seed = seed;
        sc.grid.width = 48;
        sc.grid.height = 48;
        sc.regions.clear();
        sc.weather
            .signals
            .get_mut("ambient_temperature")
            .unwrap()
            .mean = ambient;
        sc.weather
            .signals
            .get_mut("ambient_temperature")
            .unwrap()
            .amplitude = 0.0;
        // Start the fixture at its own thermal equilibrium, not the default scenario's.
        sc.initial.insert(
            "temperature".into(),
            serde_json::from_value(json!({ "kind": "elevation", "a": ambient + 2.0, "b": -10.0 }))
                .unwrap(),
        );
        sc.populations =
            serde_json::from_value(json!([forager(284.0, 40), forager(302.0, 40)])).unwrap();
        let mut w = World::new(
            sc,
            pkgs.clone(),
            RunConfig {
                sample_interval: 0,
                ..RunConfig::default()
            },
        )
        .unwrap();
        w.run(1600).unwrap();
        let e = &w.state.entities[0];
        let cool = (0..e.len()).filter(|&i| e.genome_of(i)[1] < 293.0).count() as f64;
        let hot = (0..e.len()).filter(|&i| e.genome_of(i)[1] >= 293.0).count() as f64;
        (cool, hot)
    };
    let mut hot_wins = 0;
    let mut cool_wins = 0;
    for seed in [1, 2, 3] {
        let (c_cool, h_cool) = run(284.0, seed);
        let (c_hot, h_hot) = run(302.0, seed);
        println!(
            "T24 seed {seed}: cool fixture cool/hot-adapted {c_cool}/{h_cool}; hot fixture {c_hot}/{h_hot}"
        );
        if h_hot > c_hot {
            hot_wins += 1;
        }
        if c_cool > h_cool {
            cool_wins += 1;
        }
    }
    assert!(
        hot_wins >= 2 && cool_wins >= 2,
        "trade-offs should favor the matching variant in most seeds"
    );
}

fn river(size: usize, config: RunConfig) -> World {
    let root = sim_core::assets::asset_root();
    let (mut sc, pkgs) =
        sim_core::assets::load_scenario(&root.join("scenarios/seasonal_river.json")).unwrap();
    sc.grid.width = size;
    sc.grid.height = size;
    sc.regions.retain(|r| r.id == "upstream");
    sc.regions[0].shape = serde_json::from_value(
        json!({ "kind": "rect", "x0": 0, "z0": 0, "x1": size, "z1": size / 3 }),
    )
    .unwrap();
    sc.populations[0].count = 120;
    World::new(sc, pkgs, config).unwrap()
}

#[test]
fn t25_worker_count_does_not_change_the_run() {
    let mut hashes = vec![];
    for threads in [1, 2, 8] {
        let mut w = river(
            64,
            RunConfig {
                mode: Mode::Parallel,
                optimize: true,
                threads: Some(threads),
                sample_interval: 0,
            },
        );
        w.run(300).unwrap();
        hashes.push(w.state.hash());
    }
    let mut r = river(64, RunConfig::reference());
    r.run(300).unwrap();
    hashes.push(r.state.hash());
    assert!(hashes.windows(2).all(|p| p[0] == p[1]), "{hashes:x?}");
}

#[test]
fn t26_save_load_continuation_equals_uninterrupted_execution() {
    let mut a = river(40, RunConfig::default());
    a.run(150).unwrap();
    a.submit(
        CommandKind::SetParam {
            name: "core.env.rain:rain_factor".into(),
            value: 0.5,
        },
        Some(180),
    );
    let bytes = persistence::save(&a);
    a.run(150).unwrap();
    let mut b = persistence::load(&bytes, RunConfig::reference()).unwrap();
    assert_eq!(b.tick(), 150);
    b.run(150).unwrap();
    assert_eq!(a.state.hash(), b.state.hash());
    assert_eq!(a.log, b.log);
    // Incompatible build/profile metadata is reported, not silently accepted.
    let needle = b"strict-reference.f64.v1";
    let at = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap();
    let mut tampered = bytes.clone();
    tampered[at + needle.len() - 1] = b'0';
    let err = persistence::load(&tampered, RunConfig::default())
        .err()
        .expect("rejected");
    assert!(err.contains("incompatible build"), "{err}");
}

#[test]
fn t28_invalid_hot_swap_leaves_old_rules_and_world_intact() {
    let mut a = river(32, RunConfig::default());
    let mut b = river(32, RunConfig::default());
    let mut broken = a.active.packages.clone();
    broken[0].rules[0].nodes[0].op = "no_such_op".into();
    a.submit(CommandKind::ApplyPackages { packages: broken }, None);
    let plan_before = a.plan().source_hash;
    a.run(20).unwrap();
    b.run(20).unwrap();
    assert_eq!(a.plan().source_hash, plan_before);
    assert_eq!(a.state.hash(), b.state.hash());
    assert!(a.events.iter().any(|e| matches!(&e.kind, sim_core::world::EventKind::CommandRejected { reason, .. } if reason.contains("no_such_op"))));
    // An out-of-range parameter edit is rejected too.
    a.submit(
        CommandKind::SetParam {
            name: "core.env.rain:rain_factor".into(),
            value: 99.0,
        },
        None,
    );
    a.run(1).unwrap();
    b.run(1).unwrap();
    assert_eq!(a.state.hash(), b.state.hash());
}

#[test]
fn t29_inspection_does_not_change_authoritative_state() {
    let mut a = river(32, RunConfig::default());
    let mut b = river(32, RunConfig::default());
    for _ in 0..40 {
        a.step().unwrap();
        b.step().unwrap();
        for c in [0, 100, 500] {
            let x = b.explain_cell("surface_water", c).unwrap();
            assert!(
                x.other.abs() < 1e-6,
                "explanation accounts for the change: {x:?}"
            );
            b.explain_cell("temperature", c).unwrap();
        }
        if let Some(&id) = b.state.entities[0].ids.first() {
            b.explain_entity(0, id);
        }
    }
    assert_eq!(a.state.hash(), b.state.hash());
}

#[test]
fn t30_player_rule_survives_save_load_with_identical_results() {
    // A regional drying law built from primitive nodes.
    let law = json!({
        "rule_id": "player.dry_upstream",
        "label": "Extra evaporation upstream",
        "domain": { "kind": "cells" },
        "scope": { "region": "upstream" },
        "parameters": { "k": { "value": 0.0005, "unit": "kg/(m^2*s)", "min": 0.0, "max": 0.01 } },
        "nodes": [
            { "id": "k", "op": "parameter", "name": "k" },
            { "id": "area", "op": "cell_area" },
            { "id": "w", "op": "read_state", "field": "surface_water" },
            { "id": "rate0", "op": "mul", "inputs": ["k", "area"] },
            { "id": "rate", "op": "min", "inputs": ["rate0", "cap"] },
            { "id": "cap", "op": "mul", "inputs": ["w", "per_s"] },
            { "id": "per_s", "op": "const", "value": 1.0, "unit": "1/s" },
            { "id": "dry", "op": "external_sink", "resource": "water", "from": "surface_water", "account": "external.atmosphere", "rate": "rate" }
        ],
        "effects": ["dry"]
    });
    let mut a = river(32, RunConfig::default());
    let mut pkgs = a.active.packages.clone();
    let mut player: sim_core::schema::Package = serde_json::from_value(json!({
        "schema_version": 1, "package_id": "player.experiment", "requires": ["core.environment"], "capabilities": ["core.cells.v1"], "rules": [law]
    }))
    .unwrap();
    // Round-trip the authored package through its canonical JSON.
    player = serde_json::from_str(&serde_json::to_string_pretty(&player).unwrap()).unwrap();
    pkgs.push(player);
    let mut b = river(32, RunConfig::default());
    a.submit(
        CommandKind::ApplyPackages {
            packages: pkgs.clone(),
        },
        None,
    );
    b.submit(CommandKind::ApplyPackages { packages: pkgs }, None);
    a.run(30).unwrap();
    let saved = persistence::save(&a);
    let mut c = persistence::load(&saved, RunConfig::default()).unwrap();
    assert!(
        c.plan()
            .effects
            .iter()
            .any(|e| e.id == "player.dry_upstream/dry")
    );
    a.run(30).unwrap();
    b.run(60).unwrap();
    c.run(30).unwrap();
    assert_eq!(a.state.hash(), b.state.hash());
    assert_eq!(a.state.hash(), c.state.hash());
}
