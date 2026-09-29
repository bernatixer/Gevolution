//! The authoritative world and its tick.
//!
//! Phases (versioned, part of the language contract):
//! 0 boundary commands, 1 snapshot + forcing, 2 pure evaluation, 3 proposals,
//! 4 resolution, 5 receipt outcomes, 6 candidate integration, 7 lifecycle,
//! 8 validation and commit. A failed tick publishes nothing.

use crate::biology;
use crate::commands::{Command, CommandKind};
use crate::compiler::{self, CompileEnv, Diagnostic, FieldPolicy, Plan, Schema};
use crate::eval::{self, Buffers, EvalInputs, Mode, SpatialIndex};
use crate::grid::Grid;
use crate::ir::*;
use crate::optimize;
use crate::resolver::{self, ResolveInputs, Resolved, ResolverIndex};
use crate::rng;
use crate::schema::*;
use crate::state::{Entities, KahanSum, Lineage, Origin, State};
use crate::weather::Weather;
use crate::worldgen;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PHASE_CONTRACT: &str = "phases.v1";
pub const NUMERICAL_PROFILE: &str = "strict-reference.f64.v1";
const STREAM_BIRTH_ORDER: u64 = 0xB1_0001;
const STREAM_BIRTH_PLACE: u64 = 0xB1_0002;
pub const MAX_EVENTS: usize = 4096;
pub const MAX_HISTORY: usize = 4096;

/// Identifies the build and numerical profile for replay compatibility.
pub fn build_fingerprint() -> String {
    format!(
        "sim_core-{ENGINE_VERSION}/{PHASE_CONTRACT}/{NUMERICAL_PROFILE}/{}/{}/{}-{}",
        rng::RNG_VERSION,
        resolver::ALLOCATION_POLICY,
        std::env::consts::ARCH,
        std::env::consts::OS
    )
}

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub mode: Mode,
    pub optimize: bool,
    /// Worker threads; None uses the global pool.
    pub threads: Option<usize>,
    /// Record population/trait samples every N ticks (0 disables).
    pub sample_interval: u64,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            mode: Mode::Parallel,
            optimize: true,
            threads: None,
            sample_interval: 40,
        }
    }
}

