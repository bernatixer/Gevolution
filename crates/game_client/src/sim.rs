//! The simulation worker: owns the authoritative world, runs ticks at the
//! selected speed, manages checkpoints and branches, and publishes immutable
//! presentation snapshots. The client only sends commands; it never mutates
//! world buffers. Obsolete snapshots are replaced, never simulation ticks.

use sim_core::commands::CommandKind;
use sim_core::compiler::FieldPolicy;
use sim_core::inspect::{CellExplanation, EntityExplanation};
use sim_core::ir::{Dom, EffectKind};
use sim_core::schema::{Package, Scenario};
use sim_core::world::{Event, PhaseTimings, ResourceLedger, Sample, Stats, TickFailure};
use sim_core::{RunConfig, World, persistence};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Paused,
    X1,
    X10,
    Max,
}

pub enum ToSim {
    Speed(Speed),
    Step(u64),
    Submit(CommandKind),
    SelectCell(Option<usize>, String),
    SelectEntity(Option<(usize, u64)>),
    Save(PathBuf),
    Load(PathBuf),
    Checkpoint(String),
    BranchFrom(usize, String),
    SwitchBranch(usize),
    ClearFailure,
    Restart(Box<Scenario>, Vec<Package>),
    WantFlow(bool),
}

#[derive(Clone, Debug)]
pub struct EntityView {
    pub id: u64,
    pub x: f32,
    pub z: f32,
    pub heading: f32,
    pub root: u64,
    pub generation: u32,
    pub health: f32,
    pub size: f32,
    pub traits: Vec<f32>,
    pub origin: sim_core::state::Origin,
}

#[derive(Clone, Debug)]
pub struct ParamView {
    pub name: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub label: String,
}

/// Plan-level information that changes only when laws change.
#[derive(Clone, Debug)]
pub struct PlanInfo {
    pub source_hash: u64,
    pub params: Vec<ParamView>,
    pub cell_fields: Vec<(String, String, String)>,
    pub reservoir_fields: Vec<String>,
    pub regions: Vec<String>,
    pub archetypes: Vec<(String, Vec<(String, f64, f64)>)>,
    pub resources: Vec<String>,
    pub accounts: Vec<(String, String, String)>,
    pub effects: Vec<String>,
    pub instructions: usize,
    pub element_ops: u64,
    pub transient_bytes: u64,
    pub warnings: Vec<String>,
    pub packages: Vec<Package>,
    pub scenario: Scenario,
}

#[derive(Clone, Debug)]
pub struct CheckpointInfo {
    pub name: String,
    pub branch: usize,
    pub tick: u64,
}

#[derive(Clone, Debug)]
pub struct BranchSummary {
    pub index: usize,
    pub name: String,
    pub from_checkpoint: Option<String>,
    pub tick: u64,
    pub time: f64,
    pub seed: u64,
    pub population: usize,
    pub resource_totals: Vec<(String, f64)>,
    pub field_totals: Vec<(String, f64)>,
    pub trait_means: Vec<(String, f64)>,
    pub rules_changed: Vec<String>,
    pub history: Vec<Sample>,
    pub births: u64,
    pub deaths: u64,
}

