//! M1/M2 gates and T09-T18: tick semantics, resolver, conservation, transport.

mod common;
use common::*;
use serde_json::{Value, json};
use sim_core::eval::Mode;
use sim_core::{RunConfig, World};

fn amount_rule(id: &str, from: &str, to: &str, kg: f64) -> Value {
    rule(
        id,
        "cells",
        json!([
            { "id": "a", "op": "const", "value": kg, "unit": "kg" },
            { "id": "x", "op": "transfer", "resource": "water", "from": from, "to": to, "amount": "a" }
        ]),
    )
}

fn accepted(w: &World, effect: &str) -> Vec<Vec<f64>> {
    let i = w.plan().effects.iter().position(|e| e.id == effect).unwrap();
    w.last.as_ref().unwrap().resolved.accepted[i].clone()
}

#[test]
fn t09_competing_withdrawals_share_proportionally() {
    let p = with_rules(
        base(),
        vec![
            amount_rule("r1", "surface_water", "soil_water", 8.0),
            amount_rule("r2", "surface_water", "soil_water", 8.0),
        ],
    );
    let sc = scenario(
        2,
        2,
        json!({ "surface_water": explicit(vec![10.0, 0.0, 0.0, 0.0]), "cap": constant(100.0) }),
        json!([]),
        0.0,
    );
    let mut w = world(sc, vec![p], RunConfig::reference());
    w.step().unwrap();
    assert_eq!(accepted(&w, "r1/x")[0][0], 5.0);
    assert_eq!(accepted(&w, "r2/x")[0][0], 5.0);
    assert_eq!(field(&w, "surface_water")[0], 0.0);
    assert_eq!(field(&w, "soil_water")[0], 10.0);
}

#[test]
fn t10_coupled_reaction_scales_every_leg_by_the_limiting_factor() {
    let p = with_rules(
        base(),
        vec![rule(
            "grow",
            "cells",
            json!([
                { "id": "w", "op": "const", "value": 2.0, "unit": "kg" },
                { "id": "n", "op": "const", "value": 1.0, "unit": "kg" },
                { "id": "x", "op": "reaction", "legs": [
                    { "resource": "water", "from": "surface_water", "to": "soil_water", "amount": "w" },
                    { "resource": "nutrient", "from": "soil_nutrients", "to": "plant_nutrients", "amount": "n" }
                ] }
            ]),
        )],
    );
    let sc = scenario(
        2,
        2,
        json!({ "surface_water": constant(1.0), "soil_nutrients": constant(0.8), "cap": constant(100.0) }),
        json!([]),
        0.0,
    );
    let mut w = world(sc, vec![p], RunConfig::reference());
    w.step().unwrap();
    let r = &w.last.as_ref().unwrap().resolved;
    let e = w.plan().effects.iter().position(|e| e.id == "grow/x").unwrap();
    assert_eq!(r.receipts.alpha[e][0], 0.5);
    assert_eq!(field(&w, "soil_water")[0], 1.0);
    assert_eq!(field(&w, "plant_nutrients")[0], 0.5);
    assert!((field(&w, "soil_nutrients")[0] - 0.3).abs() < 1e-15);
    assert_eq!(field(&w, "surface_water")[0], 0.0);
}

#[test]
fn t11_capacity_limits_deposits_and_rejected_material_stays_at_source() {
    let p = with_rules(
        base(),
        vec![
            amount_rule("i1", "surface_water", "soil_water", 4.0),
            amount_rule("i2", "surface_water", "soil_water", 4.0),
        ],
    );
    let sc = scenario(
        2,
        2,
        json!({ "surface_water": constant(100.0), "soil_water": constant(7.0), "cap": constant(10.0) }),
        json!([]),
        0.0,
    );
    let mut w = world(sc, vec![p], RunConfig::reference());
    w.step().unwrap();
    assert_eq!(accepted(&w, "i1/x")[0][0], 1.5);
    assert_eq!(accepted(&w, "i2/x")[0][0], 1.5);
    assert_eq!(field(&w, "soil_water")[0], 10.0);
    assert_eq!(field(&w, "surface_water")[0], 97.0);
}

