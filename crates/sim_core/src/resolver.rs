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
            ent: s.archetypes.iter().map(|a| vec![vec![]; a.fields.len()]).collect(),
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
                                (Endpoint::Cell(f), Dom::Cells(_)) => {
                                    idx.cell[f].push((ei, li, if out { Touch::CellOut } else { Touch::CellIn }))
                                }
                                (Endpoint::Cell(f), Dom::Entities(a)) => {
                                    idx.cell[f].push((ei, li, if out { Touch::EntCellOut(a) } else { Touch::EntCellIn(a) }))
                                }
                                (Endpoint::EdgeA(f), _) => idx.cell[f].push((ei, li, if out { Touch::EdgeAOut } else { Touch::EdgeAIn })),
                                (Endpoint::EdgeB(f), _) => idx.cell[f].push((ei, li, if out { Touch::EdgeBOut } else { Touch::EdgeBIn })),
                                (Endpoint::Entity(a, f), _) => idx.ent[a as usize][f].push((ei, li, out)),
                                (Endpoint::External(_), _) => {}
                                _ => unreachable!("endpoint/domain combination rejected by the compiler"),
                            }
                        }
                    }
                }
                EffectKind::EdgeTransfer { field, .. } => idx.cell[*field].push((ei, 0, Touch::EdgeSigned)),
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