pub struct Snapshot {
    pub tick: u64,
    pub time: f64,
    pub dt: f64,
    pub width: usize,
    pub height: usize,
    pub cell_size: f64,
    pub elevation: Arc<Vec<f32>>,
    /// Cell field values by schema order (f32 for presentation).
    pub fields: Vec<Vec<f32>>,
    pub regions: Vec<Vec<f32>>,
    pub entities: Vec<Vec<EntityView>>,
    /// Accepted signed flow per canonical edge (kg/s), when requested.
    pub flow: Option<Vec<f32>>,
    pub ledger: Vec<ResourceLedger>,
    pub stats: Stats,
    pub history: Vec<Sample>,
    pub events: Vec<Event>,
    pub failure: Option<TickFailure>,
    pub timings: PhaseTimings,
    pub pending: usize,
    pub plan: Arc<PlanInfo>,
    pub cell: Option<CellExplanation>,
    /// Bounded recent history of the selected cell value: (time, value, accepted in, accepted out).
    /// History of the selected spot, one sample per simulated second:
    /// [time s, surface water mm, soil moisture 0..1, plants kg, temperature °C].
    pub cell_history: Vec<[f64; 5]>,
    pub entity: Option<EntityExplanation>,
    pub speed: Speed,
    pub ticks_per_second: f64,
    pub checkpoints: Vec<CheckpointInfo>,
    pub branches: Vec<BranchSummary>,
    pub current_branch: usize,
    pub message: Option<String>,
    pub population: usize,
    /// (account, resource, label, cumulative in, cumulative out)
    pub accounts: Vec<(String, String, String, f64, f64)>,
    /// (resource, intervention in, intervention out, roundoff)
    pub interventions: Vec<(String, f64, f64, f64)>,
}

pub struct SimHandle {
    pub tx: Sender<ToSim>,
    pub latest: Arc<Mutex<Option<Snapshot>>>,
}

struct Checkpoint {
    name: String,
    branch: usize,
    tick: u64,
    bytes: Vec<u8>,
    packages: Vec<Package>,
}

struct Branch {
    name: String,
    from: Option<usize>,
    /// Saved state while this branch is not the active one.
    parked: Option<(Vec<u8>, VecDeque<Sample>)>,
    summary: Option<BranchSummary>,
}

struct Worker {
    world: World,
    speed: Speed,
    selected_cell: Option<(usize, String)>,
    selected_entity: Option<(usize, u64)>,
    want_flow: bool,
    plan_info: Arc<PlanInfo>,
    elevation: Arc<Vec<f32>>,
    checkpoints: Vec<Checkpoint>,
    branches: Vec<Branch>,
    current: usize,
    message: Option<String>,
    tick_times: VecDeque<Instant>,
    cell_history: VecDeque<[f64; 5]>,
}

pub fn spawn(scenario: Scenario, packages: Vec<Package>) -> Result<SimHandle, String> {
    let world = World::new(scenario, packages, RunConfig::default())?;
    let (tx, rx) = channel();
    let latest = Arc::new(Mutex::new(None));
    let out = latest.clone();
    std::thread::Builder::new()
        .name("simulation".into())
        .spawn(move || {
            let mut w = Worker::new(world);
            w.run(rx, out);
        })
        .map_err(|e| e.to_string())?;
    Ok(SimHandle { tx, latest })
}

fn plan_info(w: &World) -> PlanInfo {
    let plan = w.plan();
    let s = &plan.schema;
    PlanInfo {
        source_hash: plan.source_hash,
        params: plan
            .params
            .iter()
            .zip(&w.state.params)
            .map(|(p, v)| ParamView {
                name: p.name.clone(),
                value: *v,
                min: p.min,
                max: p.max,
                unit: p.unit.clone(),
                label: p.label.clone(),
            })
            .collect(),
        cell_fields: s
            .cell_fields
            .iter()
            .map(|f| {
                (
                    f.id.clone(),
                    f.unit.to_string(),
                    format!("{:?}", f.policy).split([' ', '{']).next().unwrap_or("").to_string(),
                )
            })
            .collect(),
        reservoir_fields: s
            .cell_fields
            .iter()
            .filter(|f| matches!(f.policy, FieldPolicy::Reservoir { .. }))
            .map(|f| f.id.clone())
            .collect(),
        regions: s.regions.clone(),
        archetypes: s
            .archetypes
            .iter()
            .map(|a| (a.id.clone(), a.traits.iter().map(|t| (t.id.clone(), t.min, t.max)).collect()))
            .collect(),
        resources: s.resources.iter().map(|r| r.id.clone()).collect(),
        accounts: s
            .accounts
            .iter()
            .map(|a| {
                (
                    a.id.clone(),
                    s.resources[a.resource].id.clone(),
                    if a.label.is_empty() { a.id.clone() } else { a.label.clone() },
                )
            })
            .collect(),
        effects: plan.effects.iter().map(|e| e.id.clone()).collect(),
        instructions: plan.instrs.len(),
        element_ops: plan.cost.element_ops,
        transient_bytes: plan.cost.transient_bytes,
        warnings: plan.warnings.iter().map(|d| d.to_string()).collect(),
        packages: w.active.packages.clone(),
        scenario: w.scenario.clone(),
    }
}

