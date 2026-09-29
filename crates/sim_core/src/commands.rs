//! Tick-stamped commands applied at the tick boundary (phase 0).

use crate::schema::{Package, RegionShape};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// Stable sequence id, assigned when submitted.
    pub seq: u64,
    /// Tick before which the command applies.
    pub tick: u64,
    pub kind: CommandKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandKind {
    /// Change a parameter value within its validated range.
    SetParam { name: String, value: f64 },
    /// Replace the active rule packages (hot swap).
    ApplyPackages { packages: Vec<Package> },
    /// Create a new region mask (initially empty).
    CreateRegion { id: String },
    /// Set region coverage inside a shape (`value` in [0, 1]).
    PaintRegion {
        region: String,
        shape: RegionShape,
        value: f64,
    },
    /// Edit a parameter-policy cell field inside a shape.
    SetField {
        field: String,
        shape: RegionShape,
        value: f64,
    },
    /// Explicit intervention: add (or remove, if negative) resource to a reservoir field.
    /// `amount` is per cell, scaled by shape coverage; recorded in the intervention ledger.
    AddResource {
        field: String,
        shape: RegionShape,
        amount: f64,
    },
    /// Explicit intervention: add organisms with random genomes inside a shape.
    SpawnOrganisms {
        archetype: String,
        shape: RegionShape,
        count: usize,
    },
}
