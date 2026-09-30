//! Headless runner: loads the same scenario and packages as the client,
//! executes the same runtime, and prints metrics without a renderer.

use sim_core::eval::Mode;
use sim_core::{RunConfig, World, assets};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("bench") {
        bench(&args[1..]);
        return;
    }
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

fn scale_scenario(sc: &mut sim_core::schema::Scenario, n: usize) {
    let (w0, h0) = (sc.grid.width, sc.grid.height);
    sc.grid.width = n;
    sc.grid.height = n;
    for r in &mut sc.regions {
        match &mut r.shape {
            sim_core::schema::RegionShape::Rect { x0, z0, x1, z1 } => {
                *x0 = *x0 * n / w0;
                *x1 = *x1 * n / w0;
                *z0 = *z0 * n / h0;
                *z1 = *z1 * n / h0;
            }
            sim_core::schema::RegionShape::Circle { cx, cz, radius, .. } => {
                *cx *= n as f64 / w0 as f64;
                *cz *= n as f64 / h0 as f64;
                *radius *= n as f64 / w0 as f64;
            }
            _ => {}
        }
    }
}

fn rss_mib() -> f64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok())
        .map(|kb| kb / 1024.0)
        .unwrap_or(f64::NAN)
}

fn hardware() -> String {
    let q = |k: &str| {
        std::process::Command::new("sysctl")
            .args(["-n", k])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let cpu = q("machdep.cpu.brand_string");
    let cores = q("hw.ncpu");
    let mem = q("hw.memsize")
        .parse::<f64>()
        .map(|b| format!("{:.0} GiB", b / (1u64 << 30) as f64))
        .unwrap_or_default();
    if cpu.is_empty() {
        std::env::consts::ARCH.to_string()
    } else {
        format!("{cpu}, {cores} logical cores, {mem}")
    }
}

/// Run a benchmark manifest and print/emit a reproducible report.
fn bench(args: &[String]) {
    let Some(path) = args.first() else {
        eprintln!("usage: sim_cli bench <manifest.json> [--out report.json] [--threads N]");
        std::process::exit(2);
    };
    let mut out_path = None;
    let mut threads = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out_path = args.get(i).cloned();
            }
            "--threads" => {
                i += 1;
                threads = args.get(i).and_then(|s| s.parse().ok());
            }
            _ => {}
        }
        i += 1;
    }
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("manifest")).expect("manifest json");
    let get = |k: &str| m.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let root = assets::asset_root();
    let (mut sc, pkgs) = assets::load_scenario(&root.join(m["scenario"].as_str().unwrap())).expect("scenario");
    scale_scenario(&mut sc, get("size") as usize);
    sc.seed = get("seed");
    sc.population_cap = get("population_cap") as usize;
    sc.populations[0].count = get("organisms") as usize;
    if get("organisms") == 0 {
        sc.populations.clear();
    }
    let env = sim_core::world::compile_env(&sc, sc.regions.iter().map(|r| r.id.clone()).collect());
    let t = Instant::now();
    let plan = sim_core::compiler::compile(&pkgs, &env).expect("compile");
    let plan = sim_core::optimize::optimize(plan);
    let compile_ms = t.elapsed().as_secs_f64() * 1e3;
    let nodes = plan.instrs.len();
    let t = Instant::now();
    let mut w = World::new(
        sc.clone(),
        pkgs.clone(),
        RunConfig {
            threads,
            sample_interval: 0,
            ..RunConfig::default()
        },
    )
    .expect("world");
    let init_ms = t.elapsed().as_secs_f64() * 1e3;
    // Hot-swap latency: recompile and migrate the same packages at a tick boundary.
    w.submit(sim_core::commands::CommandKind::ApplyPackages { packages: pkgs.clone() }, None);
    let t = Instant::now();
    w.step().expect("tick");
    let hotswap_ms = t.elapsed().as_secs_f64() * 1e3;
    for _ in 1..get("warmup_ticks") {
        w.step().expect("tick");
    }
    let n = get("measured_ticks") as usize;
    let (mut ticks, mut resolve, mut eval, mut brain, mut snap, mut processes) = (vec![], 0.0, 0.0, 0.0, 0.0, 0u64);
    let mut peak = rss_mib();
    for k in 0..n {
        let t = Instant::now();
        w.step().expect("tick");
        ticks.push(t.elapsed().as_secs_f64() * 1e3);
        resolve += w.timings.resolution;
        eval += w.timings.evaluation;
        brain += w.timings.brain;
        if let Some(last) = &w.last {
            processes += last
                .resolved
                .accepted
                .iter()
                .map(|e| e.iter().map(|l| l.len() as u64).sum::<u64>())
                .sum::<u64>();
        }
        // Presentation snapshot cost: f32 render fields and entity transforms.
        let t = Instant::now();
        let fields: Vec<Vec<f32>> = w.state.cells.iter().map(|c| c.iter().map(|v| *v as f32).collect()).collect();
        let ents: Vec<(f32, f32, f32)> = w
            .state
            .entities
            .iter()
            .flat_map(|e| (0..e.len()).map(|i| (e.x[i] as f32, e.z[i] as f32, e.heading[i] as f32)))
            .collect();
        std::hint::black_box((fields, ents));
        snap += t.elapsed().as_secs_f64() * 1e3;
        if k % 100 == 0 {
            peak = peak.max(rss_mib());
        }
    }
    peak = peak.max(rss_mib());
    let mut sorted = ticks.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
    let nf = n as f64;
    let total_s: f64 = ticks.iter().sum::<f64>() / 1e3;
    let report = serde_json::json!({
        "manifest": m["name"],
        "hardware": hardware(),
        "build": sim_core::world::build_fingerprint(),
        "package_hash": format!("{:016x}", w.plan().source_hash),
        "seed": sc.seed,
        "grid": [sc.grid.width, sc.grid.height],
        "initial_organisms": get("organisms"),
        "final_organisms": w.population(),
        "threads": threads.map(|t| t.to_string()).unwrap_or("all".into()),
        "warmup_ticks": get("warmup_ticks"),
        "measured_ticks": n,
        "tick_ms": { "median": q(0.5), "p95": q(0.95), "max": q(1.0) },
        "mean_ms": { "evaluation": eval / nf, "brain": brain / nf, "resolution": resolve / nf, "snapshot": snap / nf },
        "compile_ms": compile_ms,
        "hot_swap_tick_ms": hotswap_ms,
        "world_init_ms": init_ms,
        "plan_instructions": nodes,
        "accepted_process_elements_per_s": processes as f64 / total_s,
        "peak_resident_mib": peak,
        "state_hash": format!("{:016x}", w.state.hash()),
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if m["check_targets"].as_bool() == Some(true) {
        let p95 = q(0.95);
        let ok_tick = p95 <= 25.0;
        let ok_mem = peak < 1024.0;
        let ok_compile = compile_ms <= 250.0;
        println!(
            "targets: p95 {p95:.2} ms <= 25 ms: {}; resident {peak:.0} MiB < 1024 MiB: {}; plan {compile_ms:.1} ms <= 250 ms: {}",
            if ok_tick { "PASS" } else { "FAIL" },
            if ok_mem { "PASS" } else { "FAIL" },
            if ok_compile { "PASS" } else { "FAIL" }
        );
        if !(ok_tick && ok_mem && ok_compile) {
            std::process::exit(1);
        }
    }
    if let Some(p) = out_path {
        std::fs::write(&p, serde_json::to_string_pretty(&report).unwrap()).expect("write report");
    }
}