fn elevation(w: &World) -> Arc<Vec<f32>> {
    let f = w.schema().cell_field("elevation").unwrap();
    Arc::new(w.state.cells[f].iter().map(|v| *v as f32).collect())
}

fn rules_changed(before: &[Package], after: &[Package]) -> Vec<String> {
    let mut out = vec![];
    let index = |p: &[Package]| -> std::collections::BTreeMap<String, String> {
        p.iter()
            .flat_map(|p| {
                p.rules
                    .iter()
                    .map(|r| (r.rule_id.clone(), serde_json::to_string(r).unwrap_or_default()))
            })
            .collect()
    };
    let (a, b) = (index(before), index(after));
    for (k, v) in &b {
        match a.get(k) {
            None => out.push(format!("+ {k}")),
            Some(x) if x != v => out.push(format!("~ {k}")),
            _ => {}
        }
    }
    for k in a.keys() {
        if !b.contains_key(k) {
            out.push(format!("- {k}"));
        }
    }
    let params = |p: &[Package]| -> Vec<String> { p.iter().map(|p| serde_json::to_string(&p.parameters).unwrap_or_default()).collect() };
    if params(before) != params(after) {
        out.push("~ package parameters".into());
    }
    out
}

impl Worker {
    fn new(world: World) -> Worker {
        let plan_info = Arc::new(plan_info(&world));
        let elevation = elevation(&world);
        Worker {
            world,
            speed: Speed::Paused,
            selected_cell: None,
            selected_entity: None,
            want_flow: false,
            plan_info,
            elevation,
            checkpoints: vec![],
            branches: vec![Branch {
                name: "main".into(),
                from: None,
                parked: None,
                summary: None,
            }],
            current: 0,
            message: None,
            tick_times: VecDeque::new(),
            cell_history: VecDeque::new(),
        }
    }

    fn summary(&self, b: usize) -> BranchSummary {
        let w = &self.world;
        let s = w.schema();
        let br = &self.branches[b];
        let base_packages = br
            .from
            .map(|c| self.checkpoints[c].packages.clone())
            .unwrap_or_else(|| w.active.packages.clone());
        let mut trait_means = vec![];
        for (ai, a) in s.archetypes.iter().enumerate() {
            let e = &w.state.entities[ai];
            for (t, ti) in a.traits.iter().enumerate() {
                let m = if e.is_empty() {
                    f64::NAN
                } else {
                    (0..e.len()).map(|i| e.genome_of(i)[t]).sum::<f64>() / e.len() as f64
                };
                trait_means.push((format!("{}.{}", a.id, ti.id), m));
            }
        }
        BranchSummary {
            index: b,
            name: br.name.clone(),
            from_checkpoint: br.from.map(|c| self.checkpoints[c].name.clone()),
            tick: w.tick(),
            time: w.time(),
            seed: w.scenario.seed,
            population: w.population(),
            resource_totals: w.ledger.iter().map(|l| (l.resource.clone(), l.total)).collect(),
            field_totals: s
                .cell_fields
                .iter()
                .enumerate()
                .filter(|(_, f)| matches!(f.policy, FieldPolicy::Reservoir { .. }))
                .map(|(i, f)| (f.id.clone(), sim_core::state::compensated_sum(&w.state.cells[i])))
                .collect(),
            trait_means,
            rules_changed: rules_changed(&base_packages, &w.active.packages),
            history: w.history.iter().cloned().collect(),
            births: w.stats.births,
            deaths: w.stats.deaths,
        }
    }

