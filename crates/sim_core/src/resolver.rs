//! Shared resource resolver: conservative proportional scaling (policy v1).
//!
//! Every withdrawal and deposit on a reservoir is gathered from the snapshot
//! before anything is applied. Each process receives one acceptance factor,
//! the minimum of 1 and every relevant source/destination factor, and all of
//! its legs scale by it. Gathers visit contributions in canonical order
//! (effect id, then element order), so results do not depend on scheduling.

use crate::compiler::{FieldPolicy, Plan};
use crate::eval::{Receipts, SpatialIndex};
use crate::grid::Grid;
use crate::ir::*;
use crate::state::{KahanSum, State};
use rayon::prelude::*;

pub const ALLOCATION_POLICY: &str = "proportional-conservative.v1";
/// Roundoff bound relative to the magnitudes involved in one reservoir update.
pub const ROUNDOFF_REL: f64 = 1e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touch {
    CellOut,
    CellIn,
    EdgeAOut,
    EdgeBOut,
    EdgeAIn,
    EdgeBIn,
    EntCellOut(u16),
    EntCellIn(u16),
    EdgeSigned,
}

/// Which process legs touch each reservoir, in canonical order.
#[derive(Clone, Debug, Default)]
pub struct ResolverIndex {
    pub cell: Vec<Vec<(usize, usize, Touch)>>,
    /// Per archetype, per field: (effect, leg, is_withdrawal).
    pub ent: Vec<Vec<Vec<(usize, usize, bool)>>>,
}

impl ResolverIndex {
    pub fn build(plan: &Plan) -> ResolverIndex {
        let s = &plan.schema;
        let mut idx = ResolverIndex {
            cell: vec![vec![]; s.cell_fields.len()],
            ent: s
                .archetypes
                .iter()
                .map(|a| vec![vec![]; a.fields.len()])
                .collect(),
        };
        for (ei, e) in plan.effects.iter().enumerate() {
            match &e.kind {
                EffectKind::Process { legs } => {
                    for (li, l) in legs.iter().enumerate() {
                        if l.from == l.to {
                            continue; // no-op self transfer is eliminated before arbitration
                        }
                        for (ep, out) in [(l.from, true), (l.to, false)] {
                            match (ep, e.dom) {
                                (Endpoint::Cell(f), Dom::Cells(_)) => idx.cell[f].push((
                                    ei,
                                    li,
                                    if out { Touch::CellOut } else { Touch::CellIn },
                                )),
                                (Endpoint::Cell(f), Dom::Entities(a)) => idx.cell[f].push((
                                    ei,
                                    li,
                                    if out {
                                        Touch::EntCellOut(a)
                                    } else {
                                        Touch::EntCellIn(a)
                                    },
                                )),
                                (Endpoint::EdgeA(f), _) => idx.cell[f].push((
                                    ei,
                                    li,
                                    if out { Touch::EdgeAOut } else { Touch::EdgeAIn },
                                )),
                                (Endpoint::EdgeB(f), _) => idx.cell[f].push((
                                    ei,
                                    li,
                                    if out { Touch::EdgeBOut } else { Touch::EdgeBIn },
                                )),
                                (Endpoint::Entity(a, f), _) => {
                                    idx.ent[a as usize][f].push((ei, li, out))
                                }
                                (Endpoint::External(_), _) => {}
                                _ => unreachable!(
                                    "endpoint/domain combination rejected by the compiler"
                                ),
                            }
                        }
                    }
                }
                EffectKind::EdgeTransfer { field, .. } => {
                    idx.cell[*field].push((ei, 0, Touch::EdgeSigned))
                }
                _ => {}
            }
        }
        idx
    }
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub message: String,
    pub effect: Option<String>,
    pub elements: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub receipts: Receipts,
    /// Accepted amount per effect, per leg, per element.
    pub accepted: Vec<Vec<Vec<f64>>>,
    /// Edge transfer direction per effect (true = A -> B).
    pub edge_dir: Vec<Vec<bool>>,
    /// Candidate values for reservoir cell fields.
    pub cell_next: Vec<Option<Vec<f64>>>,
    pub ent_next: Vec<Vec<Option<Vec<f64>>>>,
    pub source_factor: Vec<Option<Vec<f64>>>,
    pub dest_factor: Vec<Option<Vec<f64>>>,
    /// Accepted amounts this tick by account: delivered into the world, removed from it.
    pub account_in: Vec<f64>,
    pub account_out: Vec<f64>,
    /// Bounded roundoff corrections this tick by resource.
    pub roundoff: Vec<f64>,
}

