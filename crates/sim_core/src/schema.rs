//! Versioned, serializable documents: rule packages and scenarios.
//!
//! These are the canonical authoring formats. They are untrusted data; the
//! compiler validates everything before it can affect a world.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PACKAGE_SCHEMA_VERSION: u32 = 1;
pub const SCENARIO_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub schema_version: u32,
    pub package_id: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub requires: Vec<String>,
    /// Engine capabilities the package needs, e.g. `sandbox.external_source`.
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub resources: Vec<ResourceDecl>,
    #[serde(default)]
    pub accounts: Vec<AccountDecl>,
    #[serde(default)]
    pub fields: Vec<FieldDecl>,
    #[serde(default)]
    pub forcings: Vec<ForcingDecl>,
    #[serde(default)]
    pub archetypes: Vec<ArchetypeDecl>,
    #[serde(default)]
    pub parameters: BTreeMap<String, ParamDecl>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

fn default_version() -> String {
    "1.0.0".into()
}
fn default_grid() -> String {
    "world.surface".into()
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceDecl {
    pub id: String,
    pub unit: String,
    /// Per-tick ledger tolerance: abs + rel * max(total, exchange).
    #[serde(default = "default_tol_abs")]
    pub tolerance_abs: f64,
    #[serde(default = "default_tol_rel")]
    pub tolerance_rel: f64,
}
fn default_tol_abs() -> f64 {
    1e-9
}
fn default_tol_rel() -> f64 {
    1e-12
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AccountDecl {
    pub id: String,
    pub resource: String,
    /// `source`, `sink`, or `both`.
    pub direction: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FieldDecl {
    pub id: String,
    #[serde(default = "default_grid")]
    pub grid: String,
    pub unit: String,
    #[serde(default)]
    pub quantity: Option<String>,
    pub policy: Policy,
    #[serde(default)]
    pub default: f64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Policy {
    /// Conserved inventory changed only by resolved transfers.
    Reservoir {
        resource: String,
        /// A parameter field (cells) or a node reference (entities) giving capacity.
        #[serde(default)]
        capacity: Option<String>,
    },
    /// Non-conserved state advanced by summed rate contributions.
    Integrated,
    /// Exactly one rule supplies the next value.
    NextValue,
    /// Immutable during ticks; editable at a tick boundary.
    Parameter,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ForcingDecl {
    pub id: String,
    pub unit: String,
    #[serde(default)]
    pub quantity: Option<String>,
    /// `uniform` or `cells`.
    #[serde(default = "default_forcing_domain")]
    pub domain: String,
    #[serde(default)]
    pub label: String,
}
fn default_forcing_domain() -> String {
    "uniform".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ArchetypeDecl {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub fields: Vec<FieldDecl>,
    pub traits: Vec<TraitDecl>,
    pub brain: BrainDecl,
    /// Hard engine bound on speed, m/s.
    pub max_speed: f64,
    /// Hard engine bound on turning, rad/s.
    pub max_turn_rate: f64,
    /// Every conserved entity reservoir must move somewhere on death.
    pub death_disposal: Vec<Disposal>,
    pub mutation: MutationDecl,
    /// Engine-owned: seconds after a birth before the parent may reproduce again.
    pub reproduction_cooldown: f64,
    /// Initial values for a newly seeded (not born) organism, by field id.
    #[serde(default)]
    pub seed_state: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TraitDecl {
    pub id: String,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    /// Range sampled for randomly initialized organisms.
    pub init_min: f64,
    pub init_max: f64,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrainDecl {
    /// Only `mlp.tanh.v1` exists initially.
    pub backend: String,
    pub inputs: usize,
    pub hidden: usize,
    pub outputs: usize,
    /// Absolute bound on every weight and bias.
    pub weight_bound: f64,
    /// Range for random initialization.
    pub init_scale: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Disposal {
    pub field: String,
    /// `cell.<field>` or an external account id.
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MutationDecl {
    /// Per-scalar probability.
    pub probability: f64,
    /// Perturbation magnitude as a fraction of each scalar's declared range.
    pub scale: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParamDecl {
    pub value: f64,
    pub unit: String,
    #[serde(default)]
    pub quantity: Option<String>,
    pub min: f64,
    pub max: f64,
    #[serde(default)]
    pub label: String,
    /// Optional numerical stability class checked against dt and grid spacing.
    #[serde(default)]
    pub stability: Option<Stability>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Stability {
    /// Explicit 2D diffusion: dt * D * (2/dx^2 + 2/dz^2) <= 1.
    Diffusion,
    /// Head-driven transport: dt * c * 4 / (density * area) <= limit.
    Transport { density_param: String, limit: f64 },
    /// Per-cell first-order withdrawal: dt * k <= 1.
    FirstOrder,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub rule_id: String,
    #[serde(default)]
    pub label: String,
    pub domain: DomainDecl,
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub parameters: BTreeMap<String, ParamDecl>,
    pub nodes: Vec<Node>,
    /// Effect roots. Every effect node must be listed.
    #[serde(default)]
    pub effects: Vec<String>,
    /// Template this rule was instantiated from, for the editor.
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DomainDecl {
    /// `cells`, `edges`, or `entities`.
    pub kind: String,
    #[serde(default)]
    pub grid: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Region mask id; coverage in [0,1] scales every effect of the rule.
    #[serde(default)]
    pub region: Option<String>,
    /// Boolean node reference evaluated on the snapshot.
    #[serde(default)]
    pub predicate: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: String,
    pub op: String,
    #[serde(flatten)]
    pub args: BTreeMap<String, Value>,
}

// ---------------------------------------------------------------------------
// Scenarios

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub schema_version: u32,
    pub scenario_id: String,
    #[serde(default)]
    pub label: String,
    pub seed: u64,
    pub grid: GridDecl,
    /// Package file names resolved against the rules directory, in any order.
    pub packages: Vec<String>,
    #[serde(default = "default_dt")]
    pub dt: f64,
    pub terrain: TerrainDecl,
    #[serde(default)]
    pub initial: BTreeMap<String, InitSpec>,
    pub weather: WeatherDecl,
    #[serde(default)]
    pub regions: Vec<RegionDecl>,
    #[serde(default)]
    pub populations: Vec<PopulationDecl>,
    #[serde(default = "default_cap")]
    pub population_cap: usize,
    #[serde(default)]
    pub budgets: Budgets,
    /// Guided-scenario steps shown by the client.
    #[serde(default)]
    pub guide: Vec<GuideStep>,
}

fn default_dt() -> f64 {
    0.25
}
fn default_cap() -> usize {
    10_000
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GridDecl {
    #[serde(default = "default_grid")]
    pub id: String,
    pub width: usize,
    pub height: usize,
    pub cell_size: f64,
    /// Only `closed` is supported; open outflow is expressed as a declared sink rule.
    #[serde(default = "default_boundary")]
    pub boundary: String,
    #[serde(default = "default_chunk")]
    pub chunk_size: usize,
}
fn default_boundary() -> String {
    "closed".into()
}
fn default_chunk() -> usize {
    32
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerrainDecl {
    Flat {
        elevation: f64,
    },
    /// Uplands at one end draining through a valley to a lower basin.
    Valley {
        relief: f64,
        valley_depth: f64,
        basin_depth: f64,
        roughness: f64,
    },
    /// Explicit values, row-major (z * width + x).
    Explicit {
        values: Vec<f64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InitSpec {
    Constant {
        value: f64,
    },
    /// a + b * normalized elevation (0 at lowest, 1 at highest).
    Elevation {
        a: f64,
        b: f64,
    },
    /// Seeded value noise in [lo, hi] with the given feature size in cells.
    Noise {
        lo: f64,
        hi: f64,
        feature: f64,
    },
    /// Water mass that fills terrain up to a level, in meters, capped per cell.
    FillToLevel {
        level: f64,
        density: f64,
        max_depth: f64,
    },
    /// Explicit values, row-major.
    Explicit {
        values: Vec<f64>,
    },
    /// Value inside a region, another outside.
    Region {
        region: String,
        inside: f64,
        outside: f64,
    },
    /// A multiple of another (non-derived) field's initial values.
    ScaleOf {
        field: String,
        factor: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WeatherDecl {
    /// Output forcing id -> prescribed signal.
    pub signals: BTreeMap<String, Signal>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Signal {
    pub mean: f64,
    #[serde(default)]
    pub amplitude: f64,
    /// Seconds.
    #[serde(default = "default_period")]
    pub period: f64,
    #[serde(default)]
    pub phase: f64,
    /// Seeded smooth noise amplitude, re-drawn every `noise_interval` seconds.
    #[serde(default)]
    pub noise: f64,
    #[serde(default = "default_noise_interval")]
    pub noise_interval: f64,
    /// For cell forcings: value += elevation_coefficient * normalized elevation * mean.
    #[serde(default)]
    pub elevation_coefficient: f64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}
fn default_period() -> f64 {
    1200.0
}
fn default_noise_interval() -> f64 {
    60.0
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RegionDecl {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub shape: RegionShape,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegionShape {
    /// Cell coordinates, inclusive min, exclusive max.
    Rect {
        x0: usize,
        z0: usize,
        x1: usize,
        z1: usize,
    },
    /// Soft-edged circle in cell units.
    Circle {
        cx: f64,
        cz: f64,
        radius: f64,
        feather: f64,
    },
    /// Cells on the outer ring of the world.
    Boundary,
    Explicit {
        values: Vec<f64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PopulationDecl {
    pub archetype: String,
    pub count: usize,
    /// `random` or `authored:<name>` (a labeled control fixture).
    #[serde(default = "default_genome")]
    pub genome: String,
    #[serde(default)]
    pub region: Option<String>,
    /// Trait overrides for controlled fixtures.
    #[serde(default)]
    pub traits: BTreeMap<String, f64>,
    /// Authored brains: (input, output, weight) links, each through its own hidden unit.
    #[serde(default)]
    pub authored_links: Vec<(usize, usize, f64)>,
    #[serde(default)]
    pub authored_bias: Vec<f64>,
}
fn default_genome() -> String {
    "random".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    #[serde(default = "d_nodes")]
    pub max_nodes: usize,
    #[serde(default = "d_fields")]
    pub max_cell_fields: usize,
    #[serde(default = "d_radius")]
    pub max_radius: usize,
    #[serde(default = "d_neighbors")]
    pub max_neighbors: usize,
    /// Estimated element-operations per tick.
    #[serde(default = "d_ops")]
    pub max_element_ops: u64,
    /// Estimated transient bytes for node buffers.
    #[serde(default = "d_mem")]
    pub max_transient_bytes: u64,
    #[serde(default = "d_rules")]
    pub max_rules: usize,
}
fn d_nodes() -> usize {
    4096
}
fn d_fields() -> usize {
    64
}
fn d_radius() -> usize {
    4
}
fn d_neighbors() -> usize {
    32
}
fn d_ops() -> u64 {
    2_000_000_000
}
fn d_mem() -> u64 {
    2 << 30
}
fn d_rules() -> usize {
    512
}
impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            max_nodes: d_nodes(),
            max_cell_fields: d_fields(),
            max_radius: d_radius(),
            max_neighbors: d_neighbors(),
            max_element_ops: d_ops(),
            max_transient_bytes: d_mem(),
            max_rules: d_rules(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GuideStep {
    pub title: String,
    pub body: String,
}
