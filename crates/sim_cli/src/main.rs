//! Headless runner: loads the same scenario and packages as the client,
//! executes the same runtime, and prints metrics without a renderer.

use sim_core::eval::Mode;
use sim_core::{RunConfig, World, assets};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut scenario = None;
    let mut ticks: u64 = 400;
    let mut every: u64 = 400;
    let mut config = RunConfig::default();
    let mut budget = false;
    let mut sets: Vec<(u64, String, f64)> = vec![];
    let mut size: Option<usize> = None;
    let mut no_organisms = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ticks" => {
                i += 1;
                ticks = args[i].parse().expect("--ticks N");
            }
            "--every" => {
                i += 1;
                every = args[i].parse().expect("--every N");
            }
            "--threads" => {
                i += 1;
                config.threads = Some(args[i].parse().expect("--threads N"));
            }
            "--reference" => {
                config.mode = Mode::Reference;
                config.optimize = false;
            }
            "--no-optimize" => config.optimize = false,
            "--budget" => budget = true,
            "--set" => {
                // --set TICK:qualified.param=value
                i += 1;
                let (t, rest) = args[i].split_once(':').expect("--set TICK:name=value");
                let (n, v) = rest.split_once('=').expect("--set TICK:name=value");
                sets.push((t.parse::<u64>().expect("tick"), n.to_string(), v.parse::<f64>().expect("value")));
            }
            "--size" => {
                i += 1;
                size = Some(args[i].parse().expect("--size N"));
            }
            "--no-organisms" => no_organisms = true,
            s => scenario = Some(PathBuf::from(s)),
        }
        i += 1;
    }
    let path = scenario.unwrap_or_else(|| assets::asset_root().join("scenarios/seasonal_river.json"));
    let (mut sc, pkgs) = assets::load_scenario(&path).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2)
    });
    if let Some(n) = size {
        let (w0, h0) = (sc.grid.width, sc.grid.height);
        sc.grid.width = n;
        sc.grid.height = n;
        for r in &mut sc.regions {
            if let sim_core::schema::RegionShape::Rect { x0, z0, x1, z1 } = &mut r.shape {
                *x0 = *x0 * n / w0;
                *x1 = *x1 * n / w0;
                *z0 = *z0 * n / h0;
                *z1 = *z1 * n / h0;
            }
            if let sim_core::schema::RegionShape::Circle { cx, cz, radius, .. } = &mut r.shape {
                *cx *= n as f64 / w0 as f64;
                *cz *= n as f64 / h0 as f64;
                *radius *= n as f64 / w0 as f64;
            }
        }
    }
    if no_organisms {
        sc.populations.clear();
    }
    let t0 = Instant::now();
    let mut w = World::new(sc, pkgs, config).unwrap_or_else(|e| {
        eprintln!("world creation failed:\n{e}");
        std::process::exit(2)
    });
    println!(
        "compiled {} instructions, {} effects, {} slots in {:.1} ms; build {}",
        w.plan().instrs.len(),
        w.plan().effects.len(),
        w.plan().slots.len(),
        t0.elapsed().as_secs_f64() * 1e3,
        sim_core::world::build_fingerprint()
    );
    for d in &w.plan().warnings {
        println!("  warning: {d}");
    }
    for (t, n, v) in sets {
        w.submit(sim_core::commands::CommandKind::SetParam { name: n, value: v }, Some(t));
    }
    let mut times = vec![];
    let mut phase = sim_core::world::PhaseTimings::default();
    for t in 0..ticks {
        let s = Instant::now();
        if let Err(f) = w.step() {
            eprintln!("{f}");
            std::process::exit(1);
        }
        times.push(s.elapsed().as_secs_f64() * 1e3);
        let p = &w.timings;
        for (acc, v) in [
            (&mut phase.commands, p.commands),
            (&mut phase.snapshot, p.snapshot),
            (&mut phase.evaluation, p.evaluation),
            (&mut phase.resolution, p.resolution),
            (&mut phase.receipts, p.receipts),
            (&mut phase.integration, p.integration),
            (&mut phase.lifecycle, p.lifecycle),
            (&mut phase.validation, p.validation),
            (&mut phase.commit, p.commit),
        ] {
            *acc += v / ticks as f64;
        }
        if (t + 1) % every == 0 || t + 1 == ticks {
            report(&w);
            if budget {
                budget_report(&w);
            }
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| times[((times.len() as f64 - 1.0) * q) as usize];
    println!("tick ms: median {:.2}  p95 {:.2}  max {:.2}", p(0.5), p(0.95), p(1.0));
    println!(
        "mean phase ms: commands {:.2} snapshot {:.2} eval {:.2} resolve {:.2} receipts {:.2} integrate {:.2} lifecycle {:.2} validate {:.2} commit {:.2}",
        phase.commands,
        phase.snapshot,
        phase.evaluation,
        phase.resolution,
        phase.receipts,
        phase.integration,
        phase.lifecycle,
        phase.validation,
        phase.commit
    );
    println!("state hash {:016x}", w.state.hash());
}

fn report(w: &World) {
    let s = w.schema();
    let tot = |f: &str| {
        s.cell_field(f)
            .map(|i| sim_core::state::compensated_sum(&w.state.cells[i]))
            .unwrap_or(0.0)
    };
    let n = w.grid.cells() as f64;
    let wet = s
        .cell_field("surface_water")
        .map(|i| w.state.cells[i].iter().filter(|v| **v > 500.0).count())
        .unwrap_or(0);
    println!(
        "t={:7.1}s tick={:6} pop={:5} births={:5} deaths={:5} | surface {:.3e} soil {:.3e} ground {:.3e} kg (wet cells {wet}) | veg {:.1} kg/cell | T {:.2} K",
        w.time(),
        w.tick(),
        w.population(),
        w.stats.births,
        w.stats.deaths,
        tot("surface_water"),
        tot("soil_water"),
        tot("groundwater"),
        tot("vegetation_biomass") / n,
        tot("temperature") / n
    );
    for l in &w.ledger {
        println!(
            "    {:9} total {:.6e}  tick err {:+.2e} (tol {:.1e})  cumulative err {:+.2e}",
            l.resource, l.total, l.last_tick_error, l.last_tolerance, l.cumulative_error
        );
    }
    for (ai, a) in s.archetypes.iter().enumerate() {
        let e = &w.state.entities[ai];
        if e.is_empty() {
            continue;
        }
        let m = e.len() as f64;
        let fields: Vec<String> = a
            .fields
            .iter()
            .enumerate()
            .map(|(f, fi)| format!("{}={:.3}", fi.id, e.fields[f].iter().sum::<f64>() / m))
            .collect();
        let traits: Vec<String> = a
            .traits
            .iter()
            .enumerate()
            .map(|(t, ti)| format!("{}={:.3}", ti.id, (0..e.len()).map(|i| e.genome_of(i)[t]).sum::<f64>() / m))
            .collect();
        let generation = e.lineage.iter().map(|l| l.generation as f64).sum::<f64>() / m;
        let age = e.age.iter().sum::<f64>() / m;
        println!(
            "    {} mean: {} | age={age:.0}s gen={generation:.2}\n      traits: {}",
            a.id,
            fields.join(" "),
            traits.join(" ")
        );
    }
    if !w.stats.deaths_by_reason.is_empty() {
        println!("    deaths by reason: {:?}", w.stats.deaths_by_reason);
    }
}

/// Accepted amount per process over the last tick, summed over the world, per second.
fn budget_report(w: &World) {
    let Some(last) = &w.last else { return };
    let plan = &last.active.plan;
    let dt = w.scenario.dt;
    for (e, eff) in plan.effects.iter().enumerate() {
        let acc = &last.resolved.accepted[e];
        if acc.is_empty() {
            continue;
        }
        let req = &last.resolved.receipts.requested[e];
        let legs: Vec<String> = acc
            .iter()
            .zip(req)
            .map(|(a, r)| format!("{:.3e}/{:.3e}", a.iter().sum::<f64>() / dt, r.iter().sum::<f64>() / dt))
            .collect();
        println!("      {:48} accepted/requested per s: {}", eff.id, legs.join("  "));
    }
}
