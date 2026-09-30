//! Rule compiler: validate packages, type-check graphs, and lower them to a plan.
//!
//! Pipeline: header/capability validation -> schema construction -> parameter
//! table -> symbol resolution with cycle detection -> quantity/unit/domain/stage
//! checking -> writer/effect/conservation validation -> budget analysis.
//! Optimization passes live in `optimize.rs` and operate on the resulting plan.

use crate::ir::*;
use crate::schema::*;
use crate::units::{DIMENSIONLESS, Unit};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const SUPPORTED_CAPABILITIES: &[&str] = &[
    "core.cells.v1",
    "core.edges.v1",
    "core.entities.v1",
    "brain.mlp.tanh.v1",
    "sandbox.external_source",
];

pub const MAX_CURVE_POINTS: usize = 64;
pub const MAX_ID_LEN: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub rule: Option<String>,
    pub node: Option<String>,
    pub port: Option<String>,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let loc = match (&self.rule, &self.node, &self.port) {
            (Some(r), Some(n), Some(p)) => format!("{r}/{n}.{p}: "),
            (Some(r), Some(n), None) => format!("{r}/{n}: "),
            (Some(r), None, _) => format!("{r}: "),
            _ => String::new(),
        };
        write!(f, "[{}] {loc}{}", self.code, self.message)
    }
}

fn err(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code,
        message: message.into(),
        rule: None,
        node: None,
        port: None,
    }
}

impl Diagnostic {
    fn at(mut self, rule: &str, node: Option<&str>, port: Option<&str>) -> Diagnostic {
        self.rule.get_or_insert_with(|| rule.to_string());
        if self.node.is_none() {
            self.node = node.map(|s| s.to_string());
        }
        if self.port.is_none() {
            self.port = port.map(|s| s.to_string());
        }
        self
    }
}

// ---------------------------------------------------------------------------
// Schema derived from packages

