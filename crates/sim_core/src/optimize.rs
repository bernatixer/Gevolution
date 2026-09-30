//! Plan optimization under the strict reference numerical profile.
//!
//! Every rewrite preserves each primitive operation's inputs and order, so the
//! optimized plan is bit-identical to the reference plan:
//! - constant folding evaluates the same f64 operation once, with the kernel's formula;
//! - common-subexpression elimination merges identical pure nodes (never random draws);
//! - dead pure nodes are removed (effects, capacities, and scope roots are kept);
//! - scheduling hoists uniform values and clusters instructions by domain so
//!   `eval::fuse_groups` can form large fused kernels;
//! - slots are compacted so dead temporaries allocate no buffers.

use crate::compiler::{Plan, effect_slots};
use crate::eval::curve;
use crate::ir::*;
use std::collections::{BTreeMap, HashMap, HashSet};

pub fn optimize(mut plan: Plan) -> Plan {
    fold_constants(&mut plan);
    eliminate_common(&mut plan);
    remove_dead(&mut plan);
    schedule(&mut plan);
    compact_slots(&mut plan);
    plan.optimized = true;
    plan
}

/// Same arithmetic as `eval::kernel` for one element.
pub fn fold(op: &Op, x: &[f64]) -> Option<f64> {
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    Some(match op {
        Op::Const(v) => *v,
        Op::Add => x[0] + x[1],
        Op::Sub => x[0] - x[1],
        Op::Mul => x[0] * x[1],
        Op::SafeDiv => {
            if x[1] == 0.0 {
                x[2]
            } else {
                x[0] / x[1]
            }
        }
        Op::Neg => -x[0],
        Op::Abs => x[0].abs(),
        Op::Min => {
            if x[1] < x[0] {
                x[1]
            } else {
                x[0]
            }
        }
        Op::Max => {
            if x[1] > x[0] {
                x[1]
            } else {
                x[0]
            }
        }
        Op::Clamp => {
            if x[0] < x[1] {
                x[1]
            } else if x[0] > x[2] {
                x[2]
            } else {
                x[0]
            }
        }
        Op::Lt => b(x[0] < x[1]),
        Op::Le => b(x[0] <= x[1]),
        Op::Gt => b(x[0] > x[1]),
        Op::Ge => b(x[0] >= x[1]),
        Op::And => b(x[0] != 0.0 && x[1] != 0.0),
        Op::Or => b(x[0] != 0.0 || x[1] != 0.0),
        Op::Not => b(x[0] == 0.0),
        Op::Select => {
            if x[0] != 0.0 {
                x[1]
            } else {
                x[2]
            }
        }
        Op::Lerp => x[0] + (x[1] - x[0]) * x[2],
        Op::Exp => x[0].exp(),
        Op::Ln => x[0].ln(),
        Op::Sin => x[0].sin(),
        Op::Cos => x[0].cos(),
        Op::Tanh => x[0].tanh(),
        Op::Pow => x[0].powf(x[1]),
        Op::Curve(p) => curve(p, x[0]),
        Op::Relabel => x[0],
        _ => return None,
    })
}

fn fold_constants(plan: &mut Plan) {
    let mut consts: HashMap<Slot, f64> = HashMap::new();
    for ins in &mut plan.instrs {
        if let Op::Const(v) = ins.op {
            consts.insert(ins.outputs[0], v);
            continue;
        }
        if !ins.op.is_foldable() || !ins.op.is_pointwise() || ins.dom != Dom::Uniform || ins.outputs.len() != 1 {
            continue;
        }
        let vals: Option<Vec<f64>> = ins.inputs.iter().map(|s| consts.get(s).copied()).collect();
        if let Some(v) = vals.and_then(|v| fold(&ins.op, &v)) {
            ins.op = Op::Const(v);
            ins.inputs.clear();
            consts.insert(ins.outputs[0], v);
        }
    }
}

fn op_key(op: &Op) -> String {
    format!("{op:?}")
}