/// Sum of all contributions to one field, per cell, with the same per-cell
/// addition order as `gather_cell` (touch order, then ascending edge id or
/// entity row), computed as one vectorizable pass per touch.
fn accumulate(
    grid: &Grid,
    spatial: &[SpatialIndex],
    touches: &[(usize, usize, Touch)],
    edge_dir: &[Vec<bool>],
    amounts: &[Vec<Vec<f64>>],
    want_out: bool,
    parallel: bool,
) -> Vec<f64> {
    let n = grid.cells();
    let (w, h) = (grid.width, grid.height);
    let he = grid.horizontal_edges();
    let mut acc = vec![0.0; n];
    let rows = |acc: &mut [f64], f: &(dyn Fn(usize, &mut [f64]) + Sync)| {
        if parallel && n > 4096 {
            acc.par_chunks_mut(w).enumerate().for_each(|(z, row)| f(z, row));
        } else {
            acc.chunks_mut(w).enumerate().for_each(|(z, row)| f(z, row));
        }
    };
    for &(e, l, t) in touches {
        let a = &amounts[e][l][..];
        match t {
            Touch::CellOut | Touch::CellIn => {
                if (t == Touch::CellOut) == want_out {
                    for (x, v) in acc.iter_mut().zip(a) {
                        *x += *v;
                    }
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
                rows(&mut acc, &|z, row| {
                    for (x, v) in row.iter_mut().enumerate() {
                        // left (B), right (A), up (B), down (A), ascending edge ids
                        if !role_a && x > 0 {
                            *v += a[z * (w - 1) + x - 1];
                        }
                        if role_a && x + 1 < w {
                            *v += a[z * (w - 1) + x];
                        }
                        if !role_a && z > 0 {
                            *v += a[he + (z - 1) * w + x];
                        }
                        if role_a && z + 1 < h {
                            *v += a[he + z * w + x];
                        }
                    }
                });
            }
            Touch::EdgeSigned => {
                let dir = &edge_dir[e][..];
                rows(&mut acc, &|z, row| {
                    for (x, v) in row.iter_mut().enumerate() {
                        // A -> B withdraws from A; the cell is B on left/up edges, A on right/down.
                        if x > 0 {
                            let edge = z * (w - 1) + x - 1;
                            if !dir[edge] == want_out {
                                *v += a[edge];
                            }
                        }
                        if x + 1 < w {
                            let edge = z * (w - 1) + x;
                            if dir[edge] == want_out {
                                *v += a[edge];
                            }
                        }
                        if z > 0 {
                            let edge = he + (z - 1) * w + x;
                            if !dir[edge] == want_out {
                                *v += a[edge];
                            }
                        }
                        if z + 1 < h {
                            let edge = he + z * w + x;
                            if dir[edge] == want_out {
                                *v += a[edge];
                            }
                        }
                    }
                });
            }
            Touch::EntCellOut(ar) | Touch::EntCellIn(ar) => {
                if matches!(t, Touch::EntCellOut(_)) != want_out {
                    continue;
                }
                // Rows ascend within each cell, so row order preserves the canonical per-cell order.
                for (m, &c) in spatial[ar as usize].cell_of.iter().enumerate() {
                    acc[c as usize] += a[m];
                }
            }
        }
    }
    acc
}

/// Sum of contributions to cell `c` of one field, in canonical order.
#[cfg(test)]
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

/// Run `f` over `0..n`, in parallel across items when requested. Each item is
/// computed serially, so results never depend on the worker count.
fn per_item<T: Send, F: Fn(usize) -> T + Sync + Send>(n: usize, parallel: bool, f: F) -> Vec<T> {
    if parallel {
        (0..n).into_par_iter().map(f).collect()
    } else {
        (0..n).map(f).collect()
    }
}

pub fn resolve(r: &ResolveInputs) -> Result<Resolved, Failure> {
    let plan = r.plan;
    let schema = &plan.schema;
    let nf = schema.cell_fields.len();
    let mut out = Resolved {
        account_in: vec![0.0; schema.accounts.len()],
        account_out: vec![0.0; schema.accounts.len()],
        roundoff: vec![0.0; schema.resources.len()],
        ..Default::default()
    };

    // A. Requested amounts per tick, validated. Parallel across effects.
    let requests: Vec<Result<(Vec<Vec<f64>>, Vec<bool>), Failure>> = per_item(plan.effects.len(), r.parallel, |ei| {
        let e = &plan.effects[ei];
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
                    let v: Vec<f64> = (0..n).map(|i| scale(at(src, i), i, l.is_rate)).collect();
                    check_request(&v, e)?;
                    req.push(v);
                }
                Ok((req, vec![]))
            }
            EffectKind::EdgeTransfer { rate, .. } => {
                let src = &r.slots[*rate];
                let v: Vec<f64> = (0..n).map(|i| scale(at(src, i), i, true)).collect();
                if let Some(i) = v.iter().position(|x| !x.is_finite()) {
                    return Err(Failure {
                        message: format!("non-finite transport request in {}", e.id),
                        effect: Some(e.id.clone()),
                        elements: vec![i],
                    });
                }
                let dir = v.iter().map(|x| *x > 0.0).collect();
                Ok((vec![v.iter().map(|x| x.abs()).collect()], dir))
            }
            _ => Ok((vec![], vec![])),
        }
    });
    for q in requests {
        let (req, dir) = q?;
        out.receipts.requested.push(req);
        out.edge_dir.push(dir);
    }
    let requested = &out.receipts.requested;
    let edge_dir = &out.edge_dir;

    // B/C. Demand and inflow per reservoir, then factors. Parallel across fields.
    let factors: Vec<(Option<Vec<f64>>, Option<Vec<f64>>)> = per_item(nf, r.parallel, |f| {
        let touches = &r.index.cell[f];
        if touches.is_empty() {
            return (None, None);
        }
        let FieldPolicy::Reservoir { capacity, .. } = schema.cell_fields[f].policy else {
            unreachable!()
        };
        let s = &r.state.cells[f];
        let d = accumulate(r.grid, r.spatial, touches, edge_dir, requested, true, false);
        let sf: Vec<f64> = d
            .iter()
            .zip(s)
            .map(|(d, s)| if *d == 0.0 { 1.0 } else { (s.max(0.0) / d).min(1.0) })
            .collect();
        let df = capacity.map(|cf| {
            let cap = &r.state.cells[cf];
            let i = accumulate(r.grid, r.spatial, touches, edge_dir, requested, false, false);
            i.iter()
                .zip(s)
                .zip(cap)
                .map(|((i, s), cap)| if *i == 0.0 { 1.0 } else { ((cap - s).max(0.0) / i).min(1.0) })
                .collect()
        });
        (Some(sf), df)
    });
    for (sf, df) in factors {
        out.source_factor.push(sf);
        out.dest_factor.push(df);
    }
    let mut ent_sf: Vec<Vec<Option<Vec<f64>>>> = vec![];
    let mut ent_df: Vec<Vec<Option<Vec<f64>>>> = vec![];
    for (a, fields) in r.index.ent.iter().enumerate() {
        let (mut sfs, mut dfs) = (vec![], vec![]);
        for (f, touches) in fields.iter().enumerate() {
            if touches.is_empty() {
                sfs.push(None);
                dfs.push(None);
                continue;
            }
            let s = &r.state.entities[a].fields[f];
            let sum = |i: usize, want_out: bool| -> f64 {
                let mut acc = 0.0;
                for &(e, l, is_out) in touches {
                    if is_out == want_out {
                        acc += requested[e][l][i];
                    }
                }
                acc
            };
            sfs.push(Some(
                (0..s.len())
                    .map(|i| {
                        let d = sum(i, true);
                        if d == 0.0 { 1.0 } else { (s[i].max(0.0) / d).min(1.0) }
                    })
                    .collect(),
            ));
            dfs.push(schema.archetypes[a].capacity_slots[f].map(|cs| {
                let cap = &r.slots[cs];
                (0..s.len())
                    .map(|i| {
                        let d = sum(i, false);
                        if d == 0.0 {
                            1.0
                        } else {
                            ((at(cap, i) - s[i]).max(0.0) / d).min(1.0)
                        }
                    })
                    .collect()
            }));
        }
        ent_sf.push(sfs);
        ent_df.push(dfs);
    }

    // D. One acceptance factor per process element. Parallel across effects.
    let sfac = &out.source_factor;
    let dfac = &out.dest_factor;
    let accepted: Vec<(Vec<f64>, Vec<Vec<f64>>)> = per_item(plan.effects.len(), r.parallel, |ei| {
        let e = &plan.effects[ei];
        let n = effect_len(e, r.grid, r.state);
        let factor_at = |ep: Endpoint, i: usize, dest: bool| -> f64 {
            let (tab, idx) = match ep {
                Endpoint::External(_) => return 1.0,
                Endpoint::Cell(f) => {
                    let c = match e.dom {
                        Dom::Entities(a) => r.spatial[a as usize].cell_of[i] as usize,
                        _ => i,
                    };
                    (if dest { &dfac[f] } else { &sfac[f] }, c)
                }
                Endpoint::EdgeA(f) => (if dest { &dfac[f] } else { &sfac[f] }, r.grid.edge_cells(i).0),
                Endpoint::EdgeB(f) => (if dest { &dfac[f] } else { &sfac[f] }, r.grid.edge_cells(i).1),
                Endpoint::Entity(a, f) => (if dest { &ent_df[a as usize][f] } else { &ent_sf[a as usize][f] }, i),
            };
            tab.as_ref().map_or(1.0, |t| t[idx])
        };
        match &e.kind {
            EffectKind::Process { legs } => {
                let req = &requested[ei];
                let alpha: Vec<f64> = (0..n)
                    .map(|i| {
                        let mut a = 1.0f64;
                        for (li, l) in legs.iter().enumerate() {
                            if req[li][i] > 0.0 && l.from != l.to {
                                a = a.min(factor_at(l.from, i, false)).min(factor_at(l.to, i, true));
                            }
                        }
                        a
                    })
                    .collect();
                let acc = req.iter().map(|lv| lv.iter().zip(&alpha).map(|(q, a)| q * a).collect()).collect();
                (alpha, acc)
            }
            EffectKind::EdgeTransfer { field, .. } => {
                let req = &requested[ei][0];
                let dir = &edge_dir[ei];
                let sf = sfac[*field].as_ref().unwrap();
                let df = dfac[*field].as_ref();
                let alpha: Vec<f64> = (0..n)
                    .map(|i| {
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
                    })
                    .collect();
                let acc = vec![req.iter().zip(&alpha).map(|(q, a)| q * a).collect()];
                (alpha, acc)
            }
            _ => (vec![], vec![]),
        }
    });
    for (alpha, acc) in accepted {
        out.receipts.alpha.push(alpha);
        out.accepted.push(acc);
    }

    // E. Integrate reservoirs from the snapshot plus accepted deposits minus withdrawals.
    let accepted = &out.accepted;
    let next: Vec<Result<Option<(Vec<f64>, f64)>, Failure>> = per_item(nf, r.parallel, |f| {
        let touches = &r.index.cell[f];
        if touches.is_empty() {
            return Ok(None);
        }
        let s = &r.state.cells[f];
        let FieldPolicy::Reservoir { capacity, .. } = schema.cell_fields[f].policy else {
            unreachable!()
        };
        let n = r.grid.cells();
        let ins = accumulate(r.grid, r.spatial, touches, edge_dir, accepted, false, false);
        let outs = accumulate(r.grid, r.spatial, touches, edge_dir, accepted, true, false);
        let mut next = Vec::with_capacity(n);
        let mut roundoff = 0.0;
        for c in 0..n {
            let (i, o) = (ins[c], outs[c]);
            let mut v = s[c] + i - o;
            let bound = ROUNDOFF_REL * (s[c].abs() + i + o) + 1e-300;
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
                roundoff += -v;
                v = 0.0;
            }
            if let Some(cf) = capacity {
                let cap = r.state.cells[cf][c];
                if v > cap + bound && v > s[c] + bound {
                    return Err(Failure {
                        message: format!("capacity violation in {} at cell {c}: {v} > {cap}", schema.cell_fields[f].id),
                        effect: None,
                        elements: vec![c],
                    });
                }
            }
            next.push(v);
        }
        Ok(Some((next, roundoff)))
    });
    for (f, n) in next.into_iter().enumerate() {
        match n? {
            Some((v, ro)) => {
                if let FieldPolicy::Reservoir { resource, .. } = schema.cell_fields[f].policy {
                    out.roundoff[resource] += ro;
                }
                out.cell_next.push(Some(v));
            }
            None => out.cell_next.push(None),
        }
    }
    for (a, fields) in r.index.ent.iter().enumerate() {
        let mut row = vec![];
        for (f, touches) in fields.iter().enumerate() {
            if touches.is_empty() {
                row.push(None);
                continue;
            }
            let FieldPolicy::Reservoir { resource, .. } = schema.archetypes[a].fields[f].policy else {
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
                                schema.archetypes[a].id, schema.archetypes[a].fields[f].id, r.state.entities[a].ids[m]
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
            row.push(Some(next));
        }
        out.ent_next.push(row);
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
                message: format!("negative resource request {x} in {}; requests must be nonnegative", e.id),
                effect: Some(e.id.clone()),
                elements: vec![i],
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectorized_accumulation_matches_canonical_gather_bitwise() {
        let grid = Grid {
            id: "g".into(),
            width: 7,
            height: 5,
            cell_size: 1.0,
            chunk_size: 4,
        };
        let (n, e) = (grid.cells(), grid.edges());
        let mut k = 1u64;
        let mut rnd = |len: usize| -> Vec<f64> {
            (0..len)
                .map(|_| {
                    k = crate::rng::mix(k);
                    crate::rng::unit(k) * 1e3
                })
                .collect()
        };
        let ents = crate::state::Entities {
            x: rnd(9).iter().map(|v| v * 0.007).collect(),
            z: rnd(9).iter().map(|v| v * 0.005).collect(),
            ids: (1..10).collect(),
            ..Default::default()
        };
        let spatial = vec![SpatialIndex::build(&grid, &ents)];
        let amounts = vec![vec![rnd(n)], vec![rnd(e)], vec![rnd(e)], vec![rnd(9)], vec![rnd(n)]];
        let dir: Vec<Vec<bool>> = vec![vec![], vec![], rnd(e).iter().map(|v| *v > 500.0).collect(), vec![], vec![]];
        let touches = vec![
            (0, 0, Touch::CellOut),
            (1, 0, Touch::EdgeAOut),
            (1, 0, Touch::EdgeBIn),
            (2, 0, Touch::EdgeSigned),
            (3, 0, Touch::EntCellOut(0)),
            (4, 0, Touch::CellIn),
        ];
        for want_out in [true, false] {
            let v = accumulate(&grid, &spatial, &touches, &dir, &amounts, want_out, true);
            for c in 0..n {
                assert_eq!(
                    v[c].to_bits(),
                    gather_cell(&grid, &spatial, &touches, &dir, &amounts, c, want_out).to_bits()
                );
            }
        }
    }
}
