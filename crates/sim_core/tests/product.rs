//! T06, T27, T31, T32 and the M5 gate: optimization safety, differential
//! execution, the guided river, malformed input, and player formula edits.

mod common;
use common::*;
use serde_json::{Value, json};
use sim_core::commands::CommandKind;
use sim_core::compiler;
use sim_core::eval::Mode;
use sim_core::graph_edit;
use sim_core::ir::Op;
use sim_core::schema::Package;
use sim_core::{RunConfig, World, optimize, persistence};

fn standard(size: usize, organisms: usize) -> (sim_core::schema::Scenario, Vec<Package>) {
    let root = sim_core::assets::asset_root();
    let (mut sc, pkgs) = sim_core::assets::load_scenario(&root.join("scenarios/seasonal_river.json")).unwrap();
    let scale = size as f64 / sc.grid.width as f64;
    sc.grid.width = size;
    sc.grid.height = size;
    for r in &mut sc.regions {
        match &mut r.shape {
            sim_core::schema::RegionShape::Rect { x0, z0, x1, z1 } => {
                *x0 = (*x0 as f64 * scale) as usize;
                *z0 = (*z0 as f64 * scale) as usize;
                *x1 = (*x1 as f64 * scale) as usize;
                *z1 = (*z1 as f64 * scale) as usize;
            }
            sim_core::schema::RegionShape::Circle { cx, cz, radius, .. } => {
                *cx *= scale;
                *cz *= scale;
                *radius *= scale;
            }
            _ => {}
        }
    }
    sc.populations[0].count = organisms;
    (sc, pkgs)
}

#[test]
fn t06_optimization_preserves_random_streams_and_duplicate_effects() {
    let r = rule(
        "dup",
        "cells",
        json!([
            { "id": "r1", "op": "random" },
            { "id": "r2", "op": "random" },
            { "id": "k", "op": "const", "value": 1.0, "unit": "kg/s" },
            { "id": "a", "op": "mul", "inputs": ["r1", "k"] },
            { "id": "b", "op": "mul", "inputs": ["r2", "k"] },
            { "id": "x1", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "a" },
            { "id": "x2", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "a" },
            { "id": "x3", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "b" }
        ]),
    );
    let plan = compile(with_rules(base(), vec![r.clone()])).unwrap();
    let opt = optimize::optimize(plan.clone());
    let randoms = |p: &compiler::Plan| p.instrs.iter().filter(|i| matches!(i.op, Op::Random(_))).count();
    assert_eq!(randoms(&opt), 2, "two random nodes stay two independent streams");
    assert_eq!(opt.effects.len(), 3, "identical-looking transfers remain separate requests");
    let sc = scenario(
        6,
        6,
        json!({ "surface_water": constant(100.0), "cap": constant(1e6) }),
        json!([]),
        0.0,
    );
    let mut a = world(sc.clone(), vec![with_rules(base(), vec![r.clone()])], RunConfig::reference());
    let mut b = world(sc, vec![with_rules(base(), vec![r])], RunConfig::default());
    a.run(20).unwrap();
    b.run(20).unwrap();
    assert_eq!(a.state.hash(), b.state.hash());
    let e1 = a.plan().effects.iter().position(|e| e.id == "dup/x1").unwrap();
    let e3 = a.plan().effects.iter().position(|e| e.id == "dup/x3").unwrap();
    let last = &a.last.as_ref().unwrap().resolved.receipts.requested;
    assert_ne!(last[e1][0], last[e3][0], "separate streams draw different values");
}