    fn handle(&mut self, m: ToSim) {
        match m {
            ToSim::Speed(s) => self.speed = s,
            ToSim::Step(n) => {
                self.speed = Speed::Paused;
                for _ in 0..n {
                    if self.tick_once().is_err() {
                        break;
                    }
                }
            }
            ToSim::Submit(c) => {
                self.world.submit(c, None);
            }
            ToSim::SelectCell(c, f) => {
                self.selected_cell = c.map(|c| (c, f));
                self.cell_history.clear();
            }
            ToSim::SelectEntity(e) => self.selected_entity = e,
            ToSim::WantFlow(b) => self.want_flow = b,
            ToSim::ClearFailure => self.world.clear_failure(),
            ToSim::Save(p) => {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                self.message = Some(match persistence::save_to(&self.world, &p) {
                    Ok(()) => format!("saved {}", p.display()),
                    Err(e) => format!("save failed: {e}"),
                });
            }
            ToSim::Load(p) => match persistence::load_from(&p, RunConfig::default()) {
                Ok(w) => {
                    self.replace_world(w);
                    self.message = Some(format!("loaded {}", p.display()));
                }
                Err(e) => self.message = Some(format!("load failed: {e}")),
            },
            ToSim::Checkpoint(name) => {
                let bytes = persistence::save(&self.world);
                self.checkpoints.push(Checkpoint {
                    name: name.clone(),
                    branch: self.current,
                    tick: self.world.tick(),
                    bytes,
                    packages: self.world.active.packages.clone(),
                });
                self.message = Some(format!("checkpoint '{name}' at tick {}", self.world.tick()));
            }
            ToSim::BranchFrom(ci, name) => {
                if ci >= self.checkpoints.len() {
                    return;
                }
                let bytes = self.checkpoints[ci].bytes.clone();
                match persistence::load(&bytes, RunConfig::default()) {
                    Ok(w) => {
                        self.park_current();
                        self.branches.push(Branch {
                            name: name.clone(),
                            from: Some(ci),
                            parked: None,
                            summary: None,
                        });
                        self.current = self.branches.len() - 1;
                        self.replace_world(w);
                        self.message = Some(format!("branch '{name}' from checkpoint '{}'", self.checkpoints[ci].name));
                    }
                    Err(e) => self.message = Some(format!("branch failed: {e}")),
                }
            }
            ToSim::SwitchBranch(b) => {
                if b == self.current || b >= self.branches.len() {
                    return;
                }
                let Some((bytes, history)) = self.branches[b].parked.take() else {
                    return;
                };
                match persistence::load(&bytes, RunConfig::default()) {
                    Ok(mut w) => {
                        self.park_current();
                        w.history = history;
                        self.current = b;
                        self.replace_world(w);
                        self.message = Some(format!("switched to branch '{}'", self.branches[b].name));
                    }
                    Err(e) => self.message = Some(format!("switch failed: {e}")),
                }
            }
            ToSim::Restart(sc, pkgs) => match World::new(*sc, pkgs, RunConfig::default()) {
                Ok(w) => {
                    self.checkpoints.clear();
                    self.branches = vec![Branch {
                        name: "main".into(),
                        from: None,
                        parked: None,
                        summary: None,
                    }];
                    self.current = 0;
                    self.replace_world(w);
                    self.message = Some("new world".into());
                }
                Err(e) => self.message = Some(format!("world creation failed: {e}")),
            },
        }
    }

    fn park_current(&mut self) {
        let summary = self.summary(self.current);
        let bytes = persistence::save(&self.world);
        let b = &mut self.branches[self.current];
        b.summary = Some(summary);
        b.parked = Some((bytes, self.world.history.clone()));
    }