#[test]
fn t12_rain_and_evaporation_exchange_exactly_their_accepted_amounts() {
    let p = with_rules(
        base(),
        vec![
            rule(
                "rain",
                "cells",
                json!([
                    { "id": "f", "op": "read_forcing", "forcing": "rain_flux" },
                    { "id": "a", "op": "cell_area" },
                    { "id": "r", "op": "mul", "inputs": ["f", "a"] },
                    { "id": "x", "op": "external_source", "resource": "water", "account": "external.weather", "to": "surface_water", "rate": "r" }
                ]),
            ),
            rule(
                "evap",
                "cells",
                json!([
                    { "id": "w", "op": "read_state", "field": "surface_water" },
                    { "id": "k", "op": "const", "value": 0.5, "unit": "1/s" },
                    { "id": "r", "op": "mul", "inputs": ["k", "w"] },
                    { "id": "x", "op": "external_sink", "resource": "water", "from": "surface_water", "account": "external.atmosphere", "rate": "r" }
                ]),
            ),
        ],
    );
    let sc = scenario(3, 3, json!({ "surface_water": constant(4.0) }), json!([]), 0.001);
    let mut w = world(sc, vec![p], RunConfig::reference());
    let schema = w.schema().clone();
    let (wi, ai) = (
        schema.account("external.weather").unwrap(),
        schema.account("external.atmosphere").unwrap(),
    );
    let mut rain = 0.0;
    let mut evap = 0.0;
    for _ in 0..50 {
        w.step().unwrap();
        rain += accepted(&w, "rain/x")[0].iter().sum::<f64>();
        evap += accepted(&w, "evap/x")[0].iter().sum::<f64>();
    }
    assert!((w.state.account_in[wi] - rain).abs() < 1e-9);
    assert!((w.state.account_out[ai] - evap).abs() < 1e-9);
    let l = &w.ledger[schema.resource("water").unwrap()];
    assert!(l.cumulative_error.abs() < 1e-9, "{l:?}");
    assert!((l.total - (36.0 + rain - evap)).abs() < 1e-9);
}

fn flow_rule(conductance: f64) -> Value {
    json!({
        "rule_id": "flow",
        "domain": { "kind": "edges" },
        "parameters": { "c": { "value": conductance, "unit": "kg/(m*s)", "min": 0.0, "max": 1e9 } },
        "nodes": [
            { "id": "e", "op": "read_state", "field": "elevation" },
            { "id": "w", "op": "read_state", "field": "surface_water" },
            { "id": "rho", "op": "const", "value": 1000.0, "unit": "kg/m^3" },
            { "id": "area", "op": "cell_area" },
            { "id": "ra", "op": "mul", "inputs": ["rho", "area"] },
            { "id": "z", "op": "const", "value": 0.0, "unit": "m" },
            { "id": "d", "op": "safe_divide", "inputs": ["w", "ra", "z"] },
            { "id": "h", "op": "add", "inputs": ["e", "d"] },
            { "id": "ha", "op": "edge_from", "inputs": ["h"] },
            { "id": "hb", "op": "edge_to", "inputs": ["h"] },
            { "id": "dh", "op": "sub", "inputs": ["ha", "hb"] },
            { "id": "c", "op": "parameter", "name": "c" },
            { "id": "r", "op": "mul", "inputs": ["c", "dh"] },
            { "id": "x", "op": "edge_transfer", "resource": "water", "field": "surface_water", "rate": "r" }
        ],
        "effects": ["x"]
    })
}

#[test]
fn t13_equal_heads_produce_no_transport_request() {
    let p = with_rules(base(), vec![flow_rule(5000.0)]);
    let mut sc = scenario(2, 2, json!({ "surface_water": explicit(vec![2e5, 1e5, 2e5, 1e5]) }), json!([]), 0.0);
    sc.terrain = serde_json::from_value(json!({ "kind": "explicit", "values": [0.0, 1.0, 0.0, 1.0] })).unwrap();
    let mut w = world(sc, vec![p], RunConfig::reference());
    w.step().unwrap();
    let e = w.plan().effects.iter().position(|e| e.id == "flow/x").unwrap();
    let req = &w.last.as_ref().unwrap().resolved.receipts.requested[e][0];
    assert!(req.iter().all(|v| *v == 0.0), "{req:?}");
    assert_eq!(field(&w, "surface_water"), &[2e5, 1e5, 2e5, 1e5]);
}

fn river_world(config: RunConfig, size: usize) -> World {
    let root = sim_core::assets::asset_root();
    let (mut sc, pkgs) = sim_core::assets::load_scenario(&root.join("scenarios/seasonal_river.json")).unwrap();
    sc.grid.width = size;
    sc.grid.height = size;
    sc.regions.clear();
    sc.populations[0].count = 40;
    World::new(sc, pkgs, config).unwrap()
}