pub struct ResolveInputs<'a> {
    pub plan: &'a Plan,
    pub index: &'a ResolverIndex,
    pub grid: &'a Grid,
    pub state: &'a State,
    pub slots: &'a [Vec<f64>],
    pub spatial: &'a [SpatialIndex],
    pub dt: f64,
    pub parallel: bool,
}

#[inline]
fn at(v: &[f64], i: usize) -> f64 {
    if v.len() == 1 { v[0] } else { v[i] }
}

fn effect_len(e: &Effect, grid: &Grid, state: &State) -> usize {
    match e.dom {
        Dom::Uniform => 1,
        Dom::Cells(_) => grid.cells(),
        Dom::Edges(_) => grid.edges(),
        Dom::Entities(a) => state.entities[a as usize].len(),
    }
}

fn map_elems<F: Fn(usize) -> f64 + Sync + Send>(n: usize, parallel: bool, f: F) -> Vec<f64> {
    if parallel && n > 4096 {
        (0..n).into_par_iter().map(f).collect()
    } else {
        (0..n).map(f).collect()
    }
}

/// Sum of contributions to cell `c` of one field, in canonical order.
#[inline]
fn gather_cell(
    grid: &Grid,
    spatial: &[SpatialIndex],
    touches: &[(usize, usize, Touch)],
    edge_dir: &[Vec<bool>],
    amounts: &[Vec<Vec<f64>>],
    c: usize,
    want_out: bool,
) -> f64 {
    let mut s = 0.0;
    for &(e, l, t) in touches {
        let a = &amounts[e][l];
        match t {
            Touch::CellOut => {
                if want_out {
                    s += a[c]
                }
            }
            Touch::CellIn => {
                if !want_out {
                    s += a[c]
                }
            }
            Touch::EdgeAOut | Touch::EdgeBOut | Touch::EdgeAIn | Touch::EdgeBIn => {
                let (role_a, role_out) = match t {
                    Touch::EdgeAOut => (true, true),
                    Touch::EdgeBOut => (false, true),
                    Touch::EdgeAIn => (true, false),
                    _ => (false, false),
                };
                if role_out != want_out {
                    continue;
                }
                for (edge, is_a) in grid.incident(c).into_iter().flatten() {
                    if is_a == role_a {
                        s += a[edge];
                    }
                }
            }
            Touch::EdgeSigned => {
                let dir = &edge_dir[e];
                for (edge, is_a) in grid.incident(c).into_iter().flatten() {
                    // A -> B withdraws from A; B -> A withdraws from B.
                    let is_source = dir[edge] == is_a;
                    if is_source == want_out {
                        s += a[edge];
                    }
                }
            }
            Touch::EntCellOut(ar) | Touch::EntCellIn(ar) => {
                if matches!(t, Touch::EntCellOut(_)) != want_out {
                    continue;
                }
                for &m in spatial[ar as usize].in_cell(c) {
                    s += a[m as usize];
                }
            }
        }
    }
    s
}

