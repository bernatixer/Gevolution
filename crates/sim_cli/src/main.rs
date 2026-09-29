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
            s => scenario = Some(PathBuf::from(s)),
        }
        i += 1;
    }
    let path =
        scenario.unwrap_or_else(|| assets::asset_root().join("scenarios/seasonal_river.json"));
    let (sc, pkgs) = assets::load_scenario(&path).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2)
    });
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
    let mut times = vec![];
    for t in 0..ticks {
        let s = Instant::now();
        if let Err(f) = w.step() {
            eprintln!("{f}");
            std::process::exit(1);
        }
        times.push(s.elapsed().as_secs_f64() * 1e3);
        if (t + 1) % every == 0 || t + 1 == ticks {
            report(&w);
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| times[((times.len() as f64 - 1.0) * q) as usize];
    println!(
        "tick ms: median {:.2}  p95 {:.2}  max {:.2}",
        p(0.5),
        p(0.95),
        p(1.0)
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
    if !w.stats.deaths_by_reason.is_empty() {
        println!("    deaths by reason: {:?}", w.stats.deaths_by_reason);
    }
}