#[test]
fn t14_chunked_parallel_execution_matches_unchunked_reference() {
    let mut a = river_world(RunConfig::reference(), 70);
    let mut b = river_world(
        RunConfig {
            mode: Mode::Parallel,
            optimize: false,
            threads: Some(4),
            sample_interval: 0,
        },
        70,
    );
    for _ in 0..60 {
        a.step().unwrap();
        b.step().unwrap();
        assert_eq!(a.state.hash(), b.state.hash(), "diverged at tick {}", a.tick());
    }
}

#[test]
fn t15_closed_water_soak_100k_ticks() {
    let p = with_rules(
        base(),
        vec![
            flow_rule(5000.0),
            rule(
                "infiltrate",
                "cells",
                json!([
                    { "id": "w", "op": "read_state", "field": "surface_water" },
                    { "id": "k", "op": "const", "value": 0.002, "unit": "1/s" },
                    { "id": "r", "op": "mul", "inputs": ["k", "w"] },
                    { "id": "x", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "r" }
                ]),
            ),
            rule(
                "seep",
                "cells",
                json!([
                    { "id": "s", "op": "read_state", "field": "soil_water" },
                    { "id": "k", "op": "const", "value": 0.001, "unit": "1/s" },
                    { "id": "r", "op": "mul", "inputs": ["k", "s"] },
                    { "id": "x", "op": "transfer", "resource": "water", "from": "soil_water", "to": "surface_water", "rate": "r" }
                ]),
            ),
        ],
    );
    let mut sc = scenario(
        32,
        32,
        json!({ "surface_water": { "kind": "noise", "lo": 0.0, "hi": 50000.0, "feature": 6.0 }, "cap": constant(20000.0) }),
        json!([]),
        0.0,
    );
    sc.terrain =
        serde_json::from_value(json!({ "kind": "valley", "relief": 20.0, "valley_depth": 4.0, "basin_depth": 5.0, "roughness": 2.0 }))
            .unwrap();
    let mut w = world(
        sc,
        vec![p],
        RunConfig {
            sample_interval: 0,
            ..RunConfig::default()
        },
    );
    let initial = w.ledger[0].total;
    for _ in 0..100_000 {
        w.step().unwrap();
    }
    let l = &w.ledger[0];
    let gate = 1e-6 + 1e-8 * initial;
    println!(
        "T15: initial {initial:.6e} kg, final {:.6e}, cumulative error {:e} (gate {gate:e}), max tick error {:e}",
        l.total, l.cumulative_error, l.max_tick_error
    );
    assert!(l.cumulative_error.abs() <= gate);
    for f in ["surface_water", "soil_water"] {
        assert!(field(&w, f).iter().all(|v| *v >= 0.0));
    }
    assert!(w.state.roundoff[0] < 1e-6, "roundoff corrections {}", w.state.roundoff[0]);
}

#[test]
fn t16_diffusion_converges_when_the_timestep_is_halved() {
    let p = with_rules(
        base(),
        vec![rule(
            "diffuse",
            "cells",
            json!([
                { "id": "t", "op": "read_state", "field": "temperature" },
                { "id": "l", "op": "laplacian", "inputs": ["t"] },
                { "id": "d", "op": "const", "value": 20.0, "unit": "m^2/s" },
                { "id": "r", "op": "mul", "inputs": ["d", "l"] },
                { "id": "x", "op": "rate_contribution", "field": "temperature", "rate": "r" }
            ]),
        )],
    );
    let n = 24;
    let init: Vec<f64> = (0..n * n)
        .map(|c| {
            let (x, z) = ((c % n) as f64, (c / n) as f64);
            290.0 + 5.0 * (std::f64::consts::PI * (x + 0.5) / n as f64).cos() * (std::f64::consts::PI * (z + 0.5) / n as f64).cos()
        })
        .collect();
    let run = |dt: f64| -> Vec<f64> {
        let mut sc = scenario(n, n, json!({ "temperature": explicit(init.clone()) }), json!([]), 0.0);
        sc.dt = dt;
        let mut w = world(sc, vec![p.clone()], RunConfig::reference());
        let ticks = (10.0 / dt).round() as u64;
        w.run(ticks).unwrap();
        field(&w, "temperature").to_vec()
    };
    let reference = run(0.25 / 32.0);
    let err = |v: &[f64]| v.iter().zip(&reference).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
    let (e1, e2) = (err(&run(0.25)), err(&run(0.125)));
    println!("T16: error at dt=0.25: {e1:e}, at dt=0.125: {e2:e}");
    assert!(e2 < e1 * 0.7, "halving dt should reduce error: {e1} -> {e2}");
}

