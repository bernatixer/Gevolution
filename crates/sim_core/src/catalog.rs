//! Operation catalog: ports, arguments, and descriptions for every node op.
//! Used by the graph editor and documentation; the compiler is authoritative.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    Number,
    Unit,
    Quantity,
    Text,
    /// Cell field or `self.<field>` depending on domain.
    Field,
    CellFieldOnly,
    Forcing,
    Region,
    Param,
    Resource,
    Endpoint,
    Account,
    Integer,
    Points,
    Legs,
    BirthLegs,
    Builtin,
}

#[derive(Clone, Copy, Debug)]
pub struct Arg {
    pub name: &'static str,
    pub kind: ArgKind,
    pub required: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum Inputs {
    /// Positional `inputs` array with these port names.
    Positional(&'static [&'static str]),
    /// Named reference arguments.
    Named(&'static [&'static str]),
    /// Positional, any count up to the archetype's brain input size.
    Variadic,
}

#[derive(Clone, Copy, Debug)]
pub struct OpInfo {
    pub op: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub inputs: Inputs,
    pub args: &'static [Arg],
    pub effect: bool,
    /// Output ports; effects expose receipts.
    pub outputs: &'static [&'static str],
}

const fn a(name: &'static str, kind: ArgKind) -> Arg {
    Arg {
        name,
        kind,
        required: true,
    }
}
const fn o(name: &'static str, kind: ArgKind) -> Arg {
    Arg {
        name,
        kind,
        required: false,
    }
}

const V: &[&str] = &["value"];
const RECEIPTS: &[&str] = &["accepted", "accepted_rate", "fraction"];
const NONE: &[&str] = &[];
const BIN: Inputs = Inputs::Positional(&["a", "b"]);
const UN: Inputs = Inputs::Positional(&["x"]);