    fn replace_world(&mut self, w: World) {
        self.world = w;
        self.plan_info = Arc::new(plan_info(&self.world));
        self.elevation = elevation(&self.world);
        self.selected_entity = None;
        self.speed = Speed::Paused;
    }

    fn tick_once(&mut self) -> Result<(), ()> {
        let before = self.world.plan().source_hash;
        let r = self.world.step();
        if r.is_ok()
            && let Some((c, _)) = &self.selected_cell
            && self.world.tick().is_multiple_of(4)
        {
            let w = &self.world;
            let s = w.schema();
            let get = |f: &str| s.cell_field(f).map(|i| w.state.cells[i][*c]).unwrap_or(0.0);
            let cap = get("soil_water_capacity");
            self.cell_history.push_back([
                w.time(),
                get("surface_water") / (w.grid.cell_size * w.grid.cell_size),
                if cap > 0.0 { get("soil_water") / cap } else { 0.0 },
                get("vegetation_biomass"),
                get("temperature") - 273.15,
            ]);
            while self.cell_history.len() > 600 {
                self.cell_history.pop_front();
            }
        }
        if r.is_ok() {
            self.tick_times.push_back(Instant::now());
            while self.tick_times.len() > 200 {
                self.tick_times.pop_front();
            }
        }
        let laws_changed = self.world.plan().source_hash != before
            || self.world.plan().schema.regions.len() != self.plan_info.regions.len()
            || self.world.state.params.iter().ne(self.plan_info.params.iter().map(|p| &p.value));
        if laws_changed {
            self.plan_info = Arc::new(plan_info(&self.world));
        }
        if r.is_err() {
            self.speed = Speed::Paused;
        }
        r.map_err(|_| ())
    }