fn replace_slots(plan: &mut Plan, map: &HashMap<Slot, Slot>) {
    if map.is_empty() {
        return;
    }
    let r = |s: &mut Slot| {
        if let Some(t) = map.get(s) {
            *s = *t;
        }
    };
    for ins in &mut plan.instrs {
        ins.inputs.iter_mut().for_each(r);
    }
    for e in &mut plan.effects {
        for_each_effect_slot(e, r);
    }
    for a in &mut plan.schema.archetypes {
        a.capacity_slots.iter_mut().flatten().for_each(r);
    }
    for v in plan.source_map.values_mut() {
        v.iter_mut().for_each(r);
    }
}

fn for_each_effect_slot(e: &mut Effect, mut f: impl FnMut(&mut Slot)) {
    match &mut e.kind {
        EffectKind::Process { legs } => legs.iter_mut().for_each(|l| f(&mut l.amount)),
        EffectKind::EdgeTransfer { rate, .. } => f(rate),
        EffectKind::RateCell { rate, .. } | EffectKind::RateEntity { rate, .. } => f(rate),
        EffectKind::NextCell { value, .. } | EffectKind::NextEntity { value, .. } => f(value),
        EffectKind::Move { turn, speed, .. } => {
            f(turn);
            f(speed)
        }
        EffectKind::Death { condition, .. } => f(condition),
        EffectKind::Birth { condition, legs, .. } => {
            f(condition);
            legs.iter_mut().for_each(|l| f(&mut l.1));
        }
    }
    if let Some(c) = &mut e.coverage {
        f(c);
    }
}

fn eliminate_common(plan: &mut Plan) {
    let mut seen: HashMap<(String, Vec<Slot>, Dom), Slot> = HashMap::new();
    let mut map: HashMap<Slot, Slot> = HashMap::new();
    let mut keep = vec![true; plan.instrs.len()];
    for (k, ins) in plan.instrs.iter_mut().enumerate() {
        for s in ins.inputs.iter_mut() {
            if let Some(t) = map.get(s) {
                *s = *t;
            }
        }
        if !ins.op.is_cse_safe() || ins.outputs.len() != 1 {
            continue;
        }
        let key = (op_key(&ins.op), ins.inputs.clone(), ins.dom);
        match seen.get(&key) {
            Some(&canon) => {
                map.insert(ins.outputs[0], canon);
                keep[k] = false;
            }
            None => {
                seen.insert(key, ins.outputs[0]);
            }
        }
    }
    let mut i = 0;
    plan.instrs.retain(|_| {
        i += 1;
        keep[i - 1]
    });
    replace_slots(plan, &map);
}

fn roots(plan: &Plan) -> HashSet<Slot> {
    let mut r: HashSet<Slot> = plan.effects.iter().flat_map(effect_slots).collect();
    r.extend(
        plan.schema
            .archetypes
            .iter()
            .flat_map(|a| a.capacity_slots.iter().flatten().copied()),
    );
    // Brain observations stay inspectable even when an output is unused.
    for ins in &plan.instrs {
        if matches!(ins.op, Op::Brain(_)) {
            r.extend(ins.inputs.iter().copied());
            r.extend(ins.outputs.iter().copied());
        }
    }
    r
}

fn remove_dead(plan: &mut Plan) {
    let mut live = roots(plan);
    let mut keep = vec![false; plan.instrs.len()];
    for (k, ins) in plan.instrs.iter().enumerate().rev() {
        if ins.outputs.iter().any(|o| live.contains(o)) {
            keep[k] = true;
            live.extend(ins.inputs.iter().copied());
        }
    }
    let mut i = 0;
    plan.instrs.retain(|_| {
        i += 1;
        keep[i - 1]
    });
}