impl RunConfig {
    pub fn reference() -> RunConfig {
        RunConfig {
            mode: Mode::Reference,
            optimize: false,
            threads: Some(1),
            sample_interval: 40,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TickFailure {
    pub tick: u64,
    pub phase: &'static str,
    pub message: String,
    pub effect: Option<String>,
    /// Source nodes implicated (e.g. first non-finite value).
    pub nodes: Vec<String>,
    pub elements: Vec<usize>,
    pub plan_hash: u64,
}

impl std::fmt::Display for TickFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tick {} failed in {}: {}",
            self.tick, self.phase, self.message
        )?;
        if let Some(e) = &self.effect {
            write!(f, " (effect {e})")?;
        }
        if !self.nodes.is_empty() {
            write!(f, " (nodes {})", self.nodes.join(", "))?;
        }
        if !self.elements.is_empty() {
            write!(f, " (elements {:?})", self.elements)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub enum EventKind {
    Birth {
        arch: String,
        id: u64,
        parent: u64,
    },
    Death {
        arch: String,
        id: u64,
        reason: String,
    },
    CommandApplied {
        seq: u64,
        summary: String,
    },
    CommandRejected {
        seq: u64,
        reason: String,
    },
    PopulationCap {
        rejected: usize,
    },
    RangeWarning {
        field: String,
        count: usize,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub tick: u64,
    pub kind: EventKind,
}

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
pub struct ResourceLedger {
    pub resource: String,
    pub initial: f64,
    pub total: f64,
    pub cumulative_in: f64,
    pub cumulative_out: f64,
    pub cumulative_abs_exchange: f64,
    pub last_tick_error: f64,
    pub max_tick_error: f64,
    pub last_tolerance: f64,
    /// total - (initial + in - out).
    pub cumulative_error: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sample {
    pub tick: u64,
    pub population: Vec<usize>,
    /// Per archetype, per trait: mean.
    pub trait_means: Vec<Vec<f64>>,
    /// Per archetype, per trait: 10-bin histogram over declared range.
    pub trait_hist: Vec<Vec<[u32; 10]>>,
    pub resource_totals: Vec<f64>,
    /// Sum of each cell field (for charts).
    pub field_totals: Vec<f64>,
    pub births: u64,
    pub deaths: u64,
    pub mean_generation: Vec<f64>,
}

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
pub struct Stats {
    pub births: u64,
    pub deaths: u64,
    pub deaths_by_reason: BTreeMap<String, u64>,
    pub cap_rejections: u64,
    pub range_violations: BTreeMap<String, usize>,
}

/// Wiring of non-resource effects to state fields, in canonical order.
#[derive(Clone, Debug, Default)]
pub struct Wiring {
    pub rate_cell: Vec<Vec<(Slot, Option<Slot>)>>,
    pub next_cell: Vec<Option<Slot>>,
    pub rate_ent: Vec<Vec<Vec<(Slot, Option<Slot>)>>>,
    pub next_ent: Vec<Vec<Option<Slot>>>,
    pub moves: Vec<Option<(Slot, Slot)>>,
    pub deaths: Vec<Vec<(Slot, String)>>,
    pub births: Vec<Option<(Slot, Vec<(usize, Slot)>)>>,
}

impl Wiring {
    pub fn build(plan: &Plan) -> Wiring {
        let s = &plan.schema;
        let na = s.archetypes.len();
        let mut w = Wiring {
            rate_cell: vec![vec![]; s.cell_fields.len()],
            next_cell: vec![None; s.cell_fields.len()],
            rate_ent: s
                .archetypes
                .iter()
                .map(|a| vec![vec![]; a.fields.len()])
                .collect(),
            next_ent: s
                .archetypes
                .iter()
                .map(|a| vec![None; a.fields.len()])
                .collect(),
            moves: vec![None; na],
            deaths: vec![vec![]; na],
            births: vec![None; na],
        };
        for e in &plan.effects {
            match &e.kind {
                EffectKind::RateCell { field, rate } => {
                    w.rate_cell[*field].push((*rate, e.coverage))
                }
                EffectKind::NextCell { field, value } => w.next_cell[*field] = Some(*value),
                EffectKind::RateEntity { arch, field, rate } => {
                    w.rate_ent[*arch as usize][*field].push((*rate, e.coverage))
                }
                EffectKind::NextEntity { arch, field, value } => {
                    w.next_ent[*arch as usize][*field] = Some(*value)
                }
                EffectKind::Move { arch, turn, speed } => {
                    w.moves[*arch as usize] = Some((*turn, *speed))
                }
                EffectKind::Death {
                    arch,
                    condition,
                    reason,
                } => w.deaths[*arch as usize].push((*condition, reason.clone())),
                EffectKind::Birth {
                    arch,
                    condition,
                    legs,
                } => w.births[*arch as usize] = Some((*condition, legs.clone())),
                _ => {}
            }
        }
        w
    }
}

/// The active law: packages, compiled plan, and derived execution metadata.
#[derive(Clone)]
pub struct Active {
    pub packages: Vec<Package>,
    pub plan: Arc<Plan>,
    pub index: Arc<ResolverIndex>,
    pub groups: Arc<Vec<(Stage, Vec<usize>)>>,
    pub wiring: Arc<Wiring>,
    pub weather: Arc<Weather>,
}

/// Retained data about the last committed tick, for inspection only.
pub struct LastTick {
    pub resolved: Resolved,
    pub spatial: Vec<SpatialIndex>,
    pub active: Active,
}

pub struct World {
    pub scenario: Scenario,
    pub active: Active,
    pub grid: Grid,
    pub state: State,
    /// Snapshot S_n of the last committed tick (for explanation).
    pub prev: Option<State>,
    pub elevation_norm: Vec<f64>,
    pub config: RunConfig,
    pub(crate) pool: Option<Arc<rayon::ThreadPool>>,
    pub(crate) forcing: Vec<Vec<f64>>,
    pub(crate) bufs: Buffers,
    pub(crate) spare: Option<State>,
    pub pending: Vec<Command>,
    /// Every command applied, in order (the replay log).
    pub log: Vec<Command>,
    pub next_seq: u64,
    pub last: Option<LastTick>,
    pub ledger: Vec<ResourceLedger>,
    pub failure: Option<TickFailure>,
    pub events: VecDeque<Event>,
    pub history: VecDeque<Sample>,
    pub stats: Stats,
}

pub fn compile_env(sc: &Scenario, regions: Vec<String>) -> CompileEnv {
    CompileEnv {
        grids: vec![sc.grid.id.clone()],
        width: sc.grid.width,
        height: sc.grid.height,
        cell_size: sc.grid.cell_size,
        dt: sc.dt,
        regions,
        budgets: sc.budgets.clone(),
        population_cap: sc.population_cap,
    }
}

pub fn format_diagnostics(d: &[Diagnostic]) -> String {
    d.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn validate_scenario(sc: &Scenario) -> Result<(), String> {
    if sc.schema_version != SCENARIO_SCHEMA_VERSION {
        return Err(format!(
            "scenario schema_version {} is not supported (expected {SCENARIO_SCHEMA_VERSION})",
            sc.schema_version
        ));
    }
    let g = &sc.grid;
    if g.width < 2 || g.height < 2 || g.width > 4096 || g.height > 4096 {
        return Err("grid dimensions must be within 2..=4096".into());
    }
    if !(g.cell_size.is_finite() && g.cell_size > 0.0) {
        return Err("cell_size must be positive".into());
    }
    if g.boundary != "closed" {
        return Err("only closed horizontal boundaries are supported; model outflow with a declared sink rule".into());
    }
    if !(sc.dt.is_finite() && sc.dt > 0.0 && sc.dt <= 3600.0) {
        return Err("dt must be positive".into());
    }
    if sc.population_cap > 1_000_000 {
        return Err("population_cap too large".into());
    }
    Ok(())
}

impl Active {
    pub fn build(
        packages: Vec<Package>,
        env: &CompileEnv,
        sc: &Scenario,
        config: &RunConfig,
    ) -> Result<Active, String> {
        let mut plan = compiler::compile(&packages, env).map_err(|d| format_diagnostics(&d))?;
        if plan.schema.grids.len() != 1 {
            return Err("only one simulated grid is supported".into());
        }
        if config.optimize {
            plan = optimize::optimize(plan);
        }
        let weather = Weather::new(&sc.weather, &plan.schema)?;
        let missing = weather.missing(&plan.schema);
        if !missing.is_empty() {
            return Err(format!(
                "scenario weather does not supply forcings: {}",
                missing.join(", ")
            ));
        }
        let groups = eval::fuse_groups(&plan);
        Ok(Active {
            index: Arc::new(ResolverIndex::build(&plan)),
            wiring: Arc::new(Wiring::build(&plan)),
            groups: Arc::new(groups),
            weather: Arc::new(weather),
            plan: Arc::new(plan),
            packages,
        })
    }
}

fn resource_totals(schema: &Schema, s: &State) -> Vec<f64> {
    let mut t: Vec<KahanSum> = vec![KahanSum::default(); schema.resources.len()];
    for (fi, f) in schema.cell_fields.iter().enumerate() {
        if let FieldPolicy::Reservoir { resource, .. } = f.policy {
            for v in &s.cells[fi] {
                t[resource].add(*v);
            }
        }
    }
    for (ai, a) in schema.archetypes.iter().enumerate() {
        for (fi, f) in a.fields.iter().enumerate() {
            if let FieldPolicy::Reservoir { resource, .. } = f.policy {
                for v in &s.entities[ai].fields[fi] {
                    t[resource].add(*v);
                }
            }
        }
    }
    t.iter().map(|k| k.value()).collect()
}

impl World {
    pub fn new(
        scenario: Scenario,
        packages: Vec<Package>,
        config: RunConfig,
    ) -> Result<World, String> {
        validate_scenario(&scenario)?;
        let grid = Grid {
            id: scenario.grid.id.clone(),
            width: scenario.grid.width,
            height: scenario.grid.height,
            cell_size: scenario.grid.cell_size,
            chunk_size: scenario.grid.chunk_size,
        };
        let mut regions = vec![];
        for r in &scenario.regions {
            if !compiler::valid_id(&r.id)
                || regions.iter().any(|(x, _): &(String, Vec<f64>)| *x == r.id)
            {
                return Err(format!("invalid or duplicate region id {}", r.id));
            }
            regions.push((r.id.clone(), worldgen::shape_mask(&r.shape, &grid)?));
        }
        let env = compile_env(&scenario, regions.iter().map(|r| r.0.clone()).collect());
        let active = Active::build(packages, &env, &scenario, &config)?;
        let schema = &active.plan.schema;
        let cells = worldgen::init_cells(schema, &scenario, &grid, &regions)?;
        let elevation_norm =
            worldgen::normalized(&cells[schema.cell_field(worldgen::ELEVATION_FIELD).unwrap()]);
        let mut entities: Vec<Entities> = schema
            .archetypes
            .iter()
            .map(|a| Entities::new(a.fields.len(), a.genome_len()))
            .collect();
        let mut next_id = 1u64;
        for p in &scenario.populations {
            let Some(ai) = schema.archetype(&p.archetype) else {
                return Err(format!(
                    "population references unknown archetype {}",
                    p.archetype
                ));
            };
            let mask = match &p.region {
                Some(r) => Some(
                    regions
                        .iter()
                        .find(|x| &x.0 == r)
                        .ok_or_else(|| format!("unknown population region {r}"))?
                        .1
                        .as_slice(),
                ),
                None => None,
            };
            worldgen::spawn(
                &schema.archetypes[ai],
                &mut entities[ai],
                &mut next_id,
                &grid,
                scenario.seed,
                0,
                p,
                mask,
                None,
            )?;
        }
        let total: usize = entities.iter().map(|e| e.len()).sum();
        if total > scenario.population_cap {
            return Err(format!(
                "initial population {total} exceeds cap {}",
                scenario.population_cap
            ));
        }
        let nres = schema.resources.len();
        let state = State {
            tick: 0,
            cells,
            regions: regions.into_iter().map(|r| r.1).collect(),
            entities,
            params: active.plan.params.iter().map(|p| p.value).collect(),
            account_in: vec![0.0; schema.accounts.len()],
            account_out: vec![0.0; schema.accounts.len()],
            intervention_in: vec![0.0; nres],
            intervention_out: vec![0.0; nres],
            roundoff: vec![0.0; nres],
            next_entity_id: next_id,
        };
        let totals = resource_totals(schema, &state);
        let ledger = schema
            .resources
            .iter()
            .zip(&totals)
            .map(|(r, t)| ResourceLedger {
                resource: r.id.clone(),
                initial: *t,
                total: *t,
                ..Default::default()
            })
            .collect();
        let pool = config.threads.map(|n| {
            Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .build()
                    .expect("thread pool"),
            )
        });
        let mut w = World {
            scenario,
            active,
            grid,
            state,
            prev: None,
            elevation_norm,
            config,
            pool,
            forcing: vec![],
            bufs: Buffers::default(),
            spare: None,
            pending: vec![],
            log: vec![],
            next_seq: 1,
            last: None,
            ledger,
            failure: None,
            events: VecDeque::new(),
            history: VecDeque::new(),
            stats: Stats::default(),
        };
        w.sample();
        Ok(w)
    }

    pub fn buffers(&self) -> &Buffers {
        &self.bufs
    }

    pub fn plan(&self) -> &Plan {
        &self.active.plan
    }
    pub fn schema(&self) -> &Schema {
        &self.active.plan.schema
    }
    pub fn tick(&self) -> u64 {
        self.state.tick
    }
    pub fn time(&self) -> f64 {
        self.state.tick as f64 * self.scenario.dt
    }
    pub fn population(&self) -> usize {
        self.state.entities.iter().map(|e| e.len()).sum()
    }

    /// Queue a command for the given tick (defaults to the next tick). Returns its sequence id.
    pub fn submit(&mut self, kind: CommandKind, tick: Option<u64>) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let tick = tick.unwrap_or(self.state.tick).max(self.state.tick);
        self.pending.push(Command { seq, tick, kind });
        self.pending.sort_by_key(|c| (c.tick, c.seq));
        seq
    }

    /// Clear a failure so the experiment can resume (e.g. after fixing a law).
    pub fn clear_failure(&mut self) {
        self.failure = None;
    }

    fn push_event(&mut self, kind: EventKind) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            tick: self.state.tick,
            kind,
        });
    }

    pub fn run(&mut self, ticks: u64) -> Result<(), TickFailure> {
        for _ in 0..ticks {
            self.step()?;
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<(), TickFailure> {
        if let Some(f) = &self.failure {
            return Err(f.clone());
        }
        let r = match self.pool.clone() {
            Some(p) => p.install(|| self.step_inner()),
            None => self.step_inner(),
        };
        if let Err(f) = &r {
            self.failure = Some(f.clone());
        }
        r
    }

    fn fail(
        &self,
        phase: &'static str,
        message: String,
        effect: Option<String>,
        elements: Vec<usize>,
        plan: &Plan,
        bufs: &Buffers,
    ) -> TickFailure {
        // Locate the first non-finite intermediate value, in plan order.
        let mut nodes = vec![];
        for ins in &plan.instrs {
            if ins.outputs.iter().any(|&o| {
                bufs.slots
                    .get(o)
                    .is_some_and(|b| b.iter().any(|v| !v.is_finite()))
            }) {
                nodes.push(ins.source.clone());
                break;
            }
        }
        TickFailure {
            tick: self.state.tick,
            phase,
            message,
            effect,
            nodes,
            elements,
            plan_hash: plan.source_hash,
        }
    }

    fn step_inner(&mut self) -> Result<(), TickFailure> {
        let tick = self.state.tick;
        let dt = self.scenario.dt;
        let seed = self.scenario.seed;
        let parallel = self.config.mode == Mode::Parallel;

        // Phase 0: boundary commands on a working copy.
        let due_n = self.pending.iter().take_while(|c| c.tick <= tick).count();
        let due: Vec<Command> = self.pending.drain(..due_n).collect();
        let mut work: Option<State> = None;
        let mut active = self.active.clone();
        let mut applied = vec![];
        let mut rejected = vec![];
        let mut interventions = vec![];
        if !due.is_empty() {
            let mut w = self.state.clone();
            for c in &due {
                match self.apply_command(&mut w, &mut active, c, &mut interventions) {
                    Ok(summary) => applied.push((c.clone(), summary)),
                    Err(reason) => rejected.push((c.clone(), reason)),
                }
            }
            work = Some(w);
        }
        let plan = active.plan.clone();
        let snap: &State = work.as_ref().unwrap_or(&self.state);
        let schema = &plan.schema;

        // Phase 1: snapshot and forcing.
        active.weather.fill(
            &mut self.forcing,
            seed,
            tick as f64 * dt,
            &self.elevation_norm,
        );
        let spatial: Vec<SpatialIndex> = snap
            .entities
            .iter()
            .map(|e| SpatialIndex::build(&self.grid, e))
            .collect();

        // Phase 2: pure evaluation.
        self.bufs.prepare(&plan, &self.grid, snap);
        let mode = self.config.mode;
        let max_neighbors = self.scenario.budgets.max_neighbors;
        macro_rules! inputs {
            ($cand:expr, $rec:expr) => {
                EvalInputs {
                    plan: &plan,
                    grid: &self.grid,
                    state: snap,
                    forcing: &self.forcing,
                    spatial: &spatial,
                    candidate: $cand,
                    receipts: $rec,
                    seed,
                    dt,
                    max_neighbors,
                }
            };
        }
        eval::eval_stage(
            &inputs!(None, None),
            &mut self.bufs,
            Stage::Snapshot,
            mode,
            &active.groups,
        );

        // Phases 3-4: proposals and resolution.
        let rin = ResolveInputs {
            plan: &plan,
            index: &active.index,
            grid: &self.grid,
            state: snap,
            slots: &self.bufs.slots,
            spatial: &spatial,
            dt,
            parallel,
        };
        let resolved = match resolver::resolve(&rin) {
            Ok(r) => r,
            Err(f) => {
                let fl = self.fail(
                    "resolution",
                    f.message,
                    f.effect,
                    f.elements,
                    &plan,
                    &self.bufs,
                );
                self.pending.splice(0..0, due);
                return Err(fl);
            }
        };

        // Phase 5: receipt-dependent outcomes.
        eval::eval_stage(
            &inputs!(None, Some(&resolved.receipts)),
            &mut self.bufs,
            Stage::Receipt,
            mode,
            &active.groups,
        );

        // Phase 6: candidate integration.
        let mut cand = self.spare.take().unwrap_or_else(|| snap.clone());
        cand.clone_from(snap);
        let wiring = &active.wiring;
        let slots = &self.bufs.slots;
        let at = |s: Slot, i: usize| -> f64 {
            let b = &slots[s];
            if b.len() == 1 { b[0] } else { b[i] }
        };
        for (f, info) in schema.cell_fields.iter().enumerate() {
            match info.policy {
                FieldPolicy::Reservoir { .. } => {
                    if let Some(n) = &resolved.cell_next[f] {
                        cand.cells[f].copy_from_slice(n);
                    }
                }
                FieldPolicy::Integrated => {
                    let rates = &wiring.rate_cell[f];
                    if !rates.is_empty() {
                        let col = &mut cand.cells[f];
                        let s = &snap.cells[f];
                        for c in 0..col.len() {
                            let mut sum = 0.0;
                            for &(r, cov) in rates {
                                let v = at(r, c);
                                sum += match cov {
                                    Some(cs) => v * at(cs, c),
                                    None => v,
                                };
                            }
                            col[c] = s[c] + dt * sum;
                        }
                    }
                }
                FieldPolicy::NextValue => {
                    if let Some(v) = wiring.next_cell[f] {
                        for (c, x) in cand.cells[f].iter_mut().enumerate() {
                            *x = at(v, c);
                        }
                    }
                }
                FieldPolicy::Parameter => {}
            }
        }
        for (ai, a) in schema.archetypes.iter().enumerate() {
            let e = &mut cand.entities[ai];
            let m = e.len();
            for (f, info) in a.fields.iter().enumerate() {
                match info.policy {
                    FieldPolicy::Reservoir { .. } => {
                        if let Some(n) = &resolved.ent_next[ai][f] {
                            e.fields[f].copy_from_slice(n);
                        }
                    }
                    FieldPolicy::Integrated => {
                        let rates = &wiring.rate_ent[ai][f];
                        if !rates.is_empty() {
                            let s = &snap.entities[ai].fields[f];
                            for i in 0..m {
                                let mut sum = 0.0;
                                for &(r, cov) in rates {
                                    let v = at(r, i);
                                    sum += match cov {
                                        Some(cs) => v * at(cs, i),
                                        None => v,
                                    };
                                }
                                e.fields[f][i] = s[i] + dt * sum;
                            }
                        }
                    }
                    FieldPolicy::NextValue => {
                        if let Some(v) = wiring.next_ent[ai][f] {
                            for i in 0..m {
                                e.fields[f][i] = at(v, i);
                            }
                        }
                    }
                    FieldPolicy::Parameter => {}
                }
            }
            if let Some((turn, speed)) = wiring.moves[ai] {
                let (ex, ez) = (self.grid.extent_x(), self.grid.extent_z());
                for i in 0..m {
                    let t = at(turn, i).clamp(-a.max_turn_rate, a.max_turn_rate);
                    let v = at(speed, i).clamp(0.0, a.max_speed);
                    let h = (e.heading[i] + t * dt).rem_euclid(std::f64::consts::TAU);
                    let (sn, cs) = h.sin_cos();
                    e.heading[i] = h;
                    e.x[i] = (e.x[i] + cs * v * dt).clamp(0.0, ex * (1.0 - 1e-12));
                    e.z[i] = (e.z[i] + sn * v * dt).clamp(0.0, ez * (1.0 - 1e-12));
                }
            }
            for i in 0..m {
                e.age[i] += dt;
                e.cooldown[i] = (e.cooldown[i] - dt).max(0.0);
            }
        }
        let mut account_in = resolved.account_in.clone();
        let mut account_out = resolved.account_out.clone();

        // Phase 7: lifecycle on the candidate.
        eval::eval_stage(
            &inputs!(Some(&cand.entities), Some(&resolved.receipts)),
            &mut self.bufs,
            Stage::Candidate,
            mode,
            &active.groups,
        );
        let slots = &self.bufs.slots;
        let at = |s: Slot, i: usize| -> f64 {
            let b = &slots[s];
            if b.len() == 1 { b[0] } else { b[i] }
        };
        let mut new_events = vec![];
        let mut births_total = 0u64;
        let mut deaths_total = 0u64;
        let mut death_reasons: Vec<String> = vec![];
        let mut cap_rejected = 0usize;
        let mut dying: Vec<Vec<Option<usize>>> = vec![];
        for (ai, _) in schema.archetypes.iter().enumerate() {
            let m = cand.entities[ai].len();
            let mut d = vec![None; m];
            for (k, (cond, _)) in wiring.deaths[ai].iter().enumerate() {
                for (i, di) in d.iter_mut().enumerate() {
                    if di.is_none() && at(*cond, i) != 0.0 {
                        *di = Some(k);
                    }
                }
            }
            dying.push(d);
        }
        let alive_after: usize = dying
            .iter()
            .map(|d| d.iter().filter(|x| x.is_none()).count())
            .sum();
        let mut eligible: Vec<(u64, usize, usize)> = vec![]; // (order key, arch, row)
        for (ai, _) in schema.archetypes.iter().enumerate() {
            let Some((cond, legs)) = &wiring.births[ai] else {
                continue;
            };
            let e = &cand.entities[ai];
            for i in 0..e.len() {
                if dying[ai][i].is_some() || at(*cond, i) == 0.0 || e.cooldown[i] > 0.0 {
                    continue;
                }
                let funded = legs.iter().all(|&(f, s)| {
                    let amt = at(s, i);
                    amt.is_finite() && amt >= 0.0 && amt <= e.fields[f][i]
                });
                if funded {
                    eligible.push((rng::key(seed, tick, STREAM_BIRTH_ORDER, e.ids[i], 0), ai, i));
                }
            }
        }
        eligible.sort();
        let room = self.scenario.population_cap.saturating_sub(alive_after);
        if eligible.len() > room {
            cap_rejected = eligible.len() - room;
            eligible.truncate(room);
        }
        eligible.sort_by_key(|&(_, a, i)| (a, i));
        // Death disposal: every conserved reservoir moves before rows are removed.
        for (ai, a) in schema.archetypes.iter().enumerate() {
            for i in 0..cand.entities[ai].len() {
                let Some(k) = dying[ai][i] else { continue };
                let (x, z) = (cand.entities[ai].x[i], cand.entities[ai].z[i]);
                let c = self.grid.cell_at(x, z);
                for &(f, ep) in &a.disposal {
                    let v = cand.entities[ai].fields[f][i];
                    match ep {
                        Endpoint::Cell(cf) => cand.cells[cf][c] += v,
                        Endpoint::External(acc) => account_out[acc] += v,
                        _ => unreachable!(),
                    }
                    cand.entities[ai].fields[f][i] = 0.0;
                }
                let reason = wiring.deaths[ai][k].1.clone();
                new_events.push(EventKind::Death {
                    arch: a.id.clone(),
                    id: cand.entities[ai].ids[i],
                    reason: reason.clone(),
                });
                death_reasons.push(reason);
                deaths_total += 1;
            }
        }
        // Births: debit parents atomically, build offspring.
        let mut newborns: Vec<Vec<(u64, f64, f64, f64, Vec<f64>, Vec<f64>, Lineage)>> =
            vec![vec![]; schema.archetypes.len()];
        for &(_, ai, i) in &eligible {
            let a = &schema.archetypes[ai];
            let (_, legs) = wiring.births[ai].as_ref().unwrap();
            let id = cand.next_entity_id;
            cand.next_entity_id += 1;
            let e = &mut cand.entities[ai];
            let mut fields: Vec<f64> = a
                .fields
                .iter()
                .map(|f| {
                    if matches!(f.policy, FieldPolicy::Reservoir { .. }) {
                        0.0
                    } else {
                        f.default
                    }
                })
                .collect();
            for &(f, s) in legs {
                let amt = at(s, i);
                e.fields[f][i] -= amt;
                fields[f] += amt;
            }
            e.cooldown[i] = a.cooldown;
            e.lineage[i].offspring += 1;
            let r = |k: u64| rng::draw(seed, tick, STREAM_BIRTH_PLACE, id, k);
            let cs = self.grid.cell_size;
            let x =
                (e.x[i] + (r(0) - 0.5) * 2.0 * cs).clamp(0.0, self.grid.extent_x() * (1.0 - 1e-12));
            let z =
                (e.z[i] + (r(1) - 0.5) * 2.0 * cs).clamp(0.0, self.grid.extent_z() * (1.0 - 1e-12));
            let heading = r(2) * std::f64::consts::TAU;
            let genome = biology::mutate(a, e.genome_of(i), seed, tick, id);
            let pl = &e.lineage[i];
            let lineage = Lineage {
                parent: e.ids[i],
                root: pl.root,
                generation: pl.generation + 1,
                birth_tick: tick + 1,
                birth_x: x,
                birth_z: z,
                origin: Origin::Born,
                offspring: 0,
                mutation_version: biology::MUTATION_VERSION,
            };
            new_events.push(EventKind::Birth {
                arch: a.id.clone(),
                id,
                parent: e.ids[i],
            });
            newborns[ai].push((id, x, z, heading, fields, genome, lineage));
            births_total += 1;
        }
        for (ai, e) in cand.entities.iter_mut().enumerate() {
            if dying[ai].iter().any(|d| d.is_some()) {
                let keep: Vec<bool> = dying[ai].iter().map(|d| d.is_none()).collect();
                e.retain(&keep);
            }
            for (id, x, z, h, f, g, l) in newborns[ai].drain(..) {
                e.push(id, x, z, h, &f, &g, l, 0.0);
            }
        }

        // Phase 8: validation, ledger, and commit.
        for (f, info) in schema.cell_fields.iter().enumerate() {
            if info.policy == FieldPolicy::Parameter {
                continue;
            }
            if let Some(c) = cand.cells[f].iter().position(|v| !v.is_finite()) {
                let fl = self.fail(
                    "validation",
                    format!("non-finite value in field {}", info.id),
                    None,
                    vec![c],
                    &plan,
                    &self.bufs,
                );
                self.pending.splice(0..0, due);
                return Err(fl);
            }
        }
        for (ai, a) in schema.archetypes.iter().enumerate() {
            let e = &cand.entities[ai];
            for (f, info) in a.fields.iter().enumerate() {
                if let Some(i) = e.fields[f].iter().position(|v| !v.is_finite()) {
                    let fl = self.fail(
                        "validation",
                        format!("non-finite value in {}.{}", a.id, info.id),
                        None,
                        vec![i],
                        &plan,
                        &self.bufs,
                    );
                    self.pending.splice(0..0, due);
                    return Err(fl);
                }
            }
            if e.x
                .iter()
                .chain(&e.z)
                .chain(&e.heading)
                .any(|v| !v.is_finite())
            {
                let fl = self.fail(
                    "validation",
                    format!("non-finite position in {}", a.id),
                    None,
                    vec![],
                    &plan,
                    &self.bufs,
                );
                self.pending.splice(0..0, due);
                return Err(fl);
            }
        }
        for (a, v) in account_in.iter_mut().enumerate() {
            cand.account_in[a] += *v;
        }
        for (a, v) in account_out.iter_mut().enumerate() {
            cand.account_out[a] += *v;
        }
        for (r, v) in resolved.roundoff.iter().enumerate() {
            cand.roundoff[r] += v;
        }
        cand.tick = tick + 1;
        let totals = resource_totals(schema, &cand);
        let mut ledger = self.ledger.clone();
        if applied
            .iter()
            .any(|(c, _)| matches!(c.kind, CommandKind::ApplyPackages { .. }))
        {
            // Law change: carry ledgers across by resource id, based on the migrated snapshot.
            let base = resource_totals(schema, snap);
            let iv: Vec<f64> = schema
                .resources
                .iter()
                .enumerate()
                .map(|(r, _)| interventions.iter().filter(|x| x.0 == r).map(|x| x.1).sum())
                .collect();
            ledger = schema
                .resources
                .iter()
                .enumerate()
                .map(|(r, res)| {
                    let pre = base[r] - iv[r];
                    match self.ledger.iter().find(|l| l.resource == res.id) {
                        Some(l) => ResourceLedger {
                            total: pre,
                            ..l.clone()
                        },
                        None => ResourceLedger {
                            resource: res.id.clone(),
                            initial: pre,
                            total: pre,
                            ..Default::default()
                        },
                    }
                })
                .collect();
        }
        for (r, res) in schema.resources.iter().enumerate() {
            let l = &mut ledger[r];
            let (mut ext_in, mut ext_out) = (0.0, 0.0);
            for (a, acc) in schema.accounts.iter().enumerate() {
                if acc.resource == r {
                    ext_in += account_in[a];
                    ext_out += account_out[a];
                }
            }
            let (iv_in, iv_out) = interventions.iter().filter(|(rr, _)| *rr == r).fold(
                (0.0, 0.0),
                |(i, o), (_, v): &(usize, f64)| {
                    if *v >= 0.0 { (i + v, o) } else { (i, o - v) }
                },
            );
            let roundoff = resolved.roundoff[r];
            let prev_total = l.total;
            let exchange = ext_in + ext_out + iv_in + iv_out + roundoff;
            let expected = prev_total + ext_in + iv_in + roundoff - ext_out - iv_out;
            let err = totals[r] - expected;
            let tol = res.tol_abs + res.tol_rel * prev_total.abs().max(exchange);
            l.total = totals[r];
            l.cumulative_in += ext_in + iv_in + roundoff;
            l.cumulative_out += ext_out + iv_out;
            l.cumulative_abs_exchange += exchange;
            l.last_tick_error = err;
            l.max_tick_error = l.max_tick_error.max(err.abs());
            l.last_tolerance = tol;
            l.cumulative_error = l.total - (l.initial + l.cumulative_in - l.cumulative_out);
            if err.abs() > tol {
                let fl = self.fail(
                    "validation",
                    format!(
                        "unexplained {} budget drift {err:e} exceeds tolerance {tol:e}",
                        res.id
                    ),
                    None,
                    vec![],
                    &plan,
                    &self.bufs,
                );
                self.pending.splice(0..0, due);
                return Err(fl);
            }
        }
        let mut range_events = vec![];
        for (f, info) in schema.cell_fields.iter().enumerate() {
            if info.min.is_none() && info.max.is_none() {
                continue;
            }
            let (lo, hi) = (
                info.min.unwrap_or(f64::NEG_INFINITY),
                info.max.unwrap_or(f64::INFINITY),
            );
            let n = cand.cells[f]
                .iter()
                .filter(|v| **v < lo || **v > hi)
                .count();
            let before = self
                .stats
                .range_violations
                .get(&info.id)
                .copied()
                .unwrap_or(0);
            if n > 0 && before == 0 {
                range_events.push(EventKind::RangeWarning {
                    field: info.id.clone(),
                    count: n,
                });
            }
            self.stats.range_violations.insert(info.id.clone(), n);
        }

        // Commit.
        drop(work);
        let old = std::mem::replace(&mut self.state, cand);
        self.spare = self.prev.replace(old);
        self.ledger = ledger;
        let plan_changed = !Arc::ptr_eq(&self.active.plan, &active.plan);
        self.active = active.clone();
        if plan_changed {
            self.spare = None;
        }
        self.last = Some(LastTick {
            resolved,
            spatial,
            active,
        });
        for (c, summary) in applied {
            self.push_event(EventKind::CommandApplied {
                seq: c.seq,
                summary,
            });
            self.log.push(c);
        }
        for (c, reason) in rejected {
            self.push_event(EventKind::CommandRejected { seq: c.seq, reason });
            self.log.push(c);
        }
        for k in new_events.into_iter().chain(range_events) {
            self.push_event(k);
        }
        if cap_rejected > 0 {
            self.push_event(EventKind::PopulationCap {
                rejected: cap_rejected,
            });
        }
        self.stats.births += births_total;
        self.stats.deaths += deaths_total;
        self.stats.cap_rejections += cap_rejected as u64;
        for r in death_reasons {
            *self.stats.deaths_by_reason.entry(r).or_default() += 1;
        }
        if self.config.sample_interval > 0
            && self.state.tick.is_multiple_of(self.config.sample_interval)
        {
            self.sample();
        }
        Ok(())
    }

    pub fn sample(&mut self) {
        let schema = self.active.plan.schema.clone();
        let s = &self.state;
        let mut trait_means = vec![];
        let mut trait_hist = vec![];
        let mut mean_generation = vec![];
        for (ai, a) in schema.archetypes.iter().enumerate() {
            let e = &s.entities[ai];
            let m = e.len().max(1) as f64;
            let mut means = vec![0.0; a.traits.len()];
            let mut hist = vec![[0u32; 10]; a.traits.len()];
            for i in 0..e.len() {
                let g = e.genome_of(i);
                for (t, ti) in a.traits.iter().enumerate() {
                    means[t] += g[t] / m;
                    let b = (((g[t] - ti.min) / (ti.max - ti.min).max(1e-12)) * 10.0)
                        .floor()
                        .clamp(0.0, 9.0) as usize;
                    hist[t][b] += 1;
                }
            }
            trait_means.push(means);
            trait_hist.push(hist);
            mean_generation.push(e.lineage.iter().map(|l| l.generation as f64).sum::<f64>() / m);
        }
        let sample = Sample {
            tick: s.tick,
            population: s.entities.iter().map(|e| e.len()).collect(),
            trait_means,
            trait_hist,
            resource_totals: self.ledger.iter().map(|l| l.total).collect(),
            field_totals: s
                .cells
                .iter()
                .map(|c| crate::state::compensated_sum(c))
                .collect(),
            births: self.stats.births,
            deaths: self.stats.deaths,
            mean_generation,
        };
        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(sample);
    }

    // ---- commands -------------------------------------------------------------------

    fn apply_command(
        &self,
        w: &mut State,
        active: &mut Active,
        c: &Command,
        interventions: &mut Vec<(usize, f64)>,
    ) -> Result<String, String> {
        let schema = active.plan.schema.clone();
        match &c.kind {
            CommandKind::SetParam { name, value } => {
                let plan = &active.plan;
                let Some(i) = plan.param_index(name) else {
                    return Err(format!("unknown parameter {name}"));
                };
                check_param(plan, i, *value, w, self.scenario.dt, self.grid.cell_size)?;
                w.params[i] = *value;
                Ok(format!("{name} = {value}"))
            }
            CommandKind::CreateRegion { id } => {
                if !compiler::valid_id(id) || schema.region(id).is_some() {
                    return Err(format!("invalid or existing region id {id}"));
                }
                let mut regions = schema.regions.clone();
                regions.push(id.clone());
                let env = compile_env(&self.scenario, regions);
                let new =
                    Active::build(active.packages.clone(), &env, &self.scenario, &self.config)?;
                w.regions.push(vec![0.0; self.grid.cells()]);
                *active = new;
                Ok(format!("created region {id}"))
            }
            CommandKind::PaintRegion {
                region,
                shape,
                value,
            } => {
                let Some(r) = schema.region(region) else {
                    return Err(format!("unknown region {region}"));
                };
                if !(0.0..=1.0).contains(value) {
                    return Err("region coverage must be in [0, 1]".into());
                }
                let m = worldgen::shape_mask(shape, &self.grid)?;
                for (x, cv) in w.regions[r].iter_mut().zip(&m) {
                    *x = *x * (1.0 - cv) + value * cv;
                }
                Ok(format!("painted region {region}"))
            }
            CommandKind::SetField {
                field,
                shape,
                value,
            } => {
                let Some(f) = schema.cell_field(field) else {
                    return Err(format!("unknown field {field}"));
                };
                if schema.cell_fields[f].policy != FieldPolicy::Parameter
                    || field == worldgen::ELEVATION_FIELD
                {
                    return Err(format!(
                        "{field} is not an editable parameter field; use an explicit intervention for reservoirs"
                    ));
                }
                if !value.is_finite() {
                    return Err("value must be finite".into());
                }
                let m = worldgen::shape_mask(shape, &self.grid)?;
                for (x, cv) in w.cells[f].iter_mut().zip(&m) {
                    *x = *x * (1.0 - cv) + value * cv;
                }
                Ok(format!("set {field} = {value}"))
            }
            CommandKind::AddResource {
                field,
                shape,
                amount,
            } => {
                let Some(f) = schema.cell_field(field) else {
                    return Err(format!("unknown field {field}"));
                };
                let FieldPolicy::Reservoir { resource, capacity } = schema.cell_fields[f].policy
                else {
                    return Err(format!("{field} is not a reservoir"));
                };
                if !amount.is_finite() || amount.abs() > 1e15 {
                    return Err("amount must be finite".into());
                }
                let m = worldgen::shape_mask(shape, &self.grid)?;
                let mut total = KahanSum::default();
                for c in 0..self.grid.cells() {
                    if m[c] == 0.0 {
                        continue;
                    }
                    let cur = w.cells[f][c];
                    let mut next = (cur + amount * m[c]).max(0.0);
                    if let Some(cf) = capacity {
                        next = next.min(w.cells[cf][c].max(cur));
                    }
                    total.add(next - cur);
                    w.cells[f][c] = next;
                }
                let t = total.value();
                interventions.push((resource, t));
                if t >= 0.0 {
                    w.intervention_in[resource] += t;
                } else {
                    w.intervention_out[resource] -= t;
                }
                Ok(format!(
                    "intervention: {t:.3} {} into {field}",
                    schema.resources[resource].unit
                ))
            }
            CommandKind::SpawnOrganisms {
                archetype,
                shape,
                count,
            } => {
                let Some(ai) = schema.archetype(archetype) else {
                    return Err(format!("unknown archetype {archetype}"));
                };
                if *count > 10_000 {
                    return Err("spawn count too large".into());
                }
                let total: usize = w.entities.iter().map(|e| e.len()).sum();
                if total + count > self.scenario.population_cap {
                    return Err("spawn would exceed the population cap".into());
                }
                let m = worldgen::shape_mask(shape, &self.grid)?;
                let a = &schema.archetypes[ai];
                let decl = PopulationDecl {
                    archetype: archetype.clone(),
                    count: *count,
                    genome: "random".into(),
                    region: None,
                    traits: Default::default(),
                    authored_links: vec![],
                    authored_bias: vec![],
                };
                let mut next = w.next_entity_id;
                let n = worldgen::spawn(
                    a,
                    &mut w.entities[ai],
                    &mut next,
                    &self.grid,
                    self.scenario.seed,
                    w.tick,
                    &decl,
                    Some(&m),
                    Some(Origin::Intervention),
                )?;
                w.next_entity_id = next;
                for (fi, f) in a.fields.iter().enumerate() {
                    if let FieldPolicy::Reservoir { resource, .. } = f.policy {
                        let v = a.seed_state[fi] * n as f64;
                        interventions.push((resource, v));
                        w.intervention_in[resource] += v;
                    }
                }
                Ok(format!("intervention: spawned {n} {archetype}"))
            }
            CommandKind::ApplyPackages { packages } => {
                let env = compile_env(&self.scenario, schema.regions.clone());
                let new = Active::build(packages.clone(), &env, &self.scenario, &self.config)?;
                migrate(w, &schema, &new.plan, &self.grid)?;
                let n = new.plan.instrs.len();
                *active = new;
                Ok(format!(
                    "applied {} packages ({n} instructions)",
                    packages.len()
                ))
            }
        }
    }
}

pub fn check_param(
    plan: &Plan,
    i: usize,
    value: f64,
    w: &State,
    dt: f64,
    dx: f64,
) -> Result<(), String> {
    let p = &plan.params[i];
    if !value.is_finite() || value < p.min || value > p.max {
        return Err(format!(
            "{} = {value} is outside its validated range [{}, {}]",
            p.name, p.min, p.max
        ));
    }
    if let Some(st) = &p.stability {
        let (lhs, limit) = match st {
            Stability::Diffusion => (dt * value * 4.0 / (dx * dx), 1.0),
            Stability::FirstOrder => (dt * value, 1.0),
            Stability::Transport { limit, .. } => {
                let rho = p.density_param.map(|d| w.params[d]).unwrap_or(f64::NAN);
                (dt * value * 4.0 / (rho * dx * dx), *limit)
            }
        };
        if lhs.is_nan() || lhs > limit {
            return Err(format!(
                "{} = {value} violates its numerical stability limit ({lhs:.4} > {limit})",
                p.name
            ));
        }
    }
    Ok(())
}

/// Migrate state to a new schema at a tick boundary. Compatible fields keep
/// their values; new fields take defaults; removing populated conserved
/// reservoirs or changing a populated field's meaning is rejected.
pub fn migrate(w: &mut State, old: &Schema, plan: &Plan, grid: &Grid) -> Result<(), String> {
    let new = &plan.schema;
    let n = grid.cells();
    let mut cells = vec![];
    for f in &new.cell_fields {
        match old.cell_field(&f.id) {
            Some(oi) => {
                let of = &old.cell_fields[oi];
                if of.unit != f.unit
                    || of.quantity != f.quantity
                    || std::mem::discriminant(&of.policy) != std::mem::discriminant(&f.policy)
                {
                    return Err(format!(
                        "field {} changes unit, quantity, or policy; that requires an explicit migration or a new experiment",
                        f.id
                    ));
                }
                cells.push(w.cells[oi].clone());
            }
            None => cells.push(vec![f.default; n]),
        }
    }
    for (oi, of) in old.cell_fields.iter().enumerate() {
        if new.cell_field(&of.id).is_none()
            && matches!(of.policy, FieldPolicy::Reservoir { .. })
            && w.cells[oi].iter().any(|v| *v != 0.0)
        {
            return Err(format!(
                "removing populated conserved reservoir {} requires a declared transfer or external sink first",
                of.id
            ));
        }
    }
    let mut ents = vec![];
    for a in &new.archetypes {
        match old.archetype(&a.id) {
            Some(oi) => {
                let oa = &old.archetypes[oi];
                let e = &w.entities[oi];
                if oa.genome_len() != a.genome_len() && !e.is_empty() {
                    return Err(format!(
                        "archetype {} changes genome layout while populated",
                        a.id
                    ));
                }
                let mut ne = e.clone();
                ne.fields = vec![];
                for f in &a.fields {
                    match oa.field(&f.id) {
                        Some(ofi) => {
                            if oa.fields[ofi].unit != f.unit {
                                return Err(format!("{}.{} changes unit", a.id, f.id));
                            }
                            ne.fields.push(e.fields[ofi].clone());
                        }
                        None => ne.fields.push(vec![f.default; e.len()]),
                    }
                }
                for (ofi, of) in oa.fields.iter().enumerate() {
                    if a.field(&of.id).is_none()
                        && matches!(of.policy, FieldPolicy::Reservoir { .. })
                        && e.fields[ofi].iter().any(|v| *v != 0.0)
                    {
                        return Err(format!(
                            "removing populated reservoir {}.{} requires a declared transfer first",
                            a.id, of.id
                        ));
                    }
                }
                ents.push(ne);
            }
            None => ents.push(Entities::new(a.fields.len(), a.genome_len())),
        }
    }
    for (oi, oa) in old.archetypes.iter().enumerate() {
        if new.archetype(&oa.id).is_none() && !w.entities[oi].is_empty() {
            return Err(format!("removing populated archetype {}", oa.id));
        }
    }
    if old
        .resources
        .iter()
        .map(|r| &r.id)
        .ne(new.resources.iter().map(|r| &r.id))
    {
        let remap = |v: &Vec<f64>| -> Vec<f64> {
            new.resources
                .iter()
                .map(|r| old.resource(&r.id).map_or(0.0, |i| v[i]))
                .collect()
        };
        w.intervention_in = remap(&w.intervention_in);
        w.intervention_out = remap(&w.intervention_out);
        w.roundoff = remap(&w.roundoff);
    }
    let remap_acc = |v: &Vec<f64>| -> Vec<f64> {
        new.accounts
            .iter()
            .map(|a| old.account(&a.id).map_or(0.0, |i| v[i]))
            .collect()
    };
    w.account_in = remap_acc(&w.account_in);
    w.account_out = remap_acc(&w.account_out);
    w.cells = cells;
    w.entities = ents;
    w.params = plan.params.iter().map(|p| p.value).collect();
    Ok(())
}