/// A small random generator for typed expression graphs.
struct Gen(u64);
impl Gen {
    fn next(&mut self) -> u64 {
        self.0 = sim_core::rng::mix(self.0);
        self.0
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn f(&mut self) -> f64 {
        sim_core::rng::unit(self.next())
    }
}

/// Build a random dimensionless expression over cell state; returns the root id.
fn expr(g: &mut Gen, nodes: &mut Vec<Value>, depth: u32) -> String {
    let id = format!("n{}", g.next() % 1_000_000_000);
    if depth == 0 || g.pick(4) == 0 {
        match g.pick(3) {
            0 => {
                let f = ["surface_water", "soil_water"][g.pick(2)];
                let raw = format!("{id}_raw");
                let s = format!("{id}_s");
                nodes.push(json!({ "id": raw, "op": "read_state", "field": f }));
                nodes.push(json!({ "id": s, "op": "const", "value": 0.001 + g.f() * 0.01, "unit": "1/kg" }));
                nodes.push(json!({ "id": id, "op": "mul", "inputs": [raw, s] }));
            }
            1 => nodes.push(json!({ "id": id, "op": "const", "value": g.f() * 4.0 - 2.0, "unit": "1" })),
            _ => nodes.push(json!({ "id": id, "op": "random" })),
        }
        return id;
    }
    let ops = [
        "add",
        "sub",
        "mul",
        "min",
        "max",
        "clamp",
        "select",
        "curve",
        "safe_divide",
        "laplacian",
        "neighbor_mean",
        "tanh",
        "abs",
        "lerp",
    ];
    let op = ops[g.pick(ops.len())];
    let mut a = expr(g, nodes, depth - 1);
    if matches!(op, "laplacian" | "neighbor_mean") {
        // Spatial operators need a cell-varying input: add a cell-state term.
        let raw = format!("{id}_cr");
        let sc = format!("{id}_cs");
        let sum = format!("{id}_ca");
        nodes.push(json!({ "id": raw, "op": "read_state", "field": "surface_water" }));
        nodes.push(json!({ "id": sc, "op": "const", "value": 0.004, "unit": "1/kg" }));
        let m = format!("{id}_cm");
        nodes.push(json!({ "id": m, "op": "mul", "inputs": [raw, sc] }));
        nodes.push(json!({ "id": sum, "op": "add", "inputs": [a, m] }));
        a = sum;
    }
    match op {
        "laplacian" | "tanh" | "abs" => {
            // Scale the Laplacian (1/m^2) back to dimensionless.
            if op == "laplacian" {
                let l = format!("{id}_l");
                let k = format!("{id}_k");
                nodes.push(json!({ "id": l, "op": "laplacian", "inputs": [a] }));
                nodes.push(json!({ "id": k, "op": "const", "value": 100.0, "unit": "m^2" }));
                nodes.push(json!({ "id": id, "op": "mul", "inputs": [l, k] }));
            } else {
                nodes.push(json!({ "id": id, "op": op, "inputs": [a] }));
            }
        }
        "neighbor_mean" => nodes.push(json!({ "id": id, "op": op, "inputs": [a], "radius": 1 + g.pick(2) })),
        "curve" => nodes.push(json!({ "id": id, "op": "curve", "inputs": [a], "in_unit": "1", "out_unit": "1", "points": [[-1.0, 0.0], [0.0, g.f()], [1.0, 1.0]] })),
        "clamp" | "lerp" | "safe_divide" => {
            let b = expr(g, nodes, depth - 1);
            let c = expr(g, nodes, depth - 1);
            nodes.push(json!({ "id": id, "op": op, "inputs": [a, b, c] }));
        }
        "select" => {
            let b = expr(g, nodes, depth - 1);
            let c = expr(g, nodes, depth - 1);
            let cond = format!("{id}_c");
            nodes.push(json!({ "id": cond, "op": "gt", "inputs": [b, c] }));
            nodes.push(json!({ "id": id, "op": "select", "inputs": [cond, a, b] }));
        }
        _ => {
            let b = expr(g, nodes, depth - 1);
            nodes.push(json!({ "id": id, "op": op, "inputs": [a, b] }));
        }
    }
    id
}

#[test]
fn t27_reference_and_optimized_executors_agree_on_generated_graphs() {
    for seed in 0..40u64 {
        let mut g = Gen(seed + 1000);
        let mut nodes = vec![];
        let root = expr(&mut g, &mut nodes, 4);
        let root2 = expr(&mut g, &mut nodes, 3);
        nodes.push(json!({ "id": "k", "op": "const", "value": 0.01, "unit": "dK/s" }));
        nodes.push(json!({ "id": "rate", "op": "mul", "inputs": [root, "k"] }));
        nodes.push(json!({ "id": "t", "op": "rate_contribution", "field": "temperature", "rate": "rate" }));
        nodes.push(json!({ "id": "z", "op": "const", "value": 0.0, "unit": "1" }));
        nodes.push(json!({ "id": "pos", "op": "max", "inputs": [root2, "z"] }));
        nodes.push(json!({ "id": "q", "op": "const", "value": 0.5, "unit": "kg/s" }));
        nodes.push(json!({ "id": "amt", "op": "mul", "inputs": ["pos", "q"] }));
        nodes.push(json!({ "id": "x", "op": "transfer", "resource": "water", "from": "surface_water", "to": "soil_water", "rate": "amt" }));
        let p = with_rules(base(), vec![rule("gen", "cells", Value::Array(nodes))]);
        let sc = scenario(
            40,
            40,
            json!({ "surface_water": { "kind": "noise", "lo": 0.0, "hi": 300.0, "feature": 5.0 }, "cap": constant(1e6) }),
            json!([]),
            0.0,
        );
        let mut a = world(sc.clone(), vec![p.clone()], RunConfig::reference());
        let mut b = world(
            sc,
            vec![p],
            RunConfig {
                mode: Mode::Parallel,
                optimize: true,
                threads: Some(4),
                sample_interval: 0,
            },
        );
        assert!(b.plan().instrs.len() <= a.plan().instrs.len());
        let ra = a.run(15);
        let rb = b.run(15);
        assert_eq!(ra.is_ok(), rb.is_ok(), "seed {seed}: executors disagree on failure");
        assert_eq!(
            a.state.hash(),
            b.state.hash(),
            "seed {seed}: reference and optimized executors diverged"
        );
    }
}

fn surface_outside_basin(w: &World) -> f64 {
    let s = w.schema();
    let f = s.cell_field("surface_water").unwrap();
    let basin = s.region("basin").unwrap();
    w.state.cells[f]
        .iter()
        .zip(&w.state.regions[basin])
        .filter(|(_, m)| **m < 0.05)
        .map(|(v, _)| *v)
        .sum()
}

#[test]
fn t31_guided_river_dries_and_recovers_through_budgets() {
    let (sc, pkgs) = standard(128, 0);
    // The player's formula edit: rain × lerp(1, factor, upstream coverage).
    let mut edited = pkgs.clone();
    let env = edited.iter_mut().find(|p| p.package_id == "core.environment").unwrap();
    let rain = env.rules.iter_mut().find(|r| r.rule_id == "core.env.rain").unwrap();
    let ni = rain.nodes.iter().position(|n| n.id == "rain").unwrap();
    graph_edit::insert_blend(rain, ni, 0, "upstream");
    let cfg = || RunConfig {
        sample_interval: 0,
        ..RunConfig::default()
    };
    let mut control = World::new(sc.clone(), pkgs.clone(), cfg()).unwrap();
    control.run(1200).unwrap();
    let checkpoint = persistence::save(&control);
    let mut dry = persistence::load(&checkpoint, cfg()).unwrap();
    dry.submit(CommandKind::ApplyPackages { packages: edited.clone() }, None);
    dry.submit(
        CommandKind::SetParam {
            name: "core.env.rain:upstream_factor".into(),
            value: 0.0,
        },
        Some(1201),
    );
    control.run(6000).unwrap();
    dry.run(6000).unwrap();
    assert!(dry.plan().effects.iter().any(|e| e.id == "core.env.rain/rain"));
    let (wc, wd) = (surface_outside_basin(&control), surface_outside_basin(&dry));
    let weather = |w: &World| w.state.account_in[w.schema().account("external.weather").unwrap()];
    println!(
        "T31 river water outside the basin: control {wc:.3e} kg, upstream drought {wd:.3e} kg; rain received {:.3e} vs {:.3e}",
        weather(&control),
        weather(&dry)
    );
    assert!(wd < wc * 0.8, "the river shrinks when upstream rain is withheld");
    assert!(weather(&dry) < weather(&control), "the budget shows less accepted rain");
    // Restore rainfall in one branch, keep the drought in another.
    let mut still_dry = persistence::load(&persistence::save(&dry), cfg()).unwrap();
    dry.submit(
        CommandKind::SetParam {
            name: "core.env.rain:upstream_factor".into(),
            value: 1.0,
        },
        None,
    );
    dry.run(6000).unwrap();
    still_dry.run(6000).unwrap();
    let (wr, wk) = (surface_outside_basin(&dry), surface_outside_basin(&still_dry));
    println!("T31 after restoring: restored {wr:.3e} kg, continued drought {wk:.3e} kg");
    assert!(wr > wk * 1.25 && wr > wd, "the river recovers when rain returns");
    for w in [&control, &dry, &still_dry] {
        for l in &w.ledger {
            assert!(
                l.cumulative_error.abs() <= 1e-6 + 1e-8 * (l.initial + l.cumulative_abs_exchange),
                "{l:?}"
            );
        }
    }
}

#[test]
fn m5_player_formula_edit_saves_reloads_and_has_a_documented_effect() {
    let (sc, pkgs) = standard(48, 0);
    let mut edited = pkgs.clone();
    let env = edited.iter_mut().find(|p| p.package_id == "core.environment").unwrap();
    let rain = env.rules.iter_mut().find(|r| r.rule_id == "core.env.rain").unwrap();
    let ni = rain.nodes.iter().position(|n| n.id == "rain").unwrap();
    graph_edit::insert_blend(rain, ni, 0, "upstream");
    rain.parameters.get_mut("upstream_factor").unwrap().value = 0.25;
    // Save and reload the edited package through its canonical JSON.
    let text = serde_json::to_string_pretty(edited.iter().find(|p| p.package_id == "core.environment").unwrap()).unwrap();
    let reloaded = sim_core::assets::parse_package(&text).unwrap();
    let mut edited2 = pkgs.clone();
    *edited2.iter_mut().find(|p| p.package_id == "core.environment").unwrap() = reloaded;
    let mut a = World::new(sc.clone(), edited, RunConfig::default()).unwrap();
    let mut b = World::new(sc.clone(), edited2, RunConfig::default()).unwrap();
    let mut c = World::new(sc, pkgs, RunConfig::default()).unwrap();
    a.run(40).unwrap();
    b.run(40).unwrap();
    c.run(40).unwrap();
    assert_eq!(a.state.hash(), b.state.hash(), "the reloaded law produces the same seeded results");
    // Documented effect: accepted rain inside 'upstream' is scaled by the factor.
    let e = a.plan().effects.iter().position(|e| e.id == "core.env.rain/rain").unwrap();
    let ec = c.plan().effects.iter().position(|e| e.id == "core.env.rain/rain").unwrap();
    let up = a.schema().region("upstream").unwrap();
    let mask = &a.state.regions[up];
    let cell = mask.iter().position(|m| *m == 1.0).unwrap();
    let ra = a.last.as_ref().unwrap().resolved.accepted[e][0][cell];
    let rc = c.last.as_ref().unwrap().resolved.accepted[ec][0][cell];
    assert!((ra - 0.25 * rc).abs() <= 1e-12 * rc.abs(), "{ra} vs 0.25 × {rc}");
    let outside = mask.iter().position(|m| *m == 0.0).unwrap();
    assert_eq!(
        a.last.as_ref().unwrap().resolved.accepted[e][0][outside],
        c.last.as_ref().unwrap().resolved.accepted[ec][0][outside]
    );
}

#[test]
fn t32_malformed_graphs_and_saves_are_rejected_without_panics() {
    let root = sim_core::assets::asset_root();
    let text = std::fs::read_to_string(root.join("rules/core/biology.json")).unwrap();
    let env_pkg = sim_core::assets::load_package(&root.join("rules/core/environment.json")).unwrap();
    let (sc, pkgs) = standard(16, 5);
    let cenv = sim_core::world::compile_env(&sc, sc.regions.iter().map(|r| r.id.clone()).collect());
    let mut g = Gen(77);
    let bytes = text.as_bytes();
    let mut rejected = 0;
    for _ in 0..400 {
        let mut b = bytes.to_vec();
        for _ in 0..1 + g.pick(6) {
            let i = g.pick(b.len());
            match g.pick(3) {
                0 => b[i] = b" {}[],:\"0123456789abcdefxyz-.e"[g.pick(29)],
                1 => {
                    b.remove(i);
                }
                _ => b.insert(i, b"\"[{0"[g.pick(4)]),
            }
        }
        let r = std::panic::catch_unwind(|| {
            let s = String::from_utf8_lossy(&b).to_string();
            match sim_core::assets::parse_package(&s) {
                Ok(p) => compiler::compile(&[env_pkg.clone(), p], &cenv).is_err(),
                Err(_) => true,
            }
        });
        assert!(r.is_ok(), "the loader or compiler panicked on malformed input");
        if r.unwrap() {
            rejected += 1;
        }
    }
    assert!(rejected > 300, "most mutations are rejected ({rejected}/400)");
    // Structurally valid JSON with hostile contents.
    let hostile = [
        json!({ "schema_version": 1, "package_id": "x", "rules": [{ "rule_id": "r", "domain": { "kind": "cells" }, "nodes": [{ "id": "n", "op": "neighbor_sum", "inputs": ["n"], "radius": 1e18 }] }] }),
        json!({ "schema_version": 1, "package_id": "x", "rules": [{ "rule_id": "r", "domain": { "kind": "cells" }, "nodes": [{ "id": "n", "op": "curve", "inputs": ["n"], "in_unit": "1", "out_unit": "1", "points": vec![[0.0, 0.0]; 100000] }] }] }),
        json!({ "schema_version": 99, "package_id": "../../etc", "rules": [] }),
        json!({ "schema_version": 1, "package_id": "x", "fields": [{ "id": "f", "unit": "kg^127*m^127", "policy": { "kind": "parameter" } }] }),
        json!({ "schema_version": 1, "package_id": "x", "rules": [{ "rule_id": "r", "domain": { "kind": "cells" }, "nodes": [{ "id": "n", "op": "const", "value": 1e308, "unit": "1" }] }] }),
    ];
    for h in hostile {
        let r = std::panic::catch_unwind(|| {
            let p: Result<Package, _> = serde_json::from_value(h);
            p.map(|p| compiler::compile(&[env_pkg.clone(), p], &cenv).is_err()).unwrap_or(true)
        });
        assert!(r.is_ok() && r.unwrap(), "hostile package must be rejected cleanly");
    }
    // Saves: truncation, byte flips, and absurd header lengths.
    let w = World::new(sc, pkgs, RunConfig::default()).unwrap();
    let save = persistence::save(&w);
    let mut huge = save.clone();
    huge[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(persistence::load(&huge, RunConfig::default()).is_err());
    for cut in [0, 7, 16, save.len() / 2, save.len() - 1] {
        assert!(persistence::load(&save[..cut], RunConfig::default()).is_err());
    }
    for k in 0..200 {
        let mut b = save.clone();
        let i = 8 + g.pick(b.len() - 8);
        b[i] ^= 1 << (k % 8);
        let r = std::panic::catch_unwind(|| persistence::load(&b, RunConfig::default()).map(|_| ()));
        assert!(r.is_ok(), "the save loader panicked on a corrupted file");
    }
}