#[derive(Clone, Debug, Serialize)]
pub struct ResourceInfo {
    pub id: String,
    pub unit: Unit,
    pub tol_abs: f64,
    pub tol_rel: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountInfo {
    pub id: String,
    pub resource: usize,
    pub can_source: bool,
    pub can_sink: bool,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum FieldPolicy {
    Reservoir { resource: usize, capacity: Option<usize> },
    Integrated,
    NextValue,
    Parameter,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldInfo {
    pub id: String,
    pub grid: u16,
    pub unit: Unit,
    pub quantity: Option<String>,
    pub policy: FieldPolicy,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ForcingInfo {
    pub id: String,
    pub unit: Unit,
    pub quantity: Option<String>,
    pub cells: bool,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TraitInfo {
    pub id: String,
    pub unit: Unit,
    pub min: f64,
    pub max: f64,
    pub init_min: f64,
    pub init_max: f64,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArchInfo {
    pub id: String,
    pub label: String,
    /// Entity fields; reservoir capacity indices refer to `capacity_slots`.
    pub fields: Vec<FieldInfo>,
    pub capacity_slots: Vec<Option<Slot>>,
    pub traits: Vec<TraitInfo>,
    pub brain: BrainDecl,
    pub max_speed: f64,
    pub max_turn_rate: f64,
    pub disposal: Vec<(usize, Endpoint)>,
    pub mutation: MutationDecl,
    pub cooldown: f64,
    pub seed_state: Vec<f64>,
}

impl ArchInfo {
    pub fn brain_params(&self) -> usize {
        let b = &self.brain;
        b.inputs * b.hidden + b.hidden + b.hidden * b.outputs + b.outputs
    }
    pub fn genome_len(&self) -> usize {
        self.traits.len() + self.brain_params()
    }
    pub fn field(&self, id: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.id == id)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Schema {
    pub grids: Vec<String>,
    pub resources: Vec<ResourceInfo>,
    pub accounts: Vec<AccountInfo>,
    pub cell_fields: Vec<FieldInfo>,
    pub forcings: Vec<ForcingInfo>,
    pub archetypes: Vec<ArchInfo>,
    pub regions: Vec<String>,
}

impl Schema {
    pub fn cell_field(&self, id: &str) -> Option<usize> {
        self.cell_fields.iter().position(|f| f.id == id)
    }
    pub fn resource(&self, id: &str) -> Option<usize> {
        self.resources.iter().position(|r| r.id == id)
    }
    pub fn account(&self, id: &str) -> Option<usize> {
        self.accounts.iter().position(|a| a.id == id)
    }
    pub fn archetype(&self, id: &str) -> Option<usize> {
        self.archetypes.iter().position(|a| a.id == id)
    }
    pub fn forcing(&self, id: &str) -> Option<usize> {
        self.forcings.iter().position(|f| f.id == id)
    }
    pub fn region(&self, id: &str) -> Option<usize> {
        self.regions.iter().position(|r| r == id)
    }
    pub fn arch_names(&self) -> Vec<String> {
        self.archetypes.iter().map(|a| a.id.clone()).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub schema: Schema,
    pub slots: Vec<SlotInfo>,
    pub instrs: Vec<Instr>,
    pub effects: Vec<Effect>,
    pub params: Vec<ParamInfo>,
    pub cost: CostEstimate,
    pub warnings: Vec<Diagnostic>,
    /// Hash of the canonical source packages.
    pub source_hash: u64,
    /// Qualified node id -> output slots.
    pub source_map: BTreeMap<String, Vec<Slot>>,
    pub optimized: bool,
}

impl Plan {
    pub fn param_index(&self, qualified: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name == qualified)
    }
}

/// What the compiler needs to know about the world it targets.
#[derive(Clone, Debug)]
pub struct CompileEnv {
    /// Grid ids; the first is the simulated surface.
    pub grids: Vec<String>,
    pub width: usize,
    pub height: usize,
    pub cell_size: f64,
    pub dt: f64,
    pub regions: Vec<String>,
    pub budgets: Budgets,
    pub population_cap: usize,
}

impl CompileEnv {
    pub fn cells(&self) -> u64 {
        (self.width * self.height) as u64
    }
    pub fn edges(&self) -> u64 {
        (self.width.saturating_sub(1) * self.height + self.width * self.height.saturating_sub(1)) as u64
    }
}

pub fn package_hash(packages: &[Package]) -> u64 {
    let mut sorted: Vec<&Package> = packages.iter().collect();
    sorted.sort_by(|a, b| a.package_id.cmp(&b.package_id));
    let mut h = crate::rng::Fnv::new();
    for p in sorted {
        h.write(serde_json::to_string(p).unwrap_or_default().as_bytes());
    }
    h.finish()
}

pub fn compile(packages: &[Package], env: &CompileEnv) -> Result<Plan, Vec<Diagnostic>> {
    compile_with_types(packages, env).0
}

/// Output port types per qualified node id, for editor feedback: (port, description).
pub type NodeTypes = BTreeMap<String, Vec<(String, String)>>;

/// Compile, also returning the types of every node that resolved, even when
/// other nodes have errors (so the editor can label ports of a broken draft).
pub fn compile_with_types(packages: &[Package], env: &CompileEnv) -> (Result<Plan, Vec<Diagnostic>>, NodeTypes) {
    let mut sorted: Vec<&Package> = packages.iter().collect();
    sorted.sort_by(|a, b| a.package_id.cmp(&b.package_id));
    let mut c = Compiler::new(sorted, env);
    c.run();
    let mut types = NodeTypes::new();
    let an = c.schema.arch_names();
    for (k, outs) in &c.outputs {
        let ports: Vec<(String, String)> = if c.effect_of.contains_key(k) {
            vec![("effect".into(), "effect (receipts: accepted, accepted_rate, fraction)".into())]
        } else {
            outs.iter()
                .enumerate()
                .filter(|(_, s)| **s < c.types.len())
                .map(|(i, s)| {
                    (
                        if outs.len() == 1 { "value".to_string() } else { format!("out{i}") },
                        c.types[*s].describe(&c.schema.grids, &an),
                    )
                })
                .collect()
        };
        types.insert(k.clone(), ports);
    }
    (finish(c, packages), types)
}

fn finish(c: Compiler, packages: &[Package]) -> Result<Plan, Vec<Diagnostic>> {
    let errors: Vec<Diagnostic> = c.diags.iter().filter(|d| d.severity == Severity::Error).cloned().collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    let warnings = c.diags.iter().filter(|d| d.severity == Severity::Warning).cloned().collect();
    let cost = c.cost.clone().unwrap();
    Ok(Plan {
        schema: c.schema,
        slots: c.slots,
        instrs: c.instrs,
        effects: c.effects,
        params: c.params,
        cost,
        warnings,
        source_hash: package_hash(packages),
        source_map: c.source_map,
        optimized: false,
    })
}

// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Visit {
    Active,
    Done,
}

struct NodeRef<'a> {
    rule: usize,
    node: &'a Node,
}

struct RuleCtx {
    id: String,
    package: String,
    dom: Dom,
    arch: Option<u16>,
    params: BTreeMap<String, usize>,
}

struct Compiler<'a> {
    packages: Vec<&'a Package>,
    env: &'a CompileEnv,
    diags: Vec<Diagnostic>,
    schema: Schema,
    params: Vec<ParamInfo>,
    package_params: HashMap<String, BTreeMap<String, usize>>,
    rules: Vec<(&'a Rule, RuleCtx)>,
    nodes: BTreeMap<String, NodeRef<'a>>,
    visit: HashMap<String, Visit>,
    /// Qualified node id -> output slots (in port order).
    outputs: HashMap<String, Vec<Slot>>,
    /// Effect index for effect nodes.
    effect_of: HashMap<String, usize>,
    stack: Vec<(String, bool)>,
    slots: Vec<SlotInfo>,
    types: Vec<Type>,
    instrs: Vec<Instr>,
    effects: Vec<Effect>,
    source_map: BTreeMap<String, Vec<Slot>>,
    cost: Option<CostEstimate>,
    const_values: HashMap<Slot, f64>,
}

type R<T> = Result<T, Diagnostic>;

impl<'a> Compiler<'a> {
    fn new(packages: Vec<&'a Package>, env: &'a CompileEnv) -> Self {
        Compiler {
            packages,
            env,
            diags: vec![],
            schema: Schema {
                grids: env.grids.clone(),
                resources: vec![],
                accounts: vec![],
                cell_fields: vec![],
                forcings: vec![],
                archetypes: vec![],
                regions: env.regions.clone(),
            },
            params: vec![],
            package_params: HashMap::new(),
            rules: vec![],
            nodes: BTreeMap::new(),
            visit: HashMap::new(),
            outputs: HashMap::new(),
            effect_of: HashMap::new(),
            stack: vec![],
            slots: vec![],
            types: vec![],
            instrs: vec![],
            effects: vec![],
            source_map: BTreeMap::new(),
            cost: None,
            const_values: HashMap::new(),
        }
    }

    fn run(&mut self) {
        self.headers();
        if self.has_errors() {
            return;
        }
        self.build_schema();
        if self.has_errors() {
            return;
        }
        self.build_params();
        self.collect_rules();
        if self.has_errors() {
            return;
        }
        let keys: Vec<String> = self.nodes.keys().cloned().collect();
        for k in keys {
            if let Err(d) = self.resolve(&k) {
                self.diags.push(d);
            }
        }
        if self.has_errors() {
            return;
        }
        self.entity_capacities();
        self.check_rule_effect_lists();
        self.check_writers();
        self.canonicalize_effects();
        self.canonicalize_params();
        self.estimate_cost();
    }

    fn has_errors(&self) -> bool {
        self.diags.iter().any(|d| d.severity == Severity::Error)
    }

    fn warn(&mut self, code: &'static str, msg: String, rule: Option<&str>, node: Option<&str>) {
        self.diags.push(Diagnostic {
            severity: Severity::Warning,
            code,
            message: msg,
            rule: rule.map(|s| s.into()),
            node: node.map(|s| s.into()),
            port: None,
        });
    }

    fn headers(&mut self) {
        let ids: BTreeSet<&str> = self.packages.iter().map(|p| p.package_id.as_str()).collect();
        if ids.len() != self.packages.len() {
            self.diags.push(err("E_DUPLICATE_PACKAGE", "duplicate package ids"));
        }
        for p in &self.packages {
            if p.schema_version != PACKAGE_SCHEMA_VERSION {
                self.diags.push(err(
                    "E_SCHEMA_VERSION",
                    format!(
                        "package {} has schema_version {}, expected {}",
                        p.package_id, p.schema_version, PACKAGE_SCHEMA_VERSION
                    ),
                ));
            }
            if !valid_id(&p.package_id) {
                self.diags.push(err("E_ID", format!("invalid package id '{}'", p.package_id)));
            }
            for r in &p.requires {
                let base = r.rsplit_once(".v").map(|(b, _)| b).unwrap_or(r);
                if !ids.contains(r.as_str()) && !ids.contains(base) {
                    self.diags.push(err(
                        "E_REQUIRES",
                        format!("package {} requires missing package {}", p.package_id, r),
                    ));
                }
            }
            for cap in &p.capabilities {
                if !SUPPORTED_CAPABILITIES.contains(&cap.as_str()) {
                    self.diags.push(err(
                        "E_CAPABILITY",
                        format!("package {} requires unsupported capability {}", p.package_id, cap),
                    ));
                }
            }
        }
        let rule_count: usize = self.packages.iter().map(|p| p.rules.len()).sum();
        if rule_count > self.env.budgets.max_rules {
            self.diags.push(err(
                "E_BUDGET_RULES",
                format!("{rule_count} rules exceeds limit {}", self.env.budgets.max_rules),
            ));
        }
        let node_count: usize = self.packages.iter().flat_map(|p| &p.rules).map(|r| r.nodes.len()).sum();
        if node_count > self.env.budgets.max_nodes {
            self.diags.push(err(
                "E_BUDGET_NODES",
                format!("{node_count} nodes exceeds the expanded-node limit {}", self.env.budgets.max_nodes),
            ));
        }
    }

    fn parse_unit(&mut self, s: &str, ctx: &str) -> Unit {
        match Unit::parse(s) {
            Ok(u) => u,
            Err(e) => {
                self.diags.push(err("E_UNIT", format!("{ctx}: {e}")));
                DIMENSIONLESS
            }
        }
    }

    fn build_schema(&mut self) {
        let pk = self.packages.clone();
        for p in &pk {
            for r in &p.resources {
                if self.schema.resource(&r.id).is_some() {
                    self.diags.push(err("E_DUPLICATE", format!("resource {} declared twice", r.id)));
                    continue;
                }
                let unit = self.parse_unit(&r.unit, &format!("resource {}", r.id));
                self.schema.resources.push(ResourceInfo {
                    id: r.id.clone(),
                    unit,
                    tol_abs: r.tolerance_abs,
                    tol_rel: r.tolerance_rel,
                });
            }
        }
        for p in &pk {
            for a in &p.accounts {
                if !a.id.starts_with("external.") {
                    self.diags
                        .push(err("E_ACCOUNT", format!("account id {} must start with 'external.'", a.id)));
                    continue;
                }
                if self.schema.account(&a.id).is_some() {
                    self.diags.push(err("E_DUPLICATE", format!("account {} declared twice", a.id)));
                    continue;
                }
                let Some(res) = self.schema.resource(&a.resource) else {
                    self.diags
                        .push(err("E_ACCOUNT", format!("account {} uses unknown resource {}", a.id, a.resource)));
                    continue;
                };
                let (src, sink) = match a.direction.as_str() {
                    "source" => (true, false),
                    "sink" => (false, true),
                    "both" => (true, true),
                    d => {
                        self.diags
                            .push(err("E_ACCOUNT", format!("account {} has invalid direction {d}", a.id)));
                        continue;
                    }
                };
                if src && !p.capabilities.iter().any(|c| c == "sandbox.external_source") {
                    self.diags.push(err(
                        "E_EXTERNAL_SOURCE",
                        format!("account {} creates resource from outside the world; package {} must require capability sandbox.external_source", a.id, p.package_id),
                    ));
                }
                self.schema.accounts.push(AccountInfo {
                    id: a.id.clone(),
                    resource: res,
                    can_source: src,
                    can_sink: sink,
                    label: a.label.clone(),
                });
            }
        }
        for p in &pk {
            for f in &p.fields {
                if let Some(fi) = self.field_info(f, false) {
                    if self.schema.cell_field(&fi.id).is_some() {
                        self.diags.push(err("E_DUPLICATE", format!("field {} declared twice", fi.id)));
                    } else {
                        self.schema.cell_fields.push(fi);
                    }
                }
            }
            for f in &p.forcings {
                let unit = self.parse_unit(&f.unit, &format!("forcing {}", f.id));
                let cells = match f.domain.as_str() {
                    "uniform" => false,
                    "cells" => true,
                    d => {
                        self.diags.push(err("E_DOMAIN", format!("forcing {} has invalid domain {d}", f.id)));
                        false
                    }
                };
                self.schema.forcings.push(ForcingInfo {
                    id: f.id.clone(),
                    unit,
                    quantity: f.quantity.clone(),
                    cells,
                    label: f.label.clone(),
                });
            }
        }
        if self.schema.cell_fields.len() > self.env.budgets.max_cell_fields {
            self.diags.push(err(
                "E_BUDGET_FIELDS",
                format!(
                    "{} cell fields exceeds limit {}",
                    self.schema.cell_fields.len(),
                    self.env.budgets.max_cell_fields
                ),
            ));
        }
        // Resolve cell reservoir capacities (must be parameter fields with the same unit).
        for i in 0..self.schema.cell_fields.len() {
            let f = self.schema.cell_fields[i].clone();
            let decl = pk.iter().flat_map(|p| &p.fields).find(|d| d.id == f.id).unwrap();
            if let Policy::Reservoir { capacity: Some(cap), .. } = &decl.policy {
                match self.schema.cell_field(cap) {
                    Some(ci) => {
                        let cf = &self.schema.cell_fields[ci];
                        if cf.policy != FieldPolicy::Parameter || cf.unit != f.unit {
                            self.diags.push(err(
                                "E_CAPACITY",
                                format!("capacity {cap} of {} must be a parameter field with unit {}", f.id, f.unit),
                            ));
                        } else if let FieldPolicy::Reservoir { capacity, .. } = &mut self.schema.cell_fields[i].policy {
                            *capacity = Some(ci);
                        }
                    }
                    None => self
                        .diags
                        .push(err("E_CAPACITY", format!("unknown capacity field {cap} for {}", f.id))),
                }
            }
        }
        for p in &pk {
            for a in &p.archetypes {
                self.archetype(a);
            }
        }
    }

    fn field_info(&mut self, f: &FieldDecl, entity: bool) -> Option<FieldInfo> {
        if !valid_id(&f.id) {
            self.diags.push(err("E_ID", format!("invalid field id '{}'", f.id)));
            return None;
        }
        let unit = self.parse_unit(&f.unit, &format!("field {}", f.id));
        let grid = if entity {
            0
        } else {
            match self.env.grids.iter().position(|g| *g == f.grid) {
                Some(g) => g as u16,
                None => {
                    self.diags
                        .push(err("E_GRID", format!("field {} is on unknown grid {}", f.id, f.grid)));
                    return None;
                }
            }
        };
        let policy = match &f.policy {
            Policy::Reservoir { resource, .. } => {
                let Some(r) = self.schema.resource(resource) else {
                    self.diags
                        .push(err("E_RESOURCE", format!("field {} uses unknown resource {resource}", f.id)));
                    return None;
                };
                if self.schema.resources[r].unit != unit {
                    self.diags.push(err(
                        "E_UNIT",
                        format!(
                            "reservoir {} has unit {} but resource {resource} is {}",
                            f.id, unit, self.schema.resources[r].unit
                        ),
                    ));
                }
                if f.quantity.as_deref().is_some_and(|q| q != resource) {
                    self.diags
                        .push(err("E_QUANTITY", format!("reservoir {} must carry quantity {resource}", f.id)));
                }
                FieldPolicy::Reservoir {
                    resource: r,
                    capacity: None,
                }
            }
            Policy::Integrated => FieldPolicy::Integrated,
            Policy::NextValue => FieldPolicy::NextValue,
            Policy::Parameter => FieldPolicy::Parameter,
        };
        if !f.default.is_finite() {
            self.diags.push(err("E_VALUE", format!("field {} default is not finite", f.id)));
        }
        let quantity = match &f.policy {
            Policy::Reservoir { resource, .. } => Some(resource.clone()),
            _ => f.quantity.clone(),
        };
        Some(FieldInfo {
            id: f.id.clone(),
            grid,
            unit,
            quantity,
            policy,
            default: f.default,
            min: f.min,
            max: f.max,
            label: f.label.clone(),
        })
    }

    fn archetype(&mut self, a: &ArchetypeDecl) {
        if self.schema.archetype(&a.id).is_some() {
            self.diags.push(err("E_DUPLICATE", format!("archetype {} declared twice", a.id)));
            return;
        }
        if a.brain.backend != "mlp.tanh.v1" {
            self.diags.push(err(
                "E_CAPABILITY",
                format!("archetype {} uses unsupported brain backend {}", a.id, a.brain.backend),
            ));
        }
        if a.brain.inputs == 0
            || a.brain.inputs > 64
            || a.brain.hidden == 0
            || a.brain.hidden > 128
            || a.brain.outputs == 0
            || a.brain.outputs > 32
        {
            self.diags
                .push(err("E_BRAIN", format!("archetype {} brain dimensions out of bounds", a.id)));
        }
        let mut fields = vec![];
        for f in &a.fields {
            if let Some(fi) = self.field_info(f, true) {
                if fields.iter().any(|x: &FieldInfo| x.id == fi.id) {
                    self.diags
                        .push(err("E_DUPLICATE", format!("archetype {} field {} declared twice", a.id, fi.id)));
                }
                fields.push(fi);
            }
        }
        let mut traits = vec![];
        for t in &a.traits {
            let unit = self.parse_unit(&t.unit, &format!("trait {}", t.id));
            if !(t.min <= t.init_min && t.init_min <= t.init_max && t.init_max <= t.max) || !t.min.is_finite() || !t.max.is_finite() {
                self.diags.push(err(
                    "E_TRAIT",
                    format!("trait {} ranges must satisfy min <= init_min <= init_max <= max", t.id),
                ));
            }
            traits.push(TraitInfo {
                id: t.id.clone(),
                unit,
                min: t.min,
                max: t.max,
                init_min: t.init_min,
                init_max: t.init_max,
                label: t.label.clone(),
            });
        }
        let mut disposal = vec![];
        for d in &a.death_disposal {
            let Some(fi) = fields.iter().position(|f| f.id == d.field) else {
                self.diags
                    .push(err("E_DISPOSAL", format!("archetype {} disposes unknown field {}", a.id, d.field)));
                continue;
            };
            let FieldPolicy::Reservoir { resource, .. } = fields[fi].policy else {
                self.diags.push(err(
                    "E_DISPOSAL",
                    format!("archetype {} disposes non-reservoir field {}", a.id, d.field),
                ));
                continue;
            };
            let ep = if let Some(cf) = d.to.strip_prefix("cell.") {
                match self.schema.cell_field(cf) {
                    Some(ci) if matches!(self.schema.cell_fields[ci].policy, FieldPolicy::Reservoir { resource: r, .. } if r == resource) => {
                        Endpoint::Cell(ci)
                    }
                    _ => {
                        self.diags.push(err(
                            "E_DISPOSAL",
                            format!("death disposal target {} is not a reservoir of the same resource", d.to),
                        ));
                        continue;
                    }
                }
            } else {
                match self.schema.account(&d.to) {
                    Some(ac) if self.schema.accounts[ac].resource == resource && self.schema.accounts[ac].can_sink => {
                        Endpoint::External(ac)
                    }
                    _ => {
                        self.diags.push(err(
                            "E_DISPOSAL",
                            format!("death disposal target {} is not a sink account for this resource", d.to),
                        ));
                        continue;
                    }
                }
            };
            disposal.push((fi, ep));
        }
        for (fi, f) in fields.iter().enumerate() {
            if matches!(f.policy, FieldPolicy::Reservoir { .. }) && !disposal.iter().any(|(d, _)| *d == fi) {
                self.diags.push(err(
                    "E_DISPOSAL",
                    format!(
                        "archetype {}: conserved reservoir {} has no death disposal; deleting an entity is not a resource sink",
                        a.id, f.id
                    ),
                ));
            }
        }
        if !(0.0..=1.0).contains(&a.mutation.probability) || !(0.0..=1.0).contains(&a.mutation.scale) {
            self.diags.push(err(
                "E_MUTATION",
                format!("archetype {} mutation probability/scale must be in [0,1]", a.id),
            ));
        }
        let seed_state = fields
            .iter()
            .map(|f| a.seed_state.get(&f.id).copied().unwrap_or(f.default))
            .collect();
        for k in a.seed_state.keys() {
            if !fields.iter().any(|f| &f.id == k) {
                self.diags.push(err(
                    "E_SEED_STATE",
                    format!("archetype {} seed_state names unknown field {k}", a.id),
                ));
            }
        }
        let n = fields.len();
        self.schema.archetypes.push(ArchInfo {
            id: a.id.clone(),
            label: a.label.clone(),
            fields,
            capacity_slots: vec![None; n],
            traits,
            brain: a.brain.clone(),
            max_speed: a.max_speed,
            max_turn_rate: a.max_turn_rate,
            disposal,
            mutation: a.mutation.clone(),
            cooldown: a.reproduction_cooldown,
            seed_state,
        });
    }

    fn param(&mut self, qualified: String, p: &ParamDecl) -> usize {
        let unit = self.parse_unit(&p.unit, &format!("parameter {qualified}"));
        if !(p.min <= p.value && p.value <= p.max) || !p.value.is_finite() {
            self.diags.push(err(
                "E_PARAM_RANGE",
                format!(
                    "parameter {qualified} = {} is outside its declared range [{}, {}]",
                    p.value, p.min, p.max
                ),
            ));
        }
        let _ = unit;
        self.params.push(ParamInfo {
            name: qualified,
            value: p.value,
            unit: p.unit.clone(),
            min: p.min,
            max: p.max,
            label: p.label.clone(),
            stability: p.stability.clone(),
            density_param: None,
        });
        self.params.len() - 1
    }

    fn build_params(&mut self) {
        let pk = self.packages.clone();
        for p in &pk {
            let mut m = BTreeMap::new();
            for (name, decl) in &p.parameters {
                let idx = self.param(format!("{}:{}", p.package_id, name), decl);
                m.insert(name.clone(), idx);
            }
            self.package_params.insert(p.package_id.clone(), m);
        }
        // Stability checks need all params for density lookups; done per rule below too.
        for p in &pk {
            for (name, decl) in &p.parameters {
                self.check_stability(&p.package_id, None, name, decl);
            }
        }
    }

    fn lookup_param_value(&self, package: &str, rule: Option<&Rule>, name: &str) -> Option<f64> {
        if let Some(r) = rule
            && let Some(d) = r.parameters.get(name)
        {
            return Some(d.value);
        }
        self.packages
            .iter()
            .find(|p| p.package_id == package)
            .and_then(|p| p.parameters.get(name))
            .map(|d| d.value)
    }

    fn check_stability(&mut self, package: &str, rule: Option<&Rule>, name: &str, decl: &ParamDecl) {
        let Some(st) = &decl.stability else { return };
        let dt = self.env.dt;
        let dx = self.env.cell_size;
        let (lhs, limit, what) = match st {
            Stability::Diffusion => (
                dt * decl.value * (2.0 / (dx * dx) + 2.0 / (dx * dx)),
                1.0,
                "explicit diffusion dt*D*(2/dx^2+2/dz^2)",
            ),
            Stability::FirstOrder => (dt * decl.value, 1.0, "first-order withdrawal dt*k"),
            Stability::Transport { density_param, limit } => {
                let Some(rho) = self.lookup_param_value(package, rule, density_param) else {
                    self.diags.push(err(
                        "E_STABILITY",
                        format!("stability of {name} references unknown density parameter {density_param}"),
                    ));
                    return;
                };
                let qualified = match rule {
                    Some(r) if r.parameters.contains_key(density_param) => {
                        format!("{}:{density_param}", r.rule_id)
                    }
                    _ => format!("{package}:{density_param}"),
                };
                let me = match rule {
                    Some(r) => format!("{}:{name}", r.rule_id),
                    None => format!("{package}:{name}"),
                };
                let di = self.params.iter().position(|p| p.name == qualified);
                if let Some(pi) = self.params.iter().position(|p| p.name == me) {
                    self.params[pi].density_param = di;
                }
                (
                    dt * decl.value * 4.0 / (rho * dx * dx),
                    *limit,
                    "head transport dt*c*4/(density*area)",
                )
            }
        };
        if lhs > limit {
            let mut d = err(
                "E_NUMERIC_RISK",
                format!(
                    "parameter {name} = {} violates the {what} limit: {lhs:.4} > {limit} at dt = {dt} s, dx = {dx} m",
                    decl.value
                ),
            );
            if let Some(r) = rule {
                d.rule = Some(r.rule_id.clone());
            }
            self.diags.push(d);
        }
    }

    fn collect_rules(&mut self) {
        let pk = self.packages.clone();
        let mut rule_ids = BTreeSet::new();
        for p in &pk {
            for r in &p.rules {
                if !valid_id(&r.rule_id) || r.rule_id.contains('/') {
                    self.diags.push(err("E_ID", format!("invalid rule id '{}'", r.rule_id)));
                    continue;
                }
                if !rule_ids.insert(r.rule_id.clone()) {
                    self.diags.push(err("E_DUPLICATE", format!("rule {} declared twice", r.rule_id)));
                    continue;
                }
                if !r.enabled {
                    continue;
                }
                let (dom, arch) = match r.domain.kind.as_str() {
                    "cells" | "edges" => {
                        let g = r.domain.grid.clone().unwrap_or_else(|| self.env.grids[0].clone());
                        let Some(gi) = self.env.grids.iter().position(|x| *x == g) else {
                            self.diags
                                .push(err("E_GRID", format!("rule {} targets unknown grid {g}", r.rule_id)));
                            continue;
                        };
                        if gi != 0 {
                            self.diags.push(err(
                                "E_CAPABILITY",
                                format!("rule {}: only the primary grid is simulated", r.rule_id),
                            ));
                        }
                        if r.domain.kind == "cells" {
                            (Dom::Cells(gi as u16), None)
                        } else {
                            (Dom::Edges(gi as u16), None)
                        }
                    }
                    "entities" => {
                        let Some(a) = r.domain.archetype.as_deref().and_then(|a| self.schema.archetype(a)) else {
                            self.diags.push(err(
                                "E_DOMAIN",
                                format!("rule {} targets unknown archetype {:?}", r.rule_id, r.domain.archetype),
                            ));
                            continue;
                        };
                        (Dom::Entities(a as u16), Some(a as u16))
                    }
                    k => {
                        self.diags
                            .push(err("E_DOMAIN", format!("rule {} has invalid domain kind {k}", r.rule_id)));
                        continue;
                    }
                };
                let mut params = BTreeMap::new();
                for (name, decl) in &r.parameters {
                    let idx = self.param(format!("{}:{}", r.rule_id, name), decl);
                    params.insert(name.clone(), idx);
                }
                for (name, decl) in &r.parameters {
                    self.check_stability(&p.package_id, Some(r), name, decl);
                }
                let ri = self.rules.len();
                let mut seen = BTreeSet::new();
                for n in &r.nodes {
                    if !valid_id(&n.id) || n.id.contains('/') || n.id.contains('.') {
                        self.diags
                            .push(err("E_ID", format!("invalid node id '{}'", n.id)).at(&r.rule_id, None, None));
                        continue;
                    }
                    if !seen.insert(n.id.clone()) {
                        self.diags
                            .push(err("E_DUPLICATE", format!("node {} declared twice", n.id)).at(&r.rule_id, None, None));
                        continue;
                    }
                    self.nodes.insert(format!("{}/{}", r.rule_id, n.id), NodeRef { rule: ri, node: n });
                }
                self.rules.push((
                    r,
                    RuleCtx {
                        id: r.rule_id.clone(),
                        package: p.package_id.clone(),
                        dom,
                        arch,
                        params,
                    },
                ));
            }
        }
    }

    // ---- slots and instructions -------------------------------------------------

    fn new_slot(&mut self, name: String, t: Type) -> Slot {
        self.slots.push(SlotInfo {
            name,
            dom: t.dom,
            stage: t.stage,
            unit: t.unit.to_string(),
            quantity: t.quantity.clone(),
            is_bool: t.kind == Kind::Bool,
        });
        self.types.push(t);
        self.slots.len() - 1
    }

    fn emit(&mut self, op: Op, inputs: Vec<Slot>, t: Type, source: &str) -> Slot {
        let s = self.new_slot(source.to_string(), t.clone());
        if let Op::Const(v) = op {
            self.const_values.insert(s, v);
        }
        self.instrs.push(Instr {
            op,
            inputs,
            outputs: vec![s],
            dom: t.dom,
            stage: t.stage,
            source: source.to_string(),
        });
        s
    }

    fn resolve(&mut self, key: &str) -> R<Vec<Slot>> {
        match self.visit.get(key) {
            Some(Visit::Done) => return Ok(self.outputs[key].clone()),
            Some(Visit::Active) => {
                let start = self.stack.iter().position(|(k, _)| k == key).unwrap_or(0);
                let cycle: Vec<&str> = self.stack[start..].iter().map(|(k, _)| k.as_str()).collect();
                let through_receipt = self.stack[start..].iter().any(|(_, r)| *r);
                let (rule, node) = key.split_once('/').unwrap_or((key, ""));
                return Err(if through_receipt {
                    err(
                        "E_RECEIPT_FEEDBACK",
                        format!(
                            "same-tick feedback: an accepted receipt feeds back into an earlier request ({} -> {key}); route it through stored state",
                            cycle.join(" -> ")
                        ),
                    )
                    .at(rule, Some(node), None)
                } else {
                    err(
                        "E_CYCLE",
                        format!(
                            "algebraic cycle {} -> {key}; introduce stored state or an explicit delay (the compiler never inserts hidden delays)",
                            cycle.join(" -> ")
                        ),
                    )
                    .at(rule, Some(node), None)
                });
            }
            None => {}
        }
        let Some(nr) = self.nodes.get(key) else {
            return Err(err("E_UNKNOWN_NODE", format!("unknown node {key}")));
        };
        let (rule_idx, node) = (nr.rule, nr.node);
        self.visit.insert(key.to_string(), Visit::Active);
        self.stack.push((key.to_string(), false));
        let res = self.lower(rule_idx, node, key);
        self.stack.pop();
        match res {
            Ok(outs) => {
                self.visit.insert(key.to_string(), Visit::Done);
                self.outputs.insert(key.to_string(), outs.clone());
                self.source_map.insert(key.to_string(), outs.clone());
                Ok(outs)
            }
            Err(d) => {
                let rid = self.rules[rule_idx].1.id.clone();
                // Mark done with no outputs to avoid duplicate cascading diagnostics.
                self.visit.insert(key.to_string(), Visit::Done);
                self.outputs.insert(key.to_string(), vec![]);
                Err(d.at(&rid, Some(&node.id), None))
            }
        }
    }

    /// Resolve a reference like `node`, `node.port`, or `rule/node.port`.
    fn input(&mut self, rule_idx: usize, reference: &str, port_ctx: &str) -> R<(Slot, Type)> {
        let rid = self.rules[rule_idx].1.id.clone();
        let (node_part, port) = match reference.rsplit_once('.') {
            Some((n, p)) if !p.contains('/') => (n, p),
            _ => (reference, "value"),
        };
        let key = if node_part.contains('/') {
            node_part.to_string()
        } else {
            format!("{rid}/{node_part}")
        };
        if !self.nodes.contains_key(&key) {
            return Err(err("E_UNKNOWN_NODE", format!("input '{reference}' refers to unknown node {key}")).at(&rid, None, Some(port_ctx)));
        }
        let is_receipt = matches!(port, "accepted" | "accepted_rate" | "fraction") || port.starts_with("leg");
        if let Some(top) = self.stack.last_mut() {
            top.1 = is_receipt;
        }
        let outs = self.resolve(&key)?;
        if outs.is_empty() {
            return Err(err("E_UPSTREAM", format!("input '{reference}' depends on an invalid node")).at(&rid, None, Some(port_ctx)));
        }
        let slot = if let Some(&ei) = self.effect_of.get(&key) {
            self.receipt_slot(ei, port, &key)?
        } else if port == "value" && outs.len() == 1 {
            outs[0]
        } else if let Some(n) = port.strip_prefix("out") {
            let i: usize = if n.is_empty() {
                0
            } else {
                n.parse().map_err(|_| err("E_PORT", format!("unknown port {port} on {key}")))?
            };
            *outs
                .get(i)
                .ok_or_else(|| err("E_PORT", format!("node {key} has no port {port}")).at(&rid, None, Some(port_ctx)))?
        } else {
            return Err(err("E_PORT", format!("node {key} has no port '{port}'")).at(&rid, None, Some(port_ctx)));
        };
        Ok((slot, self.types[slot].clone()))
    }

    fn receipt_slot(&mut self, ei: usize, port: &str, key: &str) -> R<Slot> {
        let (leg, form) = match port {
            "accepted" => (0, ReceiptForm::Amount),
            "accepted_rate" => (0, ReceiptForm::Rate),
            "fraction" => (0, ReceiptForm::Fraction),
            p if p.starts_with("leg") => {
                let (n, f) = p[3..].split_once('_').unwrap_or((&p[3..], "accepted"));
                let n: usize = n.parse().map_err(|_| err("E_PORT", format!("bad receipt port {p}")))?;
                let form = match f {
                    "accepted" => ReceiptForm::Amount,
                    "accepted_rate" => ReceiptForm::Rate,
                    _ => return Err(err("E_PORT", format!("bad receipt port {p}"))),
                };
                (n, form)
            }
            p => {
                return Err(err(
                    "E_PORT",
                    format!("effect {key} exposes accepted, accepted_rate, fraction, legN_accepted; not '{p}'"),
                ));
            }
        };
        let e = &self.effects[ei];
        let (unit, quantity) = match &e.kind {
            EffectKind::Process { legs } => {
                let Some(l) = legs.get(leg) else {
                    return Err(err("E_PORT", format!("effect {key} has no leg {leg}")));
                };
                let r = &self.schema.resources[l.resource];
                (r.unit, Some(r.id.clone()))
            }
            EffectKind::EdgeTransfer { resource, .. } => {
                if leg != 0 {
                    return Err(err("E_PORT", format!("effect {key} has no leg {leg}")));
                }
                let r = &self.schema.resources[*resource];
                (r.unit, Some(r.id.clone()))
            }
            _ => {
                return Err(err("E_PORT", format!("effect {key} is not a resource process and has no receipt")));
            }
        };
        let (unit, quantity) = match form {
            ReceiptForm::Amount => (unit, quantity),
            ReceiptForm::Rate => (unit.div(&Unit::new(0, 0, 1, 0)).unwrap(), quantity),
            ReceiptForm::Fraction => (DIMENSIONLESS, None),
        };
        let dom = self.effects[ei].dom;
        let t = Type {
            kind: Kind::Number,
            unit,
            quantity,
            dom,
            stage: Stage::Receipt,
        };
        Ok(self.emit(Op::Receipt { effect: ei, leg, form }, vec![], t, &format!("{key}.{port}")))
    }

    fn input_list(&mut self, rule_idx: usize, node: &Node, n: Option<usize>) -> R<Vec<(Slot, Type)>> {
        let list = match node.args.get("inputs") {
            Some(Value::Array(a)) => a.clone(),
            None if n == Some(0) => vec![],
            _ => {
                return Err(err("E_ARGS", format!("op {} requires an 'inputs' array", node.op)));
            }
        };
        if let Some(n) = n
            && list.len() != n
        {
            return Err(err("E_ARITY", format!("op {} takes {n} inputs, got {}", node.op, list.len())));
        }
        let mut out = vec![];
        for (i, v) in list.iter().enumerate() {
            let Value::String(s) = v else {
                return Err(err("E_ARGS", format!("input {i} must be a node reference string")));
            };
            out.push(self.input(rule_idx, s, &format!("inputs[{i}]"))?);
        }
        Ok(out)
    }

    fn named_input(&mut self, rule_idx: usize, node: &Node, key: &str) -> R<(Slot, Type)> {
        match node.args.get(key) {
            Some(Value::String(s)) => self.input(rule_idx, s, key),
            _ => Err(err("E_ARGS", format!("op {} requires input '{key}'", node.op))),
        }
    }

    fn unify_dom(&self, a: Dom, b: Dom) -> R<Dom> {
        match (a, b) {
            (Dom::Uniform, x) | (x, Dom::Uniform) => Ok(x),
            (x, y) if x == y => Ok(x),
            (x, y) => {
                let g = &self.schema.grids;
                let an = self.schema.arch_names();
                Err(err(
                    "E_DOMAIN",
                    format!(
                        "cannot combine {} with {} without an explicit domain operation (sample, edge_from/edge_to, resample)",
                        x.describe(g, &an),
                        y.describe(g, &an)
                    ),
                ))
            }
        }
    }

    fn join_quantity(a: &Option<String>, b: &Option<String>, op: &str) -> R<Option<String>> {
        match (a, b) {
            (None, x) | (x, None) => Ok(x.clone()),
            (Some(x), Some(y)) if x == y => Ok(Some(x.clone())),
            (Some(x), Some(y)) => Err(err(
                "E_QUANTITY",
                format!(
                    "cannot {op} {x} with {y}: same units do not make different quantities interchangeable; use an explicit conversion or reaction"
                ),
            )),
        }
    }

    fn num(&self, t: &Type, what: &str) -> R<()> {
        if t.kind != Kind::Number {
            return Err(err("E_TYPE", format!("{what} must be a number, got bool")));
        }
        Ok(())
    }

    fn lower(&mut self, ri: usize, node: &Node, key: &str) -> R<Vec<Slot>> {
        let rule_dom = self.rules[ri].1.dom;
        let arch = self.rules[ri].1.arch;
        let n_type = |unit: Unit, quantity: Option<String>, dom: Dom, stage: Stage| Type {
            kind: Kind::Number,
            unit,
            quantity,
            dom,
            stage,
        };
        let b_type = |dom: Dom, stage: Stage| Type {
            kind: Kind::Bool,
            unit: DIMENSIONLESS,
            quantity: None,
            dom,
            stage,
        };
        let cells = Dom::Cells(0);
        let op = node.op.as_str();
        let slot = match op {
            "const" => {
                let v = arg_f64(node, "value")?;
                let unit = self.unit_arg(node, "unit")?;
                let q = arg_opt_str(node, "quantity");
                self.emit(Op::Const(v), vec![], n_type(unit, q, Dom::Uniform, Stage::Snapshot), key)
            }
            "parameter" => {
                let name = arg_str(node, "name")?;
                let rc = &self.rules[ri].1;
                let idx = rc
                    .params
                    .get(&name)
                    .copied()
                    .or_else(|| self.package_params.get(&rc.package).and_then(|m| m.get(&name).copied()));
                let Some(idx) = idx else {
                    return Err(err("E_UNKNOWN_PARAM", format!("unknown parameter {name}")));
                };
                let decl_unit = Unit::parse(&self.params[idx].unit).unwrap_or(DIMENSIONLESS);
                let q = self.param_quantity(ri, &name);
                self.emit(Op::Param(idx), vec![], n_type(decl_unit, q, Dom::Uniform, Stage::Snapshot), key)
            }
            "read_state" | "candidate_state" => {
                let field = arg_str(node, "field")?;
                let candidate = op == "candidate_state";
                if let Some(ef) = field.strip_prefix("self.") {
                    let Some(a) = arch else {
                        return Err(err("E_DOMAIN", "self.<field> is only valid in an entities rule"));
                    };
                    let ai = &self.schema.archetypes[a as usize];
                    let Some(fi) = ai.field(ef) else {
                        return Err(err("E_UNKNOWN_FIELD", format!("archetype {} has no field {ef}", ai.id)));
                    };
                    let f = &ai.fields[fi];
                    let t = n_type(
                        f.unit,
                        f.quantity.clone(),
                        Dom::Entities(a),
                        if candidate { Stage::Candidate } else { Stage::Snapshot },
                    );
                    let o = if candidate {
                        Op::CandidateEntity(a, fi)
                    } else {
                        Op::ReadEntity(a, fi)
                    };
                    self.emit(o, vec![], t, key)
                } else {
                    if candidate {
                        return Err(err(
                            "E_STAGE",
                            "candidate_state is only available for self.<field> in lifecycle rules",
                        ));
                    }
                    let fname = field.strip_prefix("cell.").unwrap_or(&field);
                    let Some(fi) = self.schema.cell_field(fname) else {
                        return Err(err("E_UNKNOWN_FIELD", format!("unknown cell field {fname}")));
                    };
                    let f = &self.schema.cell_fields[fi];
                    let t = n_type(f.unit, f.quantity.clone(), Dom::Cells(f.grid), Stage::Snapshot);
                    self.emit(Op::ReadCell(fi), vec![], t, key)
                }
            }
            "trait" => {
                let Some(a) = arch else {
                    return Err(err("E_DOMAIN", "trait is only valid in an entities rule"));
                };
                let name = arg_str(node, "name")?;
                let ai = &self.schema.archetypes[a as usize];
                let Some(ti) = ai.traits.iter().position(|t| t.id == name) else {
                    return Err(err("E_UNKNOWN_TRAIT", format!("archetype {} has no trait {name}", ai.id)));
                };
                let unit = ai.traits[ti].unit;
                self.emit(Op::Trait(a, ti), vec![], n_type(unit, None, Dom::Entities(a), Stage::Snapshot), key)
            }
            "builtin" => {
                let Some(a) = arch else {
                    return Err(err("E_DOMAIN", "builtin is only valid in an entities rule"));
                };
                let name = arg_str(node, "name")?;
                let (b, unit) = match name.as_str() {
                    "age" => (Builtin::Age, Unit::new(0, 0, 1, 0)),
                    "heading" => (Builtin::Heading, DIMENSIONLESS),
                    "cooldown" => (Builtin::Cooldown, Unit::new(0, 0, 1, 0)),
                    "generation" => (Builtin::Generation, DIMENSIONLESS),
                    _ => {
                        return Err(err(
                            "E_ARGS",
                            format!("unknown builtin {name} (age, heading, cooldown, generation)"),
                        ));
                    }
                };
                self.emit(
                    Op::Builtin(a, b),
                    vec![],
                    n_type(unit, None, Dom::Entities(a), Stage::Snapshot),
                    key,
                )
            }
            "read_forcing" => {
                let name = arg_str(node, "forcing")?;
                let Some(fi) = self.schema.forcing(&name) else {
                    return Err(err("E_UNKNOWN_FORCING", format!("unknown forcing {name}")));
                };
                let f = &self.schema.forcings[fi];
                let dom = if f.cells { cells } else { Dom::Uniform };
                self.emit(
                    Op::Forcing(fi),
                    vec![],
                    n_type(f.unit, f.quantity.clone(), dom, Stage::Snapshot),
                    key,
                )
            }
            "region" => {
                let name = arg_str(node, "region")?;
                let Some(r) = self.schema.region(&name) else {
                    return Err(err("E_UNKNOWN_REGION", format!("unknown region {name}")));
                };
                self.emit(Op::Region(r), vec![], n_type(DIMENSIONLESS, None, cells, Stage::Snapshot), key)
            }
            "cell_area" => self.emit(
                Op::CellArea,
                vec![],
                n_type(Unit::new(0, 2, 0, 0), None, Dom::Uniform, Stage::Snapshot),
                key,
            ),
            "cell_size" => self.emit(
                Op::CellSize,
                vec![],
                n_type(Unit::new(0, 1, 0, 0), None, Dom::Uniform, Stage::Snapshot),
                key,
            ),
            "is_boundary" => self.emit(Op::IsBoundary, vec![], b_type(cells, Stage::Snapshot), key),
            "add" | "sub" => {
                let ins = self.input_list(ri, node, Some(2))?;
                let (a, b) = (&ins[0].1, &ins[1].1);
                self.num(a, "operand")?;
                self.num(b, "operand")?;
                let unit = if op == "add" { a.unit.add(&b.unit) } else { a.unit.sub(&b.unit) }.map_err(|e| {
                    err(
                        "E_UNIT",
                        format!(
                            "{e}: inputs[0] is {} and inputs[1] is {}",
                            a.describe(&self.schema.grids, &self.schema.arch_names()),
                            b.describe(&self.schema.grids, &self.schema.arch_names())
                        ),
                    )
                })?;
                let q = Self::join_quantity(&a.quantity, &b.quantity, if op == "add" { "add" } else { "subtract" })?;
                let dom = self.unify_dom(a.dom, b.dom)?;
                let st = a.stage.max(b.stage);
                let o = if op == "add" { Op::Add } else { Op::Sub };
                self.emit(o, vec![ins[0].0, ins[1].0], n_type(unit, q, dom, st), key)
            }
            "multiply" | "mul" => {
                let ins = self.input_list(ri, node, Some(2))?;
                let (a, b) = (&ins[0].1, &ins[1].1);
                self.num(a, "operand")?;
                self.num(b, "operand")?;
                let unit = a.unit.mul(&b.unit).map_err(|e| err("E_UNIT", e))?;
                let q = if unit.is_dimensionless() {
                    None
                } else {
                    mul_quantity(&a.quantity, &b.quantity)
                };
                let dom = self.unify_dom(a.dom, b.dom)?;
                self.emit(Op::Mul, vec![ins[0].0, ins[1].0], n_type(unit, q, dom, a.stage.max(b.stage)), key)
            }
            "safe_divide" => {
                let ins = self.input_list(ri, node, Some(3))?;
                let (a, b, f) = (&ins[0].1, &ins[1].1, &ins[2].1);
                for t in [a, b, f] {
                    self.num(t, "operand")?;
                }
                let unit = a.unit.div(&b.unit).map_err(|e| err("E_UNIT", e))?;
                if f.unit != unit {
                    return Err(err(
                        "E_UNIT",
                        format!("safe_divide fallback must have the result unit {unit}, got {}", f.unit),
                    ));
                }
                let q = if unit.is_dimensionless() {
                    None
                } else {
                    div_quantity(&a.quantity, &b.quantity)
                };
                let dom = self.unify_dom(self.unify_dom(a.dom, b.dom)?, f.dom)?;
                let st = a.stage.max(b.stage).max(f.stage);
                self.emit(Op::SafeDiv, vec![ins[0].0, ins[1].0, ins[2].0], n_type(unit, q, dom, st), key)
            }
            "neg" | "abs" => {
                let ins = self.input_list(ri, node, Some(1))?;
                let a = &ins[0].1;
                self.num(a, "operand")?;
                if a.unit.absolute {
                    return Err(err("E_UNIT", format!("{op} of an absolute temperature is meaningless")));
                }
                let o = if op == "neg" { Op::Neg } else { Op::Abs };
                self.emit(o, vec![ins[0].0], a.clone(), key)
            }
            "min" | "max" => {
                let ins = self.input_list(ri, node, Some(2))?;
                let (a, b) = (&ins[0].1, &ins[1].1);
                self.num(a, "operand")?;
                self.num(b, "operand")?;
                if !a.unit.same(&b.unit) {
                    return Err(err("E_UNIT", format!("{op} requires equal units, got {} and {}", a.unit, b.unit)));
                }
                let q = Self::join_quantity(&a.quantity, &b.quantity, op)?;
                let dom = self.unify_dom(a.dom, b.dom)?;
                let o = if op == "min" { Op::Min } else { Op::Max };
                self.emit(o, vec![ins[0].0, ins[1].0], n_type(a.unit, q, dom, a.stage.max(b.stage)), key)
            }
            "clamp" => {
                let ins = self.input_list(ri, node, Some(3))?;
                let (x, lo, hi) = (&ins[0].1, &ins[1].1, &ins[2].1);
                for t in [x, lo, hi] {
                    self.num(t, "operand")?;
                }
                if !(x.unit.same(&lo.unit) && x.unit.same(&hi.unit)) {
                    return Err(err(
                        "E_UNIT",
                        format!("clamp requires equal units, got {}, {}, {}", x.unit, lo.unit, hi.unit),
                    ));
                }
                let dom = self.unify_dom(self.unify_dom(x.dom, lo.dom)?, hi.dom)?;
                let st = x.stage.max(lo.stage).max(hi.stage);
                self.emit(
                    Op::Clamp,
                    vec![ins[0].0, ins[1].0, ins[2].0],
                    n_type(x.unit, x.quantity.clone(), dom, st),
                    key,
                )
            }
            "lt" | "le" | "gt" | "ge" => {
                let ins = self.input_list(ri, node, Some(2))?;
                let (a, b) = (&ins[0].1, &ins[1].1);
                self.num(a, "operand")?;
                self.num(b, "operand")?;
                if !a.unit.same(&b.unit) {
                    return Err(err(
                        "E_UNIT",
                        format!("comparison requires equal units, got {} and {}", a.unit, b.unit),
                    ));
                }
                Self::join_quantity(&a.quantity, &b.quantity, "compare")?;
                let dom = self.unify_dom(a.dom, b.dom)?;
                let o = match op {
                    "lt" => Op::Lt,
                    "le" => Op::Le,
                    "gt" => Op::Gt,
                    _ => Op::Ge,
                };
                self.emit(o, vec![ins[0].0, ins[1].0], b_type(dom, a.stage.max(b.stage)), key)
            }
            "and" | "or" => {
                let ins = self.input_list(ri, node, Some(2))?;
                let (a, b) = (&ins[0].1, &ins[1].1);
                if a.kind != Kind::Bool || b.kind != Kind::Bool {
                    return Err(err("E_TYPE", format!("{op} requires bool inputs")));
                }
                let dom = self.unify_dom(a.dom, b.dom)?;
                let o = if op == "and" { Op::And } else { Op::Or };
                self.emit(o, vec![ins[0].0, ins[1].0], b_type(dom, a.stage.max(b.stage)), key)
            }
            "not" => {
                let ins = self.input_list(ri, node, Some(1))?;
                if ins[0].1.kind != Kind::Bool {
                    return Err(err("E_TYPE", "not requires a bool input"));
                }
                let t = ins[0].1.clone();
                self.emit(Op::Not, vec![ins[0].0], t, key)
            }
            "select" => {
                let ins = self.input_list(ri, node, Some(3))?;
                let (c, a, b) = (&ins[0].1, &ins[1].1, &ins[2].1);
                if c.kind != Kind::Bool {
                    return Err(err("E_TYPE", "select condition (inputs[0]) must be bool"));
                }
                if a.kind != b.kind || !a.unit.same(&b.unit) {
                    return Err(err(
                        "E_UNIT",
                        format!("select branches must have equal types, got {} and {}", a.unit, b.unit),
                    ));
                }
                let q = Self::join_quantity(&a.quantity, &b.quantity, "select between")?;
                let dom = self.unify_dom(self.unify_dom(c.dom, a.dom)?, b.dom)?;
                let st = c.stage.max(a.stage).max(b.stage);
                let t = Type {
                    kind: a.kind,
                    unit: a.unit,
                    quantity: q,
                    dom,
                    stage: st,
                };
                self.emit(Op::Select, vec![ins[0].0, ins[1].0, ins[2].0], t, key)
            }
            "lerp" => {
                let ins = self.input_list(ri, node, Some(3))?;
                let (a, b, t) = (&ins[0].1, &ins[1].1, &ins[2].1);
                for x in [a, b, t] {
                    self.num(x, "operand")?;
                }
                if !a.unit.same(&b.unit) || !t.unit.is_dimensionless() {
                    return Err(err("E_UNIT", "lerp requires equal endpoint units and a dimensionless t"));
                }
                let q = Self::join_quantity(&a.quantity, &b.quantity, "interpolate")?;
                let dom = self.unify_dom(self.unify_dom(a.dom, b.dom)?, t.dom)?;
                let st = a.stage.max(b.stage).max(t.stage);
                self.emit(Op::Lerp, vec![ins[0].0, ins[1].0, ins[2].0], n_type(a.unit, q, dom, st), key)
            }
            "exp" | "ln" | "sin" | "cos" | "tanh" => {
                let ins = self.input_list(ri, node, Some(1))?;
                let a = &ins[0].1;
                self.num(a, "operand")?;
                if !a.unit.is_dimensionless() {
                    return Err(err("E_UNIT", format!("{op} requires a dimensionless argument, got {}", a.unit)));
                }
                let o = match op {
                    "exp" => Op::Exp,
                    "ln" => Op::Ln,
                    "sin" => Op::Sin,
                    "cos" => Op::Cos,
                    _ => Op::Tanh,
                };
                self.emit(o, vec![ins[0].0], n_type(DIMENSIONLESS, None, a.dom, a.stage), key)
            }
            "pow" => {
                let ins = self.input_list(ri, node, Some(2))?;
                let (a, b) = (&ins[0].1, &ins[1].1);
                if !a.unit.is_dimensionless() || !b.unit.is_dimensionless() {
                    return Err(err("E_UNIT", "pow requires dimensionless base and exponent"));
                }
                let dom = self.unify_dom(a.dom, b.dom)?;
                self.emit(
                    Op::Pow,
                    vec![ins[0].0, ins[1].0],
                    n_type(DIMENSIONLESS, None, dom, a.stage.max(b.stage)),
                    key,
                )
            }
            "curve" => {
                let ins = self.input_list(ri, node, Some(1))?;
                let a = &ins[0].1;
                self.num(a, "operand")?;
                let in_unit = self.unit_arg(node, "in_unit")?;
                let out_unit = self.unit_arg(node, "out_unit")?;
                if a.unit != in_unit {
                    return Err(err(
                        "E_UNIT",
                        format!("curve declares input unit {in_unit} but receives {}", a.unit),
                    ));
                }
                let pts = match node.args.get("points") {
                    Some(Value::Array(p)) => p,
                    _ => return Err(err("E_ARGS", "curve requires 'points': [[x, y], ...]")),
                };
                if pts.len() < 2 || pts.len() > MAX_CURVE_POINTS {
                    return Err(err("E_ARGS", format!("curve needs 2..={MAX_CURVE_POINTS} points")));
                }
                let mut v = vec![];
                for p in pts {
                    let xy = p
                        .as_array()
                        .filter(|a| a.len() == 2)
                        .and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)));
                    let Some((x, y)) = xy.filter(|(x, y)| x.is_finite() && y.is_finite()) else {
                        return Err(err("E_ARGS", "curve points must be finite [x, y] pairs"));
                    };
                    if let Some(&(px, _)) = v.last()
                        && x <= px
                    {
                        return Err(err("E_ARGS", "curve x values must be strictly increasing"));
                    }
                    v.push((x, y));
                }
                let q = arg_opt_str(node, "quantity");
                self.emit(Op::Curve(v), vec![ins[0].0], n_type(out_unit, q, a.dom, a.stage), key)
            }
            "as_quantity" => {
                let ins = self.input_list(ri, node, Some(1))?;
                let mut t = ins[0].1.clone();
                t.quantity = arg_opt_str(node, "quantity");
                self.emit(Op::Relabel, vec![ins[0].0], t, key)
            }
            "laplacian" | "gradient_x" | "gradient_z" | "neighbor_sum" | "neighbor_mean" => {
                let ins = self.input_list(ri, node, Some(1))?;
                let a = &ins[0].1;
                self.num(a, "operand")?;
                if !matches!(a.dom, Dom::Cells(_)) {
                    return Err(err("E_DOMAIN", format!("{op} requires a cells input")));
                }
                let m = Unit::new(0, 1, 0, 0);
                let (o, unit) = match op {
                    "laplacian" => (Op::Laplacian, a.unit.interval().div(&m.powi(2).unwrap()).unwrap()),
                    "gradient_x" => (Op::GradX, a.unit.interval().div(&m).unwrap()),
                    "gradient_z" => (Op::GradZ, a.unit.interval().div(&m).unwrap()),
                    _ => {
                        let r = arg_f64(node, "radius")? as usize;
                        if r == 0 || r > self.env.budgets.max_radius {
                            return Err(err(
                                "E_BUDGET_RADIUS",
                                format!("neighborhood radius {r} must be in 1..={}", self.env.budgets.max_radius),
                            ));
                        }
                        if op == "neighbor_sum" {
                            if a.unit.absolute {
                                return Err(err("E_UNIT", "cannot sum absolute temperatures"));
                            }
                            (Op::NeighborSum(r), a.unit)
                        } else {
                            (Op::NeighborMean(r), a.unit)
                        }
                    }
                };
                let q = if matches!(o, Op::NeighborSum(_) | Op::NeighborMean(_)) {
                    a.quantity.clone()
                } else {
                    None
                };
                self.emit(o, vec![ins[0].0], n_type(unit, q, a.dom, a.stage), key)
            }
            "region_mean" | "region_sum" => {
                let ins = self.input_list(ri, node, Some(1))?;
                let a = &ins[0].1;
                self.num(a, "operand")?;
                if !matches!(a.dom, Dom::Cells(_)) {
                    return Err(err("E_DOMAIN", format!("{op} requires a cells input")));
                }
                let name = arg_str(node, "region")?;
                let Some(r) = self.schema.region(&name) else {
                    return Err(err("E_UNKNOWN_REGION", format!("unknown region {name}")));
                };
                if op == "region_sum" && a.unit.absolute {
                    return Err(err("E_UNIT", "cannot sum absolute temperatures"));
                }
                let o = if op == "region_mean" { Op::RegionMean(r) } else { Op::RegionSum(r) };
                self.emit(o, vec![ins[0].0], n_type(a.unit, a.quantity.clone(), Dom::Uniform, a.stage), key)
            }
            "edge_from" | "edge_to" => {
                if !matches!(rule_dom, Dom::Edges(_)) {
                    return Err(err("E_DOMAIN", format!("{op} is only valid in an edges rule")));
                }
                let ins = self.input_list(ri, node, Some(1))?;
                let a = &ins[0].1;
                let Dom::Cells(g) = a.dom else {
                    return Err(err("E_DOMAIN", format!("{op} requires a cells input")));
                };
                let mut t = a.clone();
                t.dom = Dom::Edges(g);
                let o = if op == "edge_from" { Op::EdgeFrom } else { Op::EdgeTo };
                self.emit(o, vec![ins[0].0], t, key)
            }
            "sample" | "sample_offset" => {
                let Some(a) = arch else {
                    return Err(err("E_DOMAIN", format!("{op} is only valid in an entities rule")));
                };
                let ins = self.input_list(ri, node, Some(1))?;
                let x = &ins[0].1;
                if !matches!(x.dom, Dom::Cells(0)) {
                    return Err(err("E_DOMAIN", format!("{op} requires a cells input on the surface grid")));
                }
                let mut t = x.clone();
                t.dom = Dom::Entities(a);
                let o = if op == "sample" {
                    Op::Sample
                } else {
                    let forward = arg_f64(node, "forward")?;
                    let lateral = arg_f64(node, "lateral").unwrap_or(0.0);
                    let reach = (forward.abs().max(lateral.abs()) / self.env.cell_size).ceil() as usize;
                    if reach > self.env.budgets.max_radius {
                        return Err(err(
                            "E_BUDGET_RADIUS",
                            format!(
                                "sample_offset reach {reach} cells exceeds radius limit {}",
                                self.env.budgets.max_radius
                            ),
                        ));
                    }
                    Op::SampleOffset { forward, lateral }
                };
                self.emit(o, vec![ins[0].0], t, key)
            }
            "crowding" => {
                let Some(a) = arch else {
                    return Err(err("E_DOMAIN", "crowding is only valid in an entities rule"));
                };
                let r = arg_f64(node, "radius")? as usize;
                if r > self.env.budgets.max_radius {
                    return Err(err(
                        "E_BUDGET_RADIUS",
                        format!("crowding radius {r} exceeds limit {}", self.env.budgets.max_radius),
                    ));
                }
                self.emit(
                    Op::Crowding(r),
                    vec![],
                    n_type(DIMENSIONLESS, None, Dom::Entities(a), Stage::Snapshot),
                    key,
                )
            }
            "random" => {
                let stream = crate::rng::hash_str(key);
                self.emit(
                    Op::Random(stream),
                    vec![],
                    n_type(DIMENSIONLESS, None, rule_dom, Stage::Snapshot),
                    key,
                )
            }
            "brain" => {
                let Some(a) = arch else {
                    return Err(err("E_DOMAIN", "brain is only valid in an entities rule"));
                };
                let bd = self.schema.archetypes[a as usize].brain.clone();
                let ins = self.input_list(ri, node, Some(bd.inputs))?;
                for (i, (_, t)) in ins.iter().enumerate() {
                    if t.kind != Kind::Number || !t.unit.is_dimensionless() {
                        return Err(err(
                            "E_UNIT",
                            format!("brain input {i} must be a dimensionless normalized number, got {}", t.unit),
                        ));
                    }
                    self.unify_dom(t.dom, Dom::Entities(a))?;
                    if t.stage != Stage::Snapshot {
                        return Err(err("E_STAGE", format!("brain input {i} must be a snapshot observation")));
                    }
                }
                if self.instrs.iter().any(|x| x.op == Op::Brain(a)) {
                    return Err(err("E_BRAIN", "an archetype has exactly one brain node"));
                }
                let mut outs = vec![];
                for k in 0..bd.outputs {
                    outs.push(self.new_slot(
                        format!("{key}.out{k}"),
                        n_type(DIMENSIONLESS, None, Dom::Entities(a), Stage::Snapshot),
                    ));
                }
                self.instrs.push(Instr {
                    op: Op::Brain(a),
                    inputs: ins.iter().map(|x| x.0).collect(),
                    outputs: outs.clone(),
                    dom: Dom::Entities(a),
                    stage: Stage::Snapshot,
                    source: key.to_string(),
                });
                return Ok(outs);
            }
            _ => return self.lower_effect(ri, node, key),
        };
        Ok(vec![slot])
    }

    fn param_quantity(&self, ri: usize, name: &str) -> Option<String> {
        let (r, rc) = &self.rules[ri];
        if let Some(d) = r.parameters.get(name) {
            return d.quantity.clone();
        }
        self.packages
            .iter()
            .find(|p| p.package_id == rc.package)
            .and_then(|p| p.parameters.get(name))
            .and_then(|d| d.quantity.clone())
    }

    fn unit_arg(&self, node: &Node, key: &str) -> R<Unit> {
        let s = arg_str(node, key)?;
        Unit::parse(&s).map_err(|e| err("E_UNIT", e))
    }

    // ---- effects ------------------------------------------------------------------

    fn endpoint(&self, ri: usize, s: &str, resource: usize, as_source: bool) -> R<Endpoint> {
        let dom = self.rules[ri].1.dom;
        let res_id = &self.schema.resources[resource].id;
        if s.starts_with("external.") {
            let Some(a) = self.schema.account(s) else {
                return Err(err(
                    "E_UNDECLARED_EXTERNAL",
                    format!("undeclared external account {s}; external creation or removal must be declared"),
                ));
            };
            let acc = &self.schema.accounts[a];
            if acc.resource != resource {
                return Err(err(
                    "E_QUANTITY",
                    format!("account {s} exchanges {}, not {res_id}", self.schema.resources[acc.resource].id),
                ));
            }
            if as_source && !acc.can_source {
                return Err(err("E_UNDECLARED_EXTERNAL", format!("account {s} is not declared as a source")));
            }
            if !as_source && !acc.can_sink {
                return Err(err("E_UNDECLARED_EXTERNAL", format!("account {s} is not declared as a sink")));
            }
            return Ok(Endpoint::External(a));
        }
        let check_cell = |name: &str| -> R<usize> {
            let Some(fi) = self.schema.cell_field(name) else {
                return Err(err("E_UNKNOWN_FIELD", format!("unknown cell field {name}")));
            };
            match self.schema.cell_fields[fi].policy {
                FieldPolicy::Reservoir { resource: r, .. } if r == resource => Ok(fi),
                FieldPolicy::Reservoir { resource: r, .. } => Err(err(
                    "E_QUANTITY",
                    format!("field {name} holds {}, not {res_id}", self.schema.resources[r].id),
                )),
                _ => Err(err(
                    "E_WRITER",
                    format!("field {name} is not a conserved reservoir; transfers only move reservoir contents"),
                )),
            }
        };
        match dom {
            Dom::Cells(_) => Ok(Endpoint::Cell(check_cell(s.strip_prefix("cell.").unwrap_or(s))?)),
            Dom::Edges(_) => {
                if let Some(f) = s.strip_suffix("@from") {
                    Ok(Endpoint::EdgeA(check_cell(f)?))
                } else if let Some(f) = s.strip_suffix("@to") {
                    Ok(Endpoint::EdgeB(check_cell(f)?))
                } else {
                    Err(err(
                        "E_ENDPOINT",
                        format!("edge endpoints must be <field>@from or <field>@to, got {s}"),
                    ))
                }
            }
            Dom::Entities(a) => {
                if let Some(f) = s.strip_prefix("self.") {
                    let ai = &self.schema.archetypes[a as usize];
                    let Some(fi) = ai.field(f) else {
                        return Err(err("E_UNKNOWN_FIELD", format!("archetype {} has no field {f}", ai.id)));
                    };
                    match ai.fields[fi].policy {
                        FieldPolicy::Reservoir { resource: r, .. } if r == resource => Ok(Endpoint::Entity(a, fi)),
                        _ => Err(err("E_QUANTITY", format!("self.{f} is not a {res_id} reservoir"))),
                    }
                } else if let Some(f) = s.strip_prefix("cell.") {
                    Ok(Endpoint::Cell(check_cell(f)?))
                } else {
                    Err(err(
                        "E_ENDPOINT",
                        format!("entity endpoints must be self.<field>, cell.<field>, or external.<account>; got {s}"),
                    ))
                }
            }
            Dom::Uniform => unreachable!(),
        }
    }

    fn amount(&mut self, ri: usize, node: &Node, obj: &serde_json::Map<String, Value>, resource: usize, ctx: &str) -> R<(Slot, bool)> {
        let (reference, is_rate) = match (obj.get("rate"), obj.get("amount")) {
            (Some(Value::String(s)), None) => (s.clone(), true),
            (None, Some(Value::String(s))) => (s.clone(), false),
            _ => {
                return Err(err("E_ARGS", format!("{ctx} requires exactly one of 'rate' or 'amount'")));
            }
        };
        let port = if is_rate { "rate" } else { "amount" };
        let (slot, t) = self.input(ri, &reference, port)?;
        let rid = self.rules[ri].1.id.clone();
        let r = &self.schema.resources[resource];
        let expect = if is_rate {
            r.unit.div(&Unit::new(0, 0, 1, 0)).unwrap()
        } else {
            r.unit
        };
        let an = self.schema.arch_names();
        if t.kind != Kind::Number || t.unit != expect {
            return Err(err(
                "E_UNIT",
                format!(
                    "{ctx} {port} must be {expect} ({}), got {}",
                    r.id,
                    t.describe(&self.schema.grids, &an)
                ),
            )
            .at(&rid, Some(&node.id), Some(port)));
        }
        if let Some(q) = &t.quantity
            && *q != r.id
        {
            return Err(err(
                "E_QUANTITY",
                format!(
                    "{ctx} moves {} but its {port} is a {q} quantity; a {q} amount cannot silently become {}",
                    r.id, r.id
                ),
            )
            .at(&rid, Some(&node.id), Some(port)));
        }
        if t.stage != Stage::Snapshot {
            return Err(err(
                "E_RECEIPT_FEEDBACK",
                format!("{ctx} {port} depends on an accepted receipt; receipt-dependent values cannot submit new resource demands in the same tick"),
            )
            .at(&rid, Some(&node.id), Some(port)));
        }
        self.unify_dom(t.dom, self.rules[ri].1.dom)
            .map_err(|d| d.at(&rid, Some(&node.id), Some(port)))?;
        if let Some(&v) = self.const_values.get(&slot)
            && v < 0.0
        {
            return Err(err(
                "E_NEGATIVE_REQUEST",
                format!("{ctx} requests a negative amount ({v}); resource requests must be nonnegative"),
            )
            .at(&rid, Some(&node.id), Some(port)));
        }
        Ok((slot, is_rate))
    }

    fn leg(&mut self, ri: usize, node: &Node, obj: &serde_json::Map<String, Value>, ctx: &str) -> R<Leg> {
        let res_name = obj
            .get("resource")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err("E_ARGS", format!("{ctx} requires 'resource'")))?;
        let resource = self
            .schema
            .resource(res_name)
            .ok_or_else(|| err("E_RESOURCE", format!("unknown resource {res_name}")))?;
        let from = obj
            .get("from")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err("E_ARGS", format!("{ctx} requires 'from'")))?;
        let to = obj
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err("E_ARGS", format!("{ctx} requires 'to'")))?;
        let from = self.endpoint(ri, from, resource, true)?;
        let to = self.endpoint(ri, to, resource, false)?;
        if from.is_external() && to.is_external() {
            return Err(err("E_ENDPOINT", format!("{ctx} moves between two external accounts")));
        }
        let (amount, is_rate) = self.amount(ri, node, obj, resource, ctx)?;
        Ok(Leg {
            resource,
            from,
            to,
            amount,
            is_rate,
        })
    }

    fn state_target(&self, ri: usize, field: &str, want: FieldPolicy) -> R<(Option<u16>, usize, Unit)> {
        let rc = &self.rules[ri].1;
        if let Some(f) = field.strip_prefix("self.") {
            let Some(a) = rc.arch else {
                return Err(err("E_DOMAIN", "self.<field> is only valid in an entities rule"));
            };
            let ai = &self.schema.archetypes[a as usize];
            let fi = ai
                .field(f)
                .ok_or_else(|| err("E_UNKNOWN_FIELD", format!("archetype {} has no field {f}", ai.id)))?;
            if ai.fields[fi].policy != want {
                return Err(err(
                    "E_WRITER",
                    format!(
                        "self.{f} has update policy {:?}; this effect requires {want:?}",
                        ai.fields[fi].policy
                    ),
                ));
            }
            Ok((Some(a), fi, ai.fields[fi].unit))
        } else {
            if !matches!(rc.dom, Dom::Cells(_)) {
                return Err(err("E_DOMAIN", "cell state effects must come from a cells rule"));
            }
            let fi = self
                .schema
                .cell_field(field)
                .ok_or_else(|| err("E_UNKNOWN_FIELD", format!("unknown field {field}")))?;
            let f = &self.schema.cell_fields[fi];
            if f.policy != want {
                let hint = if matches!(f.policy, FieldPolicy::Reservoir { .. }) {
                    "; a rule cannot write an arbitrary delta to a conserved reservoir, use a transfer"
                } else {
                    ""
                };
                return Err(err(
                    "E_WRITER",
                    format!(
                        "field {field} has update policy {:?}; this effect requires {want:?}{hint}",
                        f.policy
                    ),
                ));
            }
            Ok((None, fi, f.unit))
        }
    }

    fn lower_effect(&mut self, ri: usize, node: &Node, key: &str) -> R<Vec<Slot>> {
        let rc_dom = self.rules[ri].1.dom;
        let rule_id = self.rules[ri].1.id.clone();
        let op = node.op.as_str();
        let obj: serde_json::Map<String, Value> = node.args.clone().into_iter().collect();
        let check_dom = |c: &Self, t: &Type| c.unify_dom(t.dom, rc_dom);
        let kind = match op {
            "transfer" => EffectKind::Process {
                legs: vec![self.leg(ri, node, &obj, "transfer")?],
            },
            "external_source" | "external_sink" => {
                let mut o = obj.clone();
                let acc = obj
                    .get("account")
                    .cloned()
                    .ok_or_else(|| err("E_ARGS", format!("{op} requires 'account'")))?;
                if op == "external_source" {
                    o.insert("from".into(), acc);
                } else {
                    o.insert("to".into(), acc);
                }
                let leg = self.leg(ri, node, &o, op)?;
                let ext_ok = if op == "external_source" {
                    leg.from.is_external()
                } else {
                    leg.to.is_external()
                };
                if !ext_ok {
                    return Err(err("E_UNDECLARED_EXTERNAL", format!("{op} account must be an external account")));
                }
                EffectKind::Process { legs: vec![leg] }
            }
            "reaction" => {
                let legs_v = match obj.get("legs") {
                    Some(Value::Array(a)) if !a.is_empty() && a.len() <= 8 => a.clone(),
                    _ => return Err(err("E_ARGS", "reaction requires 1..=8 'legs'")),
                };
                let mut legs = vec![];
                for (i, l) in legs_v.iter().enumerate() {
                    let Value::Object(m) = l else {
                        return Err(err("E_ARGS", "reaction legs must be objects"));
                    };
                    legs.push(self.leg(ri, node, m, &format!("reaction leg {i}"))?);
                }
                EffectKind::Process { legs }
            }
            "edge_transfer" => {
                if !matches!(rc_dom, Dom::Edges(_)) {
                    return Err(err("E_DOMAIN", "edge_transfer is only valid in an edges rule"));
                }
                let res = arg_str(node, "resource")?;
                let resource = self
                    .schema
                    .resource(&res)
                    .ok_or_else(|| err("E_RESOURCE", format!("unknown resource {res}")))?;
                let field = arg_str(node, "field")?;
                let fi = match self.endpoint(ri, &format!("{field}@from"), resource, true)? {
                    Endpoint::EdgeA(f) => f,
                    _ => unreachable!(),
                };
                let (rate, is_rate) = self.amount(ri, node, &obj, resource, "edge_transfer")?;
                if !is_rate {
                    return Err(err("E_ARGS", "edge_transfer takes a signed 'rate'"));
                }
                EffectKind::EdgeTransfer { resource, field: fi, rate }
            }
            "rate_contribution" => {
                let field = arg_str(node, "field")?;
                let (arch, fi, unit) = self.state_target(ri, &field, FieldPolicy::Integrated)?;
                let (rate, t) = self.named_input(ri, node, "rate")?;
                let expect = unit.interval().div(&Unit::new(0, 0, 1, 0)).unwrap();
                if t.unit != expect || t.kind != Kind::Number {
                    return Err(err("E_UNIT", format!("rate for {field} must be {expect}, got {}", t.unit)).at(
                        &rule_id,
                        Some(&node.id),
                        Some("rate"),
                    ));
                }
                if t.stage > Stage::Receipt {
                    return Err(err("E_STAGE", "rate contributions cannot read candidate state"));
                }
                check_dom(self, &t)?;
                match arch {
                    Some(a) => EffectKind::RateEntity { arch: a, field: fi, rate },
                    None => EffectKind::RateCell { field: fi, rate },
                }
            }
            "next_value" => {
                let field = arg_str(node, "field")?;
                let (arch, fi, unit) = self.state_target(ri, &field, FieldPolicy::NextValue)?;
                let (value, t) = self.named_input(ri, node, "value")?;
                if t.unit != unit {
                    return Err(err("E_UNIT", format!("next value for {field} must be {unit}, got {}", t.unit)));
                }
                if t.stage > Stage::Receipt {
                    return Err(err("E_STAGE", "next values cannot read candidate state"));
                }
                check_dom(self, &t)?;
                match arch {
                    Some(a) => EffectKind::NextEntity { arch: a, field: fi, value },
                    None => EffectKind::NextCell { field: fi, value },
                }
            }
            "move" => {
                let Dom::Entities(a) = rc_dom else {
                    return Err(err("E_DOMAIN", "move is only valid in an entities rule"));
                };
                let (turn, tt) = self.named_input(ri, node, "turn")?;
                let (speed, st) = self.named_input(ri, node, "speed")?;
                if tt.unit != Unit::new(0, 0, -1, 0) {
                    return Err(err("E_UNIT", format!("move turn must be rad/s (1/s), got {}", tt.unit)));
                }
                if st.unit != Unit::new(0, 1, -1, 0) {
                    return Err(err("E_UNIT", format!("move speed must be m/s, got {}", st.unit)));
                }
                if tt.stage > Stage::Receipt || st.stage > Stage::Receipt {
                    return Err(err("E_STAGE", "movement cannot read candidate state"));
                }
                check_dom(self, &tt)?;
                check_dom(self, &st)?;
                if self
                    .effects
                    .iter()
                    .any(|e| matches!(e.kind, EffectKind::Move { arch, .. } if arch == a))
                {
                    return Err(err("E_WRITER", "an archetype has exactly one move effect"));
                }
                EffectKind::Move { arch: a, turn, speed }
            }
            "death" => {
                let Dom::Entities(a) = rc_dom else {
                    return Err(err("E_DOMAIN", "death is only valid in an entities rule"));
                };
                let (cond, t) = self.named_input(ri, node, "condition")?;
                if t.kind != Kind::Bool {
                    return Err(err("E_TYPE", "death condition must be bool"));
                }
                check_dom(self, &t)?;
                let reason = arg_opt_str(node, "reason").unwrap_or_else(|| node.id.clone());
                EffectKind::Death {
                    arch: a,
                    condition: cond,
                    reason,
                }
            }
            "birth" => {
                let Dom::Entities(a) = rc_dom else {
                    return Err(err("E_DOMAIN", "birth is only valid in an entities rule"));
                };
                let (cond, t) = self.named_input(ri, node, "condition")?;
                if t.kind != Kind::Bool {
                    return Err(err("E_TYPE", "birth condition must be bool"));
                }
                check_dom(self, &t)?;
                let legs_v = match obj.get("legs") {
                    Some(Value::Array(v)) => v.clone(),
                    _ => return Err(err("E_ARGS", "birth requires 'legs': [{field, amount}]")),
                };
                let mut legs = vec![];
                for l in &legs_v {
                    let f = l
                        .get("field")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| err("E_ARGS", "birth leg needs 'field'"))?;
                    let amt = l
                        .get("amount")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| err("E_ARGS", "birth leg needs 'amount'"))?;
                    let ai = &self.schema.archetypes[a as usize];
                    let fi = ai.field(f).ok_or_else(|| err("E_UNKNOWN_FIELD", format!("unknown field {f}")))?;
                    let FieldPolicy::Reservoir { .. } = ai.fields[fi].policy else {
                        return Err(err("E_WRITER", format!("birth can only transfer reservoirs, {f} is not one")));
                    };
                    let unit = ai.fields[fi].unit;
                    let (slot, at) = self.input(ri, amt, "amount")?;
                    if at.unit != unit {
                        return Err(err("E_UNIT", format!("birth transfer of {f} must be {unit}, got {}", at.unit)));
                    }
                    check_dom(self, &at)?;
                    legs.push((fi, slot));
                }
                if self
                    .effects
                    .iter()
                    .any(|e| matches!(e.kind, EffectKind::Birth { arch, .. } if arch == a))
                {
                    return Err(err("E_WRITER", "an archetype has exactly one birth effect"));
                }
                EffectKind::Birth {
                    arch: a,
                    condition: cond,
                    legs,
                }
            }
            _ => return Err(err("E_UNKNOWN_OP", format!("unknown op '{op}'"))),
        };
        let coverage = self.scope_coverage(ri, &kind)?;
        self.effects.push(Effect {
            id: key.to_string(),
            rule: rule_id,
            dom: rc_dom,
            kind,
            coverage,
        });
        let ei = self.effects.len() - 1;
        self.effect_of.insert(key.to_string(), ei);
        // Effects have no value output; a placeholder keeps the resolver uniform.
        Ok(vec![usize::MAX])
    }

    fn scope_coverage(&mut self, ri: usize, kind: &EffectKind) -> R<Option<Slot>> {
        let rule = self.rules[ri].0;
        let Some(scope) = &rule.scope else {
            return Ok(None);
        };
        if scope.region.is_none() && scope.predicate.is_none() {
            return Ok(None);
        }
        if !matches!(
            kind,
            EffectKind::Process { .. } | EffectKind::EdgeTransfer { .. } | EffectKind::RateCell { .. } | EffectKind::RateEntity { .. }
        ) {
            return Err(err(
                "E_SCOPE",
                "scoped rules may only contain transfers, reactions, and rate contributions",
            ));
        }
        let dom = self.rules[ri].1.dom;
        let src = format!("{}/$scope", rule.rule_id);
        if let Some(&s) = self.source_map.get(&src).and_then(|v| v.first()) {
            return Ok(Some(s));
        }
        let unit_t = |dom| Type {
            kind: Kind::Number,
            unit: DIMENSIONLESS,
            quantity: None,
            dom,
            stage: Stage::Snapshot,
        };
        let mut cov: Option<Slot> = None;
        if let Some(region) = &scope.region {
            let r = self
                .schema
                .region(region)
                .ok_or_else(|| err("E_UNKNOWN_REGION", format!("scope region {region} does not exist")))?;
            let cells = self.emit(Op::Region(r), vec![], unit_t(Dom::Cells(0)), &src);
            cov = Some(match dom {
                Dom::Cells(_) => cells,
                Dom::Edges(g) => {
                    let a = self.emit(Op::EdgeFrom, vec![cells], unit_t(Dom::Edges(g)), &src);
                    let b = self.emit(Op::EdgeTo, vec![cells], unit_t(Dom::Edges(g)), &src);
                    let s = self.emit(Op::Add, vec![a, b], unit_t(Dom::Edges(g)), &src);
                    let h = self.emit(Op::Const(0.5), vec![], unit_t(Dom::Uniform), &src);
                    self.emit(Op::Mul, vec![s, h], unit_t(Dom::Edges(g)), &src)
                }
                Dom::Entities(a) => self.emit(Op::Sample, vec![cells], unit_t(Dom::Entities(a)), &src),
                Dom::Uniform => unreachable!(),
            });
        }
        if let Some(p) = &scope.predicate {
            let (ps, pt) = self.input(ri, p, "scope.predicate")?;
            if pt.kind != Kind::Bool || pt.stage != Stage::Snapshot {
                return Err(err("E_SCOPE", "scope predicate must be a snapshot bool"));
            }
            self.unify_dom(pt.dom, dom)?;
            let pd = if pt.dom == Dom::Uniform { Dom::Uniform } else { dom };
            let as_num = self.emit(Op::Relabel, vec![ps], unit_t(pd), &src);
            cov = Some(match cov {
                Some(c) => self.emit(Op::Mul, vec![c, as_num], unit_t(dom), &src),
                None => as_num,
            });
        }
        self.source_map.insert(src, vec![cov.unwrap()]);
        Ok(cov)
    }

    fn entity_capacities(&mut self) {
        let pk = self.packages.clone();
        for p in &pk {
            for a in &p.archetypes {
                let Some(ai) = self.schema.archetype(&a.id) else {
                    continue;
                };
                for (fi, f) in a.fields.iter().enumerate() {
                    let Policy::Reservoir { capacity: Some(cap), .. } = &f.policy else {
                        continue;
                    };
                    if !self.nodes.contains_key(cap) {
                        self.diags.push(err(
                            "E_CAPACITY",
                            format!("capacity of {}.{} must name a node as rule_id/node_id, got {cap}", a.id, f.id),
                        ));
                        continue;
                    }
                    let res = self.resolve(cap);
                    match res {
                        Ok(outs) if outs.len() == 1 && outs[0] != usize::MAX => {
                            let t = &self.types[outs[0]];
                            let ok = t.unit == self.schema.archetypes[ai].fields[fi].unit
                                && matches!(t.dom, Dom::Entities(x) if x as usize == ai)
                                && t.stage == Stage::Snapshot;
                            if ok {
                                self.schema.archetypes[ai].capacity_slots[fi] = Some(outs[0]);
                            } else {
                                self.diags.push(err(
                                    "E_CAPACITY",
                                    format!(
                                        "capacity {cap} of {}.{} must be a snapshot {} value over its archetype",
                                        a.id, f.id, self.schema.archetypes[ai].fields[fi].unit
                                    ),
                                ));
                            }
                        }
                        Ok(_) => self.diags.push(err("E_CAPACITY", format!("capacity {cap} must be a value node"))),
                        Err(d) => self.diags.push(d),
                    }
                }
            }
        }
    }

    fn check_rule_effect_lists(&mut self) {
        let rules: Vec<(String, Vec<String>)> = self.rules.iter().map(|(r, rc)| (rc.id.clone(), r.effects.clone())).collect();
        for (rid, listed) in rules {
            if listed.is_empty() {
                continue;
            }
            let listed: BTreeSet<String> = listed.iter().map(|e| e.split('.').next().unwrap().to_string()).collect();
            let actual: BTreeSet<String> = self
                .effects
                .iter()
                .filter(|e| e.rule == rid)
                .map(|e| e.id.split_once('/').unwrap().1.to_string())
                .collect();
            for m in listed.difference(&actual) {
                self.diags
                    .push(err("E_EFFECTS", format!("listed effect {m} is not an effect node")).at(&rid, None, None));
            }
            for m in actual.difference(&listed) {
                self.diags
                    .push(err("E_EFFECTS", format!("effect node {m} is missing from the rule's effects list")).at(&rid, None, None));
            }
        }
    }

    fn check_writers(&mut self) {
        let mut next_writers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for e in &self.effects {
            let target = match &e.kind {
                EffectKind::NextCell { field, .. } => Some(self.schema.cell_fields[*field].id.clone()),
                EffectKind::NextEntity { arch, field, .. } => {
                    let a = &self.schema.archetypes[*arch as usize];
                    Some(format!("{}.{}", a.id, a.fields[*field].id))
                }
                _ => None,
            };
            if let Some(t) = target {
                next_writers.entry(t).or_default().push(e.id.clone());
            }
        }
        for (f, ws) in next_writers {
            if ws.len() > 1 {
                self.diags.push(err(
                    "E_MULTIPLE_WRITERS",
                    format!(
                        "field {f} has {} exclusive next-value writers ({}); combine their values in the graph before a single writer",
                        ws.len(),
                        ws.join(", ")
                    ),
                ));
            }
        }
        // Unused pure nodes are allowed but reported.
        let used: BTreeSet<Slot> = self.instrs.iter().flat_map(|i| i.inputs.iter().copied()).collect();
        let effect_inputs: BTreeSet<Slot> = self.effects.iter().flat_map(effect_slots).collect();
        let cap_slots: BTreeSet<Slot> = self
            .schema
            .archetypes
            .iter()
            .flat_map(|a| a.capacity_slots.iter().flatten().copied())
            .collect();
        let mut unused = vec![];
        for i in &self.instrs {
            if matches!(i.op, Op::Receipt { .. }) || i.source.ends_with("$scope") {
                continue;
            }
            if i.outputs
                .iter()
                .all(|o| !used.contains(o) && !effect_inputs.contains(o) && !cap_slots.contains(o))
            {
                unused.push(i.source.clone());
            }
        }
        for u in unused {
            let (r, n) = u.split_once('/').unwrap_or(("", &u));
            let (r, n) = (r.to_string(), n.to_string());
            self.warn("W_UNUSED", format!("node {u} does not reach any effect"), Some(&r), Some(&n));
        }
    }

    /// Effects run in canonical (qualified id) order, never registration order.
    fn canonicalize_effects(&mut self) {
        let mut order: Vec<usize> = (0..self.effects.len()).collect();
        order.sort_by(|&a, &b| self.effects[a].id.cmp(&self.effects[b].id));
        let mut remap = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old] = new;
        }
        let old = std::mem::take(&mut self.effects);
        let mut slots: Vec<Option<Effect>> = old.into_iter().map(Some).collect();
        self.effects = order.iter().map(|&o| slots[o].take().unwrap()).collect();
        for i in &mut self.instrs {
            if let Op::Receipt { effect, .. } = &mut i.op {
                *effect = remap[*effect];
            }
        }
    }

    /// Parameter table in canonical (qualified name) order.
    fn canonicalize_params(&mut self) {
        let mut order: Vec<usize> = (0..self.params.len()).collect();
        order.sort_by(|&a, &b| self.params[a].name.cmp(&self.params[b].name));
        let mut remap = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old] = new;
        }
        let old = std::mem::take(&mut self.params);
        self.params = order.iter().map(|&o| old[o].clone()).collect();
        for p in &mut self.params {
            if let Some(d) = &mut p.density_param {
                *d = remap[*d];
            }
        }
        for i in &mut self.instrs {
            if let Op::Param(p) = &mut i.op {
                *p = remap[*p];
            }
        }
    }

    fn estimate_cost(&mut self) {
        let card = |d: Dom, env: &CompileEnv| -> u64 {
            match d {
                Dom::Uniform => 1,
                Dom::Cells(_) => env.cells(),
                Dom::Edges(_) => env.edges(),
                Dom::Entities(_) => env.population_cap as u64,
            }
        };
        let mut ops = 0u64;
        let mut bytes = 0u64;
        for i in &self.instrs {
            let n = card(i.dom, self.env);
            let w = match &i.op {
                Op::Laplacian | Op::GradX | Op::GradZ => 5,
                Op::NeighborSum(r) | Op::NeighborMean(r) => ((2 * r + 1) * (2 * r + 1)) as u64,
                Op::Crowding(r) => ((2 * r + 1) * (2 * r + 1)) as u64 * 4,
                Op::Brain(a) => self.schema.archetypes[*a as usize].brain_params() as u64,
                Op::Curve(p) => (p.len() as u64).max(2),
                Op::RegionMean(_) | Op::RegionSum(_) => 2,
                _ => 1,
            };
            ops = ops.saturating_add(n.saturating_mul(w));
            bytes = bytes.saturating_add(n.saturating_mul(8).saturating_mul(i.outputs.len() as u64));
        }
        let mut effect_elems = 0u64;
        for e in &self.effects {
            let legs = match &e.kind {
                EffectKind::Process { legs } => legs.len() as u64,
                _ => 1,
            };
            effect_elems = effect_elems.saturating_add(card(e.dom, self.env).saturating_mul(legs));
        }
        ops = ops.saturating_add(effect_elems.saturating_mul(6));
        let cost = CostEstimate {
            element_ops: ops,
            transient_bytes: bytes,
            effect_elements: effect_elems,
            nodes: self.instrs.len(),
        };
        if ops > self.env.budgets.max_element_ops {
            self.diags.push(err(
                "E_BUDGET_OPS",
                format!(
                    "estimated {ops} element operations per tick exceeds budget {}",
                    self.env.budgets.max_element_ops
                ),
            ));
        }
        if bytes > self.env.budgets.max_transient_bytes {
            self.diags.push(err(
                "E_BUDGET_MEMORY",
                format!(
                    "estimated {bytes} transient bytes exceeds budget {}",
                    self.env.budgets.max_transient_bytes
                ),
            ));
        }
        self.cost = Some(cost);
    }
}