fn scoped_rain(id: &str, region: &str) -> Value {
    json!({
        "rule_id": id,
        "domain": { "kind": "cells" },
        "scope": { "region": region },
        "nodes": [
            { "id": "r", "op": "const", "value": 1.0, "unit": "kg/s" },
            { "id": "x", "op": "external_source", "resource": "water", "account": "external.weather", "to": "surface_water", "rate": "r" }
        ],
        "effects": ["x"]
    })
}

#[test]
fn t17_region_overlap_is_additive_and_order_independent() {
    let regions = json!([
        { "id": "a", "shape": { "kind": "rect", "x0": 0, "z0": 0, "x1": 3, "z1": 4 } },
        { "id": "b", "shape": { "kind": "rect", "x0": 2, "z0": 0, "x1": 4, "z1": 4 } }
    ]);
    let run = |rules: Vec<Value>| {
        let sc = scenario(4, 4, json!({}), regions.clone(), 0.0);
        let mut w = world(sc, vec![with_rules(base(), rules)], RunConfig::reference());
        w.step().unwrap();
        (w.state.hash(), field(&w, "surface_water").to_vec())
    };
    let (h1, v) = run(vec![scoped_rain("ra", "a"), scoped_rain("rb", "b")]);
    let (h2, _) = run(vec![scoped_rain("rb", "b"), scoped_rain("ra", "a")]);
    assert_eq!(h1, h2);
    assert_eq!(v[0], 0.25); // only a
    assert_eq!(v[2], 0.5); // overlap: both add
    assert_eq!(v[3], 0.25); // only b
}

#[test]
fn t18_cross_region_transport_debits_and_credits_both_endpoints() {
    let mut f = flow_rule(5000.0);
    f["scope"] = json!({ "region": "a" });
    let regions = json!([{ "id": "a", "shape": { "kind": "rect", "x0": 0, "z0": 0, "x1": 1, "z1": 2 } }]);
    let sc = scenario(2, 2, json!({ "surface_water": explicit(vec![1e5, 0.0, 1e5, 0.0]) }), regions, 0.0);
    let mut w = world(sc, vec![with_rules(base(), vec![f])], RunConfig::reference());
    w.step().unwrap();
    let v = field(&w, "surface_water");
    // Edge coverage is the mean of its endpoints: 0.5 across the region boundary.
    assert!(v[1] > 0.0 && v[3] > 0.0, "destination outside the region is credited: {v:?}");
    assert_eq!(v[0] + v[1], 1e5);
    assert_eq!(v[2] + v[3], 1e5);
    assert!(w.ledger[0].cumulative_error.abs() < 1e-9);
}

#[test]
fn m1_rule_registration_order_does_not_change_results() {
    let root = sim_core::assets::asset_root();
    let (mut sc, mut pkgs) = sim_core::assets::load_scenario(&root.join("scenarios/seasonal_river.json")).unwrap();
    sc.grid.width = 24;
    sc.grid.height = 24;
    sc.regions.clear();
    sc.populations[0].count = 30;
    let mut a = World::new(sc.clone(), pkgs.clone(), RunConfig::reference()).unwrap();
    pkgs.reverse();
    for p in &mut pkgs {
        p.rules.reverse();
        p.fields.rotate_left(0);
    }
    let mut b = World::new(sc, pkgs, RunConfig::reference()).unwrap();
    a.run(30).unwrap();
    b.run(30).unwrap();
    assert_eq!(a.state.hash(), b.state.hash());
}

#[test]
fn m1_explicit_feedback_works_across_ticks() {
    let p = with_rules(
        base(),
        vec![rule(
            "count",
            "cells",
            json!([
                { "id": "m", "op": "read_state", "field": "memory" },
                { "id": "one", "op": "const", "value": 1.0, "unit": "kg" },
                { "id": "n", "op": "add", "inputs": ["m", "one"] },
                { "id": "x", "op": "next_value", "field": "memory", "value": "n" }
            ]),
        )],
    );
    let mut w = world(scenario(2, 2, json!({}), json!([]), 0.0), vec![p], RunConfig::reference());
    w.run(7).unwrap();
    assert!(field(&w, "memory").iter().all(|v| *v == 7.0));
}