pub const OPS: &[OpInfo] = &[
    OpInfo {
        op: "const",
        category: "Values",
        description: "A literal number with a declared unit (and optional quantity).",
        inputs: Inputs::Positional(&[]),
        args: &[
            a("value", ArgKind::Number),
            a("unit", ArgKind::Unit),
            o("quantity", ArgKind::Quantity),
        ],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "parameter",
        category: "Values",
        description: "A rule or package parameter, editable within its validated range.",
        inputs: Inputs::Positional(&[]),
        args: &[a("name", ArgKind::Param)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "read_state",
        category: "Values",
        description: "Snapshot value of a cell field, or self.<field> of the organism.",
        inputs: Inputs::Positional(&[]),
        args: &[a("field", ArgKind::Field)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "read_forcing",
        category: "Values",
        description: "External forcing supplied by the weather model.",
        inputs: Inputs::Positional(&[]),
        args: &[a("forcing", ArgKind::Forcing)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "region",
        category: "Values",
        description: "Region mask coverage in [0, 1] (not a biome).",
        inputs: Inputs::Positional(&[]),
        args: &[a("region", ArgKind::Region)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "cell_area",
        category: "Values",
        description: "Ground area of a cell (m^2).",
        inputs: Inputs::Positional(&[]),
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "cell_size",
        category: "Values",
        description: "Cell spacing (m).",
        inputs: Inputs::Positional(&[]),
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "is_boundary",
        category: "Values",
        description: "True on the outer ring of the world.",
        inputs: Inputs::Positional(&[]),
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "random",
        category: "Values",
        description: "Keyed uniform random in [0,1): its own stream per node, cell/entity, and tick.",
        inputs: Inputs::Positional(&[]),
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "add",
        category: "Arithmetic",
        description: "a + b (units must match; absolute + interval temperature allowed).",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "sub",
        category: "Arithmetic",
        description: "a − b (two absolute temperatures give an interval).",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "mul",
        category: "Arithmetic",
        description: "a × b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "safe_divide",
        category: "Arithmetic",
        description: "a ÷ b, or the fallback (same unit as the result) when b is zero.",
        inputs: Inputs::Positional(&["a", "b", "fallback"]),
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "neg",
        category: "Arithmetic",
        description: "−x.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "abs",
        category: "Arithmetic",
        description: "|x|.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "min",
        category: "Arithmetic",
        description: "Smaller of a and b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "max",
        category: "Arithmetic",
        description: "Larger of a and b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "clamp",
        category: "Arithmetic",
        description: "x limited to [lo, hi].",
        inputs: Inputs::Positional(&["x", "lo", "hi"]),
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "lerp",
        category: "Arithmetic",
        description: "a + (b − a)·t, t dimensionless.",
        inputs: Inputs::Positional(&["a", "b", "t"]),
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "pow",
        category: "Arithmetic",
        description: "a^b, both dimensionless.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "exp",
        category: "Arithmetic",
        description: "e^x, x dimensionless.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "ln",
        category: "Arithmetic",
        description: "ln x, x dimensionless.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "sin",
        category: "Arithmetic",
        description: "sin x (radians).",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "cos",
        category: "Arithmetic",
        description: "cos x (radians).",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "tanh",
        category: "Arithmetic",
        description: "tanh x.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "curve",
        category: "Arithmetic",
        description: "Bounded piecewise-linear response with declared input and output units.",
        inputs: UN,
        args: &[
            a("in_unit", ArgKind::Unit),
            a("out_unit", ArgKind::Unit),
            a("points", ArgKind::Points),
            o("quantity", ArgKind::Quantity),
        ],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "as_quantity",
        category: "Arithmetic",
        description: "Explicitly relabel the semantic quantity (or clear it).",
        inputs: UN,
        args: &[o("quantity", ArgKind::Quantity)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "lt",
        category: "Logic",
        description: "a < b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "le",
        category: "Logic",
        description: "a ≤ b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "gt",
        category: "Logic",
        description: "a > b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "ge",
        category: "Logic",
        description: "a ≥ b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "and",
        category: "Logic",
        description: "a and b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "or",
        category: "Logic",
        description: "a or b.",
        inputs: BIN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "not",
        category: "Logic",
        description: "not x.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "select",
        category: "Logic",
        description: "if cond then a else b (both evaluated).",
        inputs: Inputs::Positional(&["cond", "a", "b"]),
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "laplacian",
        category: "Spatial",
        description: "Discrete Laplacian over the four neighbors (closed boundary).",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "gradient_x",
        category: "Spatial",
        description: "Gradient along x.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "gradient_z",
        category: "Spatial",
        description: "Gradient along z.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "neighbor_sum",
        category: "Spatial",
        description: "Sum over a bounded square neighborhood.",
        inputs: UN,
        args: &[a("radius", ArgKind::Integer)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "neighbor_mean",
        category: "Spatial",
        description: "Mean over a bounded square neighborhood.",
        inputs: UN,
        args: &[a("radius", ArgKind::Integer)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "region_mean",
        category: "Spatial",
        description: "Coverage-weighted mean over a region (snapshot).",
        inputs: UN,
        args: &[a("region", ArgKind::Region)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "region_sum",
        category: "Spatial",
        description: "Coverage-weighted sum over a region (snapshot).",
        inputs: UN,
        args: &[a("region", ArgKind::Region)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "edge_from",
        category: "Spatial",
        description: "Cell value at an edge's first endpoint (edges rules).",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "edge_to",
        category: "Spatial",
        description: "Cell value at an edge's second endpoint (edges rules).",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "sample",
        category: "Organisms",
        description: "Cell value at the organism's snapshot position.",
        inputs: UN,
        args: &[],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "sample_offset",
        category: "Organisms",
        description: "Cell value at a bounded offset relative to heading.",
        inputs: UN,
        args: &[a("forward", ArgKind::Number), o("lateral", ArgKind::Number)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "trait",
        category: "Organisms",
        description: "Inherited trait value from the genome.",
        inputs: Inputs::Positional(&[]),
        args: &[a("name", ArgKind::Text)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "builtin",
        category: "Organisms",
        description: "Engine-owned value: age, heading, cooldown, generation.",
        inputs: Inputs::Positional(&[]),
        args: &[a("name", ArgKind::Builtin)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "crowding",
        category: "Organisms",
        description: "Number of other organisms within a bounded radius (capped).",
        inputs: Inputs::Positional(&[]),
        args: &[a("radius", ArgKind::Integer)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "candidate_state",
        category: "Organisms",
        description: "Candidate (post-integration) value of self.<field>, for lifecycle conditions.",
        inputs: Inputs::Positional(&[]),
        args: &[a("field", ArgKind::Field)],
        effect: false,
        outputs: V,
    },
    OpInfo {
        op: "brain",
        category: "Organisms",
        description: "The inherited neural network: normalized observations in, action tendencies out.",
        inputs: Inputs::Variadic,
        args: &[],
        effect: false,
        outputs: &["out0", "out1", "out2", "out3", "out4", "out5"],
    },
    OpInfo {
        op: "transfer",
        category: "Effects",
        description: "Request moving one resource between two reservoirs; the shared resolver decides the accepted amount.",
        inputs: Inputs::Named(&["rate"]),
        args: &[
            a("resource", ArgKind::Resource),
            a("from", ArgKind::Endpoint),
            a("to", ArgKind::Endpoint),
        ],
        effect: true,
        outputs: RECEIPTS,
    },
    OpInfo {
        op: "external_source",
        category: "Effects",
        description: "Bring a resource in from a declared external account (e.g. weather).",
        inputs: Inputs::Named(&["rate"]),
        args: &[
            a("resource", ArgKind::Resource),
            a("account", ArgKind::Account),
            a("to", ArgKind::Endpoint),
        ],
        effect: true,
        outputs: RECEIPTS,
    },
    OpInfo {
        op: "external_sink",
        category: "Effects",
        description: "Send a resource out to a declared external account (e.g. atmosphere).",
        inputs: Inputs::Named(&["rate"]),
        args: &[
            a("resource", ArgKind::Resource),
            a("from", ArgKind::Endpoint),
            a("account", ArgKind::Account),
        ],
        effect: true,
        outputs: RECEIPTS,
    },
    OpInfo {
        op: "reaction",
        category: "Effects",
        description: "Coupled transfers sharing one acceptance factor.",
        inputs: Inputs::Named(&[]),
        args: &[a("legs", ArgKind::Legs)],
        effect: true,
        outputs: RECEIPTS,
    },
    OpInfo {
        op: "edge_transfer",
        category: "Effects",
        description: "Signed transport along canonical edges (positive: first → second endpoint).",
        inputs: Inputs::Named(&["rate"]),
        args: &[a("resource", ArgKind::Resource), a("field", ArgKind::CellFieldOnly)],
        effect: true,
        outputs: RECEIPTS,
    },
    OpInfo {
        op: "rate_contribution",
        category: "Effects",
        description: "Add a rate to an integrated (non-conserved) state field.",
        inputs: Inputs::Named(&["rate"]),
        args: &[a("field", ArgKind::Field)],
        effect: true,
        outputs: NONE,
    },
    OpInfo {
        op: "next_value",
        category: "Effects",
        description: "The single exclusive next value of a next-value field.",
        inputs: Inputs::Named(&["value"]),
        args: &[a("field", ArgKind::Field)],
        effect: true,
        outputs: NONE,
    },
    OpInfo {
        op: "move",
        category: "Effects",
        description: "Bounded movement intent (turn rate, speed); fund it with an energy transfer's receipt.",
        inputs: Inputs::Named(&["turn", "speed"]),
        args: &[],
        effect: true,
        outputs: NONE,
    },
    OpInfo {
        op: "death",
        category: "Effects",
        description: "Lifecycle death when the condition holds; death precedes birth.",
        inputs: Inputs::Named(&["condition"]),
        args: &[o("reason", ArgKind::Text)],
        effect: true,
        outputs: NONE,
    },
    OpInfo {
        op: "birth",
        category: "Effects",
        description: "Asexual birth: transfers stores from parent to offspring atomically.",
        inputs: Inputs::Named(&["condition"]),
        args: &[a("legs", ArgKind::BirthLegs)],
        effect: true,
        outputs: NONE,
    },
];

pub fn info(op: &str) -> Option<&'static OpInfo> {
    OPS.iter().find(|o| o.op == op)
}

pub fn categories() -> Vec<&'static str> {
    let mut c: Vec<&str> = vec![];
    for o in OPS {
        if !c.contains(&o.category) {
            c.push(o.category);
        }
    }
    c
}

/// Every reference a node makes: (port name, reference string).
pub fn node_refs(node: &crate::schema::Node) -> Vec<(String, String)> {
    let mut out = vec![];
    if let Some(serde_json::Value::Array(a)) = node.args.get("inputs") {
        let names: Vec<&str> = match info(&node.op).map(|i| i.inputs) {
            Some(Inputs::Positional(n)) => n.to_vec(),
            _ => vec![],
        };
        for (i, v) in a.iter().enumerate() {
            if let Some(s) = v.as_str() {
                out.push((names.get(i).map(|x| x.to_string()).unwrap_or(format!("in{i}")), s.to_string()));
            }
        }
    }
    for key in ["rate", "amount", "value", "turn", "speed", "condition"] {
        if let Some(s) = node.args.get(key).and_then(|v| v.as_str()) {
            out.push((key.to_string(), s.to_string()));
        }
    }
    if let Some(serde_json::Value::Array(legs)) = node.args.get("legs") {
        for (i, l) in legs.iter().enumerate() {
            for key in ["rate", "amount"] {
                if let Some(s) = l.get(key).and_then(|v| v.as_str()) {
                    out.push((format!("legs[{i}].{key}"), s.to_string()));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_covers_every_compiled_op() {
        // Every op used by the standard pack is described.
        let root = crate::assets::asset_root().join("rules/core");
        for f in ["environment.json", "biology.json"] {
            let p = crate::assets::load_package(&root.join(f)).unwrap();
            for r in &p.rules {
                for n in &r.nodes {
                    assert!(super::info(&n.op).is_some(), "op {} missing from catalog", n.op);
                }
            }
        }
    }
}