/// List scheduling per stage: uniform values first, then long runs of one domain.
fn schedule(plan: &mut Plan) {
    let n = plan.instrs.len();
    let producer: HashMap<Slot, usize> = plan
        .instrs
        .iter()
        .enumerate()
        .flat_map(|(k, i)| i.outputs.iter().map(move |o| (*o, k)))
        .collect();
    let deps: Vec<Vec<usize>> = plan
        .instrs
        .iter()
        .map(|i| i.inputs.iter().filter_map(|s| producer.get(s).copied()).collect())
        .collect();
    let mut users: Vec<Vec<usize>> = vec![vec![]; n];
    let mut pending: Vec<usize> = vec![0; n];
    for (k, d) in deps.iter().enumerate() {
        let uniq: HashSet<usize> = d.iter().copied().collect();
        pending[k] = uniq.len();
        for u in uniq {
            users[u].push(k);
        }
    }
    let mut order = Vec::with_capacity(n);
    let mut ready: BTreeMap<usize, ()> = (0..n).filter(|&k| pending[k] == 0).map(|k| (k, ())).collect();
    let mut cur: Option<(Stage, Dom)> = None;
    let mut in_group: HashSet<Slot> = HashSet::new();
    while !ready.is_empty() {
        let min_stage = ready.keys().map(|&k| plan.instrs[k].stage).min().unwrap();
        let cands: Vec<usize> = ready.keys().copied().filter(|&k| plan.instrs[k].stage == min_stage).collect();
        let joinable = |k: usize, in_group: &HashSet<Slot>| {
            let ins = &plan.instrs[k];
            cur == Some((ins.stage, ins.dom))
                && !matches!(ins.op, Op::Brain(_))
                && !ins.op.is_reduction()
                && (ins.op.is_pointwise() || ins.inputs.iter().all(|s| !in_group.contains(s)))
        };
        let pick = cands
            .iter()
            .copied()
            .find(|&k| plan.instrs[k].dom == Dom::Uniform && !plan.instrs[k].op.is_reduction())
            .or_else(|| cands.iter().copied().find(|&k| joinable(k, &in_group)))
            .unwrap_or_else(|| {
                // Start a new group in the domain with the most ready work.
                let mut count: BTreeMap<Dom, usize> = BTreeMap::new();
                for &k in &cands {
                    *count.entry(plan.instrs[k].dom).or_default() += 1;
                }
                let best = count
                    .iter()
                    .max_by_key(|(d, c)| (**c, std::cmp::Reverse(**d)))
                    .map(|(d, _)| *d)
                    .unwrap();
                cands.iter().copied().find(|&k| plan.instrs[k].dom == best).unwrap()
            });
        ready.remove(&pick);
        let ins = &plan.instrs[pick];
        if ins.dom != Dom::Uniform {
            if !joinable(pick, &in_group) {
                in_group.clear();
                cur = Some((ins.stage, ins.dom));
            }
            in_group.extend(ins.outputs.iter().copied());
            if matches!(ins.op, Op::Brain(_)) || ins.op.is_reduction() {
                cur = None;
                in_group.clear();
            }
        }
        order.push(pick);
        for &u in &users[pick] {
            pending[u] -= 1;
            if pending[u] == 0 {
                ready.insert(u, ());
            }
        }
    }
    debug_assert_eq!(order.len(), n);
    let old = std::mem::take(&mut plan.instrs);
    let mut slots: Vec<Option<Instr>> = old.into_iter().map(Some).collect();
    plan.instrs = order.into_iter().map(|k| slots[k].take().unwrap()).collect();
}

fn compact_slots(plan: &mut Plan) {
    let mut used: Vec<Slot> = plan.instrs.iter().flat_map(|i| i.outputs.iter().copied()).collect();
    used.sort_unstable();
    used.dedup();
    let map: HashMap<Slot, Slot> = used.iter().enumerate().map(|(new, &old)| (old, new)).collect();
    plan.slots = used.iter().map(|&s| plan.slots[s].clone()).collect();
    for ins in &mut plan.instrs {
        ins.outputs.iter_mut().for_each(|s| *s = map[s]);
        ins.inputs.iter_mut().for_each(|s| *s = map[s]);
    }
    for e in &mut plan.effects {
        for_each_effect_slot(e, |s| *s = map[s]);
    }
    for a in &mut plan.schema.archetypes {
        a.capacity_slots.iter_mut().flatten().for_each(|s| *s = map[s]);
    }
    plan.source_map.retain(|_, v| v.iter().all(|s| map.contains_key(s)));
    for v in plan.source_map.values_mut() {
        v.iter_mut().for_each(|s| *s = map[s]);
    }
}