pub fn effect_slots(e: &Effect) -> Vec<Slot> {
    let mut v: Vec<Slot> = match &e.kind {
        EffectKind::Process { legs } => legs.iter().map(|l| l.amount).collect(),
        EffectKind::EdgeTransfer { rate, .. } => vec![*rate],
        EffectKind::RateCell { rate, .. } | EffectKind::RateEntity { rate, .. } => vec![*rate],
        EffectKind::NextCell { value, .. } | EffectKind::NextEntity { value, .. } => vec![*value],
        EffectKind::Move { turn, speed, .. } => vec![*turn, *speed],
        EffectKind::Death { condition, .. } => vec![*condition],
        EffectKind::Birth { condition, legs, .. } => std::iter::once(*condition).chain(legs.iter().map(|l| l.1)).collect(),
    };
    if let Some(c) = e.coverage {
        v.push(c);
    }
    v
}

/// A ratio keeps the numerator's quantity only when the denominator is untagged.
fn div_quantity(a: &Option<String>, b: &Option<String>) -> Option<String> {
    match (a, b) {
        (Some(x), None) => Some(x.clone()),
        _ => None,
    }
}

fn mul_quantity(a: &Option<String>, b: &Option<String>) -> Option<String> {
    match (a, b) {
        (Some(x), None) | (None, Some(x)) => Some(x.clone()),
        _ => None,
    }
}

pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_ID_LEN && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
}

fn arg_str(node: &Node, key: &str) -> R<String> {
    node.args
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| err("E_ARGS", format!("op {} requires string argument '{key}'", node.op)))
}

fn arg_opt_str(node: &Node, key: &str) -> Option<String> {
    node.args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn arg_f64(node: &Node, key: &str) -> R<f64> {
    node.args
        .get(key)
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && v.abs() < 1e300)
        .ok_or_else(|| err("E_ARGS", format!("op {} requires finite numeric argument '{key}'", node.op)))
}