pub fn resolve(r: &ResolveInputs) -> Result<Resolved, Failure> {
    let plan = r.plan;
    let schema = &plan.schema;
    let ne = plan.effects.len();
    let mut out = Resolved {
        receipts: Receipts {
            alpha: vec![vec![]; ne],
            requested: vec![vec![]; ne],
        },
        accepted: vec![vec![]; ne],
        edge_dir: vec![vec![]; ne],
        cell_next: vec![None; schema.cell_fields.len()],
        ent_next: schema
            .archetypes
            .iter()
            .map(|a| vec![None; a.fields.len()])
            .collect(),
        source_factor: vec![None; schema.cell_fields.len()],
        dest_factor: vec![None; schema.cell_fields.len()],
        account_in: vec![0.0; schema.accounts.len()],
        account_out: vec![0.0; schema.accounts.len()],
        roundoff: vec![0.0; schema.resources.len()],
    };

    // A. Requested amounts per tick, validated.
    for (ei, e) in plan.effects.iter().enumerate() {
        let n = effect_len(e, r.grid, r.state);
        let cov = e.coverage.map(|s| &r.slots[s][..]);
        let scale = |v: f64, i: usize, is_rate: bool| -> f64 {
            let v = if is_rate { v * r.dt } else { v };
            match cov {
                Some(c) => v * at(c, i),
                None => v,
            }
        };
        match &e.kind {
            EffectKind::Process { legs } => {
                let mut req = Vec::with_capacity(legs.len());
                for l in legs {
                    let src = &r.slots[l.amount];
                    let v = map_elems(n, r.parallel, |i| scale(at(src, i), i, l.is_rate));
                    check_request(&v, e)?;
                    req.push(v);
                }
                out.receipts.requested[ei] = req;
            }
            EffectKind::EdgeTransfer { rate, .. } => {
                let src = &r.slots[*rate];
                let v = map_elems(n, r.parallel, |i| scale(at(src, i), i, true));
                if let Some(i) = v.iter().position(|x| !x.is_finite()) {
                    return Err(Failure {
                        message: format!("non-finite transport request in {}", e.id),
                        effect: Some(e.id.clone()),
                        elements: vec![i],
                    });
                }
                out.edge_dir[ei] = v.iter().map(|x| *x > 0.0).collect();
                out.receipts.requested[ei] = vec![v.iter().map(|x| x.abs()).collect()];
            }
            _ => {}
        }
    }

    let requested = &out.receipts.requested;
    let edge_dir = &out.edge_dir;

    // B/C. Demand and inflow per reservoir, then factors.
    for (f, touches) in r.index.cell.iter().enumerate() {
        if touches.is_empty() {
            continue;
        }
        let FieldPolicy::Reservoir { capacity, .. } = schema.cell_fields[f].policy else {
            unreachable!()
        };
        let s = &r.state.cells[f];
        let n = r.grid.cells();
        let sf = map_elems(n, r.parallel, |c| {
            let d = gather_cell(r.grid, r.spatial, touches, edge_dir, requested, c, true);
            if d == 0.0 {
                1.0
            } else {
                (s[c].max(0.0) / d).min(1.0)
            }
        });
        out.source_factor[f] = Some(sf);
        if let Some(cf) = capacity {
            let cap = &r.state.cells[cf];
            let df = map_elems(n, r.parallel, |c| {
                let i = gather_cell(r.grid, r.spatial, touches, edge_dir, requested, c, false);
                if i == 0.0 {
                    1.0
                } else {
                    ((cap[c] - s[c]).max(0.0) / i).min(1.0)
                }
            });
            out.dest_factor[f] = Some(df);
        }
    }
    let mut ent_sf: Vec<Vec<Option<Vec<f64>>>> = schema
        .archetypes
        .iter()
        .map(|a| vec![None; a.fields.len()])
        .collect();
    let mut ent_df = ent_sf.clone();
    for (a, fields) in r.index.ent.iter().enumerate() {
        for (f, touches) in fields.iter().enumerate() {
            if touches.is_empty() {
                continue;
            }
            let s = &r.state.entities[a].fields[f];
            let m = s.len();
            let sum = |i: usize, want_out: bool| -> f64 {
                let mut acc = 0.0;
                for &(e, l, is_out) in touches {
                    if is_out == want_out {
                        acc += requested[e][l][i];
                    }
                }
                acc
            };
            ent_sf[a][f] = Some(map_elems(m, r.parallel, |i| {
                let d = sum(i, true);
                if d == 0.0 {
                    1.0
                } else {
                    (s[i].max(0.0) / d).min(1.0)
                }
            }));
            if let Some(cs) = schema.archetypes[a].capacity_slots[f] {
                let cap = &r.slots[cs];
                ent_df[a][f] = Some(map_elems(m, r.parallel, |i| {
                    let d = sum(i, false);
                    if d == 0.0 {
                        1.0
                    } else {
                        ((at(cap, i) - s[i]).max(0.0) / d).min(1.0)
                    }
                }));
            }
        }
    }

    // D. One acceptance factor per process element.
    for (ei, e) in plan.effects.iter().enumerate() {
        let n = effect_len(e, r.grid, r.state);
        let factor_at = |ep: Endpoint, i: usize, dest: bool| -> f64 {
            let (tab, idx) = match ep {
                Endpoint::External(_) => return 1.0,
                Endpoint::Cell(f) => {
                    let c = match e.dom {
                        Dom::Entities(a) => r.spatial[a as usize].cell_of[i] as usize,
                        _ => i,
                    };
                    (
                        if dest {
                            &out.dest_factor[f]
                        } else {
                            &out.source_factor[f]
                        },
                        c,
                    )
                }
                Endpoint::EdgeA(f) => (
                    if dest {
                        &out.dest_factor[f]
                    } else {
                        &out.source_factor[f]
                    },
                    r.grid.edge_cells(i).0,
                ),
                Endpoint::EdgeB(f) => (
                    if dest {
                        &out.dest_factor[f]
                    } else {
                        &out.source_factor[f]
                    },
                    r.grid.edge_cells(i).1,
                ),
                Endpoint::Entity(a, f) => (
                    if dest {
                        &ent_df[a as usize][f]
                    } else {
                        &ent_sf[a as usize][f]
                    },
                    i,
                ),
            };
            tab.as_ref().map_or(1.0, |t| t[idx])
        };
        match &e.kind {
            EffectKind::Process { legs } => {
                let req = &requested[ei];
                let alpha = map_elems(n, r.parallel, |i| {
                    let mut a = 1.0f64;
                    for (li, l) in legs.iter().enumerate() {
                        if req[li][i] > 0.0 && l.from != l.to {
                            a = a
                                .min(factor_at(l.from, i, false))
                                .min(factor_at(l.to, i, true));
                        }
                    }
                    a
                });
                out.accepted[ei] = req
                    .iter()
                    .map(|lv| lv.iter().zip(&alpha).map(|(q, a)| q * a).collect())
                    .collect();
                out.receipts.alpha[ei] = alpha;
            }
            EffectKind::EdgeTransfer { field, .. } => {
                let req = &requested[ei][0];
                let dir = &out.edge_dir[ei];
                let sf = out.source_factor[*field].as_ref().unwrap();
                let df = out.dest_factor[*field].as_ref();
                let alpha = map_elems(n, r.parallel, |i| {
                    if req[i] <= 0.0 {
                        return 1.0;
                    }
                    let (a, b) = r.grid.edge_cells(i);
                    let (src, dst) = if dir[i] { (a, b) } else { (b, a) };
                    let mut x = sf[src].min(1.0);
                    if let Some(df) = df {
                        x = x.min(df[dst]);
                    }
                    x
                });
                out.accepted[ei] = vec![req.iter().zip(&alpha).map(|(q, a)| q * a).collect()];
                out.receipts.alpha[ei] = alpha;
            }
            _ => {}
        }
    }

    // E. Integrate reservoirs from the snapshot plus accepted deposits minus withdrawals.
    let accepted = &out.accepted;
    for (f, touches) in r.index.cell.iter().enumerate() {
        if touches.is_empty() {
            continue;
        }
        let s = &r.state.cells[f];
        let FieldPolicy::Reservoir { capacity, resource } = schema.cell_fields[f].policy else {
            unreachable!()
        };
        let n = r.grid.cells();
        let parts: Vec<(f64, f64, f64)> = (0..n)
            .into_par_iter()
            .with_min_len(if r.parallel { 4096 } else { usize::MAX })
            .map(|c| {
                let i = gather_cell(r.grid, r.spatial, touches, edge_dir, accepted, c, false);
                let o = gather_cell(r.grid, r.spatial, touches, edge_dir, accepted, c, true);
                (s[c] + i - o, i, o)
            })
            .collect();
        let mut next = Vec::with_capacity(n);
        for (c, (v, i, o)) in parts.into_iter().enumerate() {
            let bound = ROUNDOFF_REL * (s[c].abs() + i + o) + 1e-300;
            let mut v = v;
            if v < 0.0 {
                if v < -bound {
                    return Err(Failure {
                        message: format!(
                            "material negative inventory in {} at cell {c}: {v} (snapshot {}, in {i}, out {o})",
                            schema.cell_fields[f].id, s[c]
                        ),
                        effect: None,
                        elements: vec![c],
                    });
                }
                out.roundoff[resource] += -v;
                v = 0.0;
            }
            if let Some(cf) = capacity {
                let cap = r.state.cells[cf][c];
                if v > cap + bound && v > s[c] + bound {
                    return Err(Failure {
                        message: format!(
                            "capacity violation in {} at cell {c}: {v} > {cap}",
                            schema.cell_fields[f].id
                        ),
                        effect: None,
                        elements: vec![c],
                    });
                }
            }
            next.push(v);
        }
        out.cell_next[f] = Some(next);
    }
    for (a, fields) in r.index.ent.iter().enumerate() {
        for (f, touches) in fields.iter().enumerate() {
            if touches.is_empty() {
                continue;
            }
            let FieldPolicy::Reservoir { resource, .. } = schema.archetypes[a].fields[f].policy
            else {
                unreachable!()
            };
            let s = &r.state.entities[a].fields[f];
            let mut next = Vec::with_capacity(s.len());
            for (m, &sm) in s.iter().enumerate() {
                let (mut i, mut o) = (0.0, 0.0);
                for &(e, l, is_out) in touches {
                    let v = accepted[e][l][m];
                    if is_out { o += v } else { i += v }
                }
                let mut v = sm + i - o;
                let bound = ROUNDOFF_REL * (sm.abs() + i + o) + 1e-300;
                if v < 0.0 {
                    if v < -bound {
                        return Err(Failure {
                            message: format!(
                                "material negative inventory in {}.{} of entity {}: {v}",
                                schema.archetypes[a].id,
                                schema.archetypes[a].fields[f].id,
                                r.state.entities[a].ids[m]
                            ),
                            effect: None,
                            elements: vec![m],
                        });
                    }
                    out.roundoff[resource] += -v;
                    v = 0.0;
                }
                next.push(v);
            }
            out.ent_next[a][f] = Some(next);
        }
    }

    // External exchanges, canonical order, compensated.
    let mut acc_in: Vec<KahanSum> = vec![KahanSum::default(); schema.accounts.len()];
    let mut acc_out = acc_in.clone();
    for (ei, e) in plan.effects.iter().enumerate() {
        if let EffectKind::Process { legs } = &e.kind {
            for (li, l) in legs.iter().enumerate() {
                let target = match (l.from, l.to) {
                    (Endpoint::External(a), _) => &mut acc_in[a],
                    (_, Endpoint::External(a)) => &mut acc_out[a],
                    _ => continue,
                };
                for v in &accepted[ei][li] {
                    target.add(*v);
                }
            }
        }
    }
    out.account_in = acc_in.iter().map(|k| k.value()).collect();
    out.account_out = acc_out.iter().map(|k| k.value()).collect();
    Ok(out)
}

fn check_request(v: &[f64], e: &Effect) -> Result<(), Failure> {
    for (i, x) in v.iter().enumerate() {
        if !x.is_finite() {
            return Err(Failure {
                message: format!("non-finite resource request in {}", e.id),
                effect: Some(e.id.clone()),
                elements: vec![i],
            });
        }
        if *x < 0.0 {
            return Err(Failure {
                message: format!(
                    "negative resource request {x} in {}; requests must be nonnegative",
                    e.id
                ),
                effect: Some(e.id.clone()),
                elements: vec![i],
            });
        }
    }
    Ok(())
}