#[test]
fn m1_failed_tick_leaves_committed_state_untouched() {
    let p = with_rules(
        base(),
        vec![rule(
            "bad",
            "cells",
            json!([
                { "id": "w", "op": "read_state", "field": "surface_water" },
                { "id": "one", "op": "const", "value": 1.0, "unit": "1" },
                { "id": "k", "op": "const", "value": 0.001, "unit": "1/(kg*s)" },
                { "id": "z", "op": "mul", "inputs": ["k", "w"] },
                { "id": "zz", "op": "const", "value": 1.0, "unit": "s" },
                { "id": "u", "op": "mul", "inputs": ["z", "zz"] },
                { "id": "l", "op": "ln", "inputs": ["u"] },
                { "id": "dk", "op": "const", "value": 1.0, "unit": "dK/s" },
                { "id": "r", "op": "mul", "inputs": ["l", "dk"] },
                { "id": "x", "op": "rate_contribution", "field": "temperature", "rate": "r" }
            ]),
        )],
    );
    let sc = scenario(
        2,
        2,
        json!({ "surface_water": explicit(vec![1000.0, 1000.0, 1000.0, 0.0]) }),
        json!([]),
        0.0,
    );
    let mut w = world(sc, vec![p], RunConfig::reference());
    let before = w.state.hash();
    let seq = w.submit(sim_core::commands::CommandKind::CreateRegion { id: "late".into() }, None);
    let f = w.step().unwrap_err();
    assert_eq!(w.state.hash(), before);
    assert_eq!(w.tick(), 0);
    assert!(f.nodes.iter().any(|n| n == "bad/l"), "{f}");
    assert!(w.pending.iter().any(|c| c.seq == seq), "boundary commands are restored");
    assert!(w.step().is_err(), "the experiment stays paused until cleared");
}

#[test]
fn m0_identical_manifests_produce_identical_initial_states() {
    let a = river_world(RunConfig::reference(), 48);
    let b = river_world(RunConfig::default(), 48);
    assert_eq!(a.state.hash(), b.state.hash());
    let mut sc = a.scenario.clone();
    sc.seed += 1;
    let c = World::new(sc, a.active.packages.clone(), RunConfig::reference()).unwrap();
    assert_ne!(a.state.hash(), c.state.hash());
}

#[test]
fn m2_basin_pools_to_a_level_surface_without_growing_oscillation() {
    let n = 16;
    let bowl: Vec<f64> = (0..n * n)
        .map(|c| {
            let (x, z) = ((c % n) as f64 - 7.5, (c / n) as f64 - 7.5);
            0.02 * (x * x + z * z)
        })
        .collect();
    let mut sc = scenario(n, n, json!({ "surface_water": constant(20000.0) }), json!([]), 0.0);
    sc.terrain = serde_json::from_value(json!({ "kind": "explicit", "values": bowl.clone() })).unwrap();
    let mut w = world(sc, vec![with_rules(base(), vec![flow_rule(5000.0)])], RunConfig::reference());
    let total0 = w.ledger[0].total;
    let head = |w: &World| -> Vec<f64> { field(w, "surface_water").iter().zip(&bowl).map(|(m, e)| e + m / 1e5).collect() };
    let spread = |w: &World| {
        let wet: Vec<f64> = head(w)
            .into_iter()
            .zip(field(w, "surface_water"))
            .filter(|(_, m)| **m > 1.0)
            .map(|(h, _)| h)
            .collect();
        let mean = wet.iter().sum::<f64>() / wet.len() as f64;
        wet.iter().map(|h| (h - mean).abs()).fold(0.0, f64::max)
    };
    w.run(200).unwrap();
    let early = spread(&w);
    w.run(4000).unwrap();
    let late = spread(&w);
    println!("basin head spread: {early:e} -> {late:e}");
    assert!(late < 1e-3 && late <= early, "water surface levels out: {early} -> {late}");
    let v = field(&w, "surface_water");
    assert!(v[0] < v[n * n / 2 + n / 2], "water pools at the bottom of the bowl");
    assert!((w.ledger[0].total - total0).abs() < 1e-6);
}
