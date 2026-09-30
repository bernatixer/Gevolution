//! Typed intermediate representation and compiled execution plan.

use crate::units::Unit;
use serde::Serialize;

pub type Slot = usize;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, PartialOrd, Ord)]
pub enum Dom {
    Uniform,
    Cells(u16),
    Edges(u16),
    Entities(u16),
}

impl Dom {
    pub fn describe(&self, grids: &[String], archetypes: &[String]) -> String {
        match self {
            Dom::Uniform => "uniform".into(),
            Dom::Cells(g) => format!("cells<{}>", grids.get(*g as usize).map(|s| s.as_str()).unwrap_or("?")),
            Dom::Edges(g) => format!("edges<{}>", grids.get(*g as usize).map(|s| s.as_str()).unwrap_or("?")),
            Dom::Entities(a) => format!("entities<{}>", archetypes.get(*a as usize).map(|s| s.as_str()).unwrap_or("?")),
        }
    }
}

/// Temporal stage: what a value may depend on within one tick.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize)]
pub enum Stage {
    Snapshot = 0,
    Receipt = 1,
    Candidate = 2,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize)]
pub enum Kind {
    Number,
    Bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Type {
    pub kind: Kind,
    pub unit: Unit,
    pub quantity: Option<String>,
    pub dom: Dom,
    pub stage: Stage,
}

impl Type {
    pub fn describe(&self, grids: &[String], archetypes: &[String]) -> String {
        let k = match self.kind {
            Kind::Number => "",
            Kind::Bool => "bool ",
        };
        let q = self.quantity.as_deref().map(|q| format!(" {q}")).unwrap_or_default();
        let st = match self.stage {
            Stage::Snapshot => "snapshot",
            Stage::Receipt => "receipt",
            Stage::Candidate => "candidate",
        };
        format!("{k}[{}]{q} @{} ({st})", self.unit, self.dom.describe(grids, archetypes))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Const(f64),
    Param(usize),
    ReadCell(usize),
    ReadEntity(u16, usize),
    CandidateEntity(u16, usize),
    Trait(u16, usize),
    Builtin(u16, Builtin),
    Forcing(usize),
    Region(usize),
    CellArea,
    CellSize,
    IsBoundary,
    Add,
    Sub,
    Mul,
    SafeDiv,
    Neg,
    Abs,
    Min,
    Max,
    Clamp,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    Select,
    Lerp,
    Exp,
    Ln,
    Sin,
    Cos,
    Tanh,
    Pow,
    Curve(Vec<(f64, f64)>),
    Relabel,
    Laplacian,
    GradX,
    GradZ,
    NeighborSum(usize),
    NeighborMean(usize),
    RegionMean(usize),
    RegionSum(usize),
    EdgeFrom,
    EdgeTo,
    Sample,
    SampleOffset {
        forward: f64,
        lateral: f64,
    },
    Crowding(usize),
    /// Keyed random stream id (hash of qualified node id).
    Random(u64),
    Brain(u16),
    /// Receipt of an effect: which leg and what form.
    Receipt {
        effect: usize,
        leg: usize,
        form: ReceiptForm,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Builtin {
    Age,
    Heading,
    Cooldown,
    Generation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum ReceiptForm {
    Amount,
    Rate,
    Fraction,
}

impl Op {
    /// Pointwise ops evaluate each element independently from the same element of inputs.
    pub fn is_pointwise(&self) -> bool {
        !matches!(
            self,
            Op::Laplacian
                | Op::GradX
                | Op::GradZ
                | Op::NeighborSum(_)
                | Op::NeighborMean(_)
                | Op::RegionMean(_)
                | Op::RegionSum(_)
                | Op::EdgeFrom
                | Op::EdgeTo
                | Op::Sample
                | Op::SampleOffset { .. }
        )
    }
    pub fn is_reduction(&self) -> bool {
        matches!(self, Op::RegionMean(_) | Op::RegionSum(_))
    }
    /// Ops whose value is not a pure function of their inputs and args.
    pub fn is_foldable(&self) -> bool {
        !matches!(
            self,
            Op::Param(_)
                | Op::ReadCell(_)
                | Op::ReadEntity(..)
                | Op::CandidateEntity(..)
                | Op::Trait(..)
                | Op::Builtin(..)
                | Op::Forcing(_)
                | Op::Region(_)
                | Op::CellArea
                | Op::CellSize
                | Op::IsBoundary
                | Op::Random(_)
                | Op::Brain(_)
                | Op::Receipt { .. }
                | Op::Crowding(_)
        )
    }
    /// Common-subexpression elimination may merge two identical instances.
    pub fn is_cse_safe(&self) -> bool {
        !matches!(self, Op::Random(_))
    }
}

#[derive(Clone, Debug)]
pub struct Instr {
    pub op: Op,
    pub inputs: Vec<Slot>,
    pub outputs: Vec<Slot>,
    pub dom: Dom,
    pub stage: Stage,
    /// Qualified source node id, e.g. `core.rain/request`.
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SlotInfo {
    pub name: String,
    pub dom: Dom,
    pub stage: Stage,
    pub unit: String,
    pub quantity: Option<String>,
    pub is_bool: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Endpoint {
    /// Cell field at the element's own cell (cells domain) or at the entity's cell (entities).
    Cell(usize),
    EdgeA(usize),
    EdgeB(usize),
    Entity(u16, usize),
    External(usize),
}

impl Endpoint {
    pub fn is_external(&self) -> bool {
        matches!(self, Endpoint::External(_))
    }
}

#[derive(Clone, Debug)]
pub struct Leg {
    pub resource: usize,
    pub from: Endpoint,
    pub to: Endpoint,
    pub amount: Slot,
    /// Amount port is a rate; the engine multiplies by dt exactly once.
    pub is_rate: bool,
}

#[derive(Clone, Debug)]
pub enum EffectKind {
    /// Transfer or coupled reaction: every leg scales by one acceptance factor.
    Process {
        legs: Vec<Leg>,
    },
    /// Signed transport on canonical edges: positive moves A -> B.
    EdgeTransfer {
        resource: usize,
        field: usize,
        rate: Slot,
    },
    RateCell {
        field: usize,
        rate: Slot,
    },
    RateEntity {
        arch: u16,
        field: usize,
        rate: Slot,
    },
    NextCell {
        field: usize,
        value: Slot,
    },
    NextEntity {
        arch: u16,
        field: usize,
        value: Slot,
    },
    Move {
        arch: u16,
        turn: Slot,
        speed: Slot,
    },
    Death {
        arch: u16,
        condition: Slot,
        reason: String,
    },
    Birth {
        arch: u16,
        condition: Slot,
        legs: Vec<(usize, Slot)>,
    },
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub id: String,
    pub rule: String,
    pub dom: Dom,
    pub kind: EffectKind,
    /// Scope coverage in [0,1], multiplied into every amount/rate.
    pub coverage: Option<Slot>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParamInfo {
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub label: String,
    pub stability: Option<crate::schema::Stability>,
    pub density_param: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CostEstimate {
    pub element_ops: u64,
    pub transient_bytes: u64,
    pub effect_elements: u64,
    pub nodes: usize,
}