    fn snapshot(&self) -> Snapshot {
        let w = &self.world;
        let s = w.schema();
        let flow = if self.want_flow {
            w.last.as_ref().and_then(|last| {
                let e =
                    last.active.plan.effects.iter().position(|e| {
                        matches!(e.kind, EffectKind::EdgeTransfer { .. }) && e.dom == Dom::Edges(0) && e.id.contains("flow")
                    })?;
                let acc = last.resolved.accepted[e].first()?;
                let dir = &last.resolved.edge_dir[e];
                Some(
                    acc.iter()
                        .zip(dir)
                        .map(|(a, d)| (if *d { *a } else { -*a } / w.scenario.dt) as f32)
                        .collect(),
                )
            })
        } else {
            None
        };
        let entities = s
            .archetypes
            .iter()
            .enumerate()
            .map(|(ai, a)| {
                let e = &w.state.entities[ai];
                let hi = a.field("health");
                let si = a.traits.iter().position(|t| t.id == "body_size");
                (0..e.len())
                    .map(|i| EntityView {
                        id: e.ids[i],
                        x: e.x[i] as f32,
                        z: e.z[i] as f32,
                        heading: e.heading[i] as f32,
                        root: e.lineage[i].root,
                        generation: e.lineage[i].generation,
                        health: hi.map(|h| e.fields[h][i] as f32).unwrap_or(1.0),
                        size: si.map(|t| e.genome_of(i)[t] as f32).unwrap_or(1.0),
                        traits: e.genome_of(i)[..a.traits.len()].iter().map(|v| *v as f32).collect(),
                        origin: e.lineage[i].origin,
                    })
                    .collect()
            })
            .collect();
        let tps = match (self.tick_times.front(), self.tick_times.back()) {
            (Some(a), Some(b)) if self.tick_times.len() > 1 && *b > *a => (self.tick_times.len() - 1) as f64 / (*b - *a).as_secs_f64(),
            _ => 0.0,
        };
        let mut branches: Vec<BranchSummary> = self.branches.iter().filter_map(|b| b.summary.clone()).collect();
        let cur = self.summary(self.current);
        branches.retain(|b| b.index != cur.index);
        branches.insert(0, cur);
        Snapshot {
            tick: w.tick(),
            time: w.time(),
            dt: w.scenario.dt,
            width: w.grid.width,
            height: w.grid.height,
            cell_size: w.grid.cell_size,
            elevation: self.elevation.clone(),
            fields: w.state.cells.iter().map(|c| c.iter().map(|v| *v as f32).collect()).collect(),
            regions: w.state.regions.iter().map(|c| c.iter().map(|v| *v as f32).collect()).collect(),
            entities,
            flow,
            ledger: w.ledger.clone(),
            stats: w.stats.clone(),
            history: w.history.iter().rev().take(1024).rev().cloned().collect(),
            events: w.events.iter().rev().take(80).cloned().collect(),
            failure: w.failure.clone(),
            timings: w.timings.clone(),
            pending: w.pending.len(),
            plan: self.plan_info.clone(),
            cell: self.selected_cell.as_ref().and_then(|(c, f)| w.explain_cell(f, *c)),
            cell_history: self.cell_history.iter().copied().collect(),
            entity: self.selected_entity.and_then(|(a, id)| w.explain_entity(a, id)),
            speed: self.speed,
            ticks_per_second: tps,
            checkpoints: self
                .checkpoints
                .iter()
                .map(|c| CheckpointInfo {
                    name: c.name.clone(),
                    branch: c.branch,
                    tick: c.tick,
                })
                .collect(),
            branches,
            current_branch: self.current,
            message: self.message.clone(),
            population: w.population(),
            accounts: s
                .accounts
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    (
                        a.id.clone(),
                        s.resources[a.resource].id.clone(),
                        a.label.clone(),
                        w.state.account_in[i],
                        w.state.account_out[i],
                    )
                })
                .collect(),
            interventions: s
                .resources
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    (
                        r.id.clone(),
                        w.state.intervention_in[i],
                        w.state.intervention_out[i],
                        w.state.roundoff[i],
                    )
                })
                .collect(),
        }
    }

    fn run(&mut self, rx: Receiver<ToSim>, out: Arc<Mutex<Option<Snapshot>>>) {
        let mut last_publish = Instant::now() - Duration::from_secs(1);
        let mut clock = Instant::now();
        let mut due = 0.0f64;
        let mut dirty = true;
        loop {
            let wait = if self.speed == Speed::Paused {
                Duration::from_millis(30)
            } else {
                Duration::from_millis(0)
            };
            match rx.recv_timeout(wait) {
                Ok(m) => {
                    self.handle(m);
                    dirty = true;
                    while let Ok(m) = rx.try_recv() {
                        self.handle(m);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                Err(_) => {}
            }
            let now = Instant::now();
            let elapsed = (now - clock).as_secs_f64();
            clock = now;
            let rate = match self.speed {
                Speed::Paused => 0.0,
                Speed::X1 => 1.0 / self.world.scenario.dt,
                Speed::X10 => 10.0 / self.world.scenario.dt,
                Speed::Max => f64::INFINITY,
            };
            if rate > 0.0 && self.world.failure.is_none() {
                let budget = Instant::now() + Duration::from_millis(30);
                if rate.is_finite() {
                    due = (due + elapsed * rate).min(rate * 0.25);
                    while due >= 1.0 && Instant::now() < budget {
                        due -= 1.0;
                        if self.tick_once().is_err() {
                            break;
                        }
                        dirty = true;
                    }
                    if due < 1.0 {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                } else {
                    while Instant::now() < budget {
                        if self.tick_once().is_err() {
                            break;
                        }
                        dirty = true;
                    }
                }
            }
            if dirty && last_publish.elapsed() >= Duration::from_millis(33) {
                let snap = self.snapshot();
                self.message = None;
                *out.lock().unwrap() = Some(snap);
                last_publish = Instant::now();
                dirty = false;
            }
        }
    }
}
