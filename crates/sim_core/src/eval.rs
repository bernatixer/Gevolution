//! Graph executor. Evaluates plan instructions column-wise over their domain.
//!
//! The reference mode runs instructions one at a time, serially, over whole
//! domains. The parallel mode partitions each domain into fixed chunks and runs
//! fused groups of instructions per chunk; every element performs exactly the
//! same primitive operations in the same order, so results are bit-identical.

use crate::biology::eval_brain;
use crate::compiler::Plan;
use crate::grid::Grid;
use crate::ir::*;
use crate::rng;
use crate::state::{Entities, KahanSum, State};
use rayon::prelude::*;

/// Per-archetype cell buckets built from snapshot positions.
#[derive(Clone, Debug, Default)]
pub struct SpatialIndex {
    pub cell_of: Vec<u32>,
    pub start: Vec<u32>,
    pub members: Vec<u32>,
}

impl SpatialIndex {
    pub fn build(grid: &Grid, e: &Entities) -> SpatialIndex {
        let n = grid.cells();
        let cell_of: Vec<u32> = (0..e.len())
            .map(|i| grid.cell_at(e.x[i], e.z[i]) as u32)
            .collect();
        let mut start = vec![0u32; n + 1];
        for &c in &cell_of {
            start[c as usize + 1] += 1;
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut members = vec![0u32; e.len()];
        for (row, &c) in cell_of.iter().enumerate() {
            members[fill[c as usize] as usize] = row as u32;
            fill[c as usize] += 1;
        }
        SpatialIndex {
            cell_of,
            start,
            members,
        }
    }
    #[inline]
    pub fn in_cell(&self, c: usize) -> &[u32] {
        &self.members[self.start[c] as usize..self.start[c + 1] as usize]
    }
}

/// Accepted/requested process amounts, available to receipt-stage nodes.
#[derive(Clone, Debug, Default)]
pub struct Receipts {
    /// Per effect: acceptance factor per element (empty for non-process effects).
    pub alpha: Vec<Vec<f64>>,
    /// Per effect, per leg: requested amount per tick (nonnegative).
    pub requested: Vec<Vec<Vec<f64>>>,
}

pub struct EvalInputs<'a> {
    pub plan: &'a Plan,
    pub grid: &'a Grid,
    pub state: &'a State,
    pub forcing: &'a [Vec<f64>],
    pub spatial: &'a [SpatialIndex],
    pub candidate: Option<&'a [Entities]>,
    pub receipts: Option<&'a Receipts>,
    pub seed: u64,
    pub dt: f64,
    pub max_neighbors: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Reference,
    Parallel,
}

pub const CHUNK: usize = 1024;

pub fn dom_len(dom: Dom, grid: &Grid, state: &State) -> usize {
    match dom {
        Dom::Uniform => 1,
        Dom::Cells(_) => grid.cells(),
        Dom::Edges(_) => grid.edges(),
        Dom::Entities(a) => state.entities[a as usize].len(),
    }
}

#[derive(Default)]
pub struct Buffers {
    pub slots: Vec<Vec<f64>>,
}

impl Buffers {
    /// Size every slot for its domain, reusing allocations across ticks.
    pub fn prepare(&mut self, plan: &Plan, grid: &Grid, state: &State) {
        self.slots.resize_with(plan.slots.len(), Vec::new);
        for (s, info) in plan.slots.iter().enumerate() {
            let n = dom_len(info.dom, grid, state);
            let b = &mut self.slots[s];
            b.clear();
            b.resize(n, 0.0);
        }
    }
}

/// Fused groups: consecutive instructions of one domain and stage whose
/// non-pointwise inputs are not produced inside the group.
pub fn fuse_groups(plan: &Plan) -> Vec<(Stage, Vec<usize>)> {
    let mut groups: Vec<(Stage, Vec<usize>)> = vec![];
    let mut produced: std::collections::HashSet<Slot> = Default::default();
    for (k, ins) in plan.instrs.iter().enumerate() {
        let standalone =
            matches!(ins.op, Op::Brain(_)) || ins.op.is_reduction() || ins.dom == Dom::Uniform;
        let can_join = groups.last().is_some_and(|(st, g)| {
            let last = &plan.instrs[*g.last().unwrap()];
            *st == ins.stage
                && last.dom == ins.dom
                && !standalone
                && !(matches!(last.op, Op::Brain(_))
                    || last.op.is_reduction()
                    || last.dom == Dom::Uniform)
                && (ins.op.is_pointwise() || ins.inputs.iter().all(|i| !produced.contains(i)))
        });
        if !can_join {
            groups.push((ins.stage, vec![]));
            produced.clear();
        }
        groups.last_mut().unwrap().1.push(k);
        produced.extend(ins.outputs.iter().copied());
    }
    groups
}

pub fn eval_stage(
    inp: &EvalInputs,
    bufs: &mut Buffers,
    stage: Stage,
    mode: Mode,
    groups: &[(Stage, Vec<usize>)],
) {
    match mode {
        Mode::Reference => {
            for (k, ins) in inp.plan.instrs.iter().enumerate() {
                if ins.stage == stage {
                    eval_whole(inp, bufs, k);
                }
            }
        }
        Mode::Parallel => {
            for (st, g) in groups {
                if *st != stage {
                    continue;
                }
                let first = &inp.plan.instrs[g[0]];
                let n = dom_len(first.dom, inp.grid, inp.state);
                if g.len() == 1
                    && (first.dom == Dom::Uniform || first.op.is_reduction() || n <= CHUNK)
                {
                    eval_whole(inp, bufs, g[0]);
                    continue;
                }
                if matches!(first.op, Op::Brain(_)) {
                    eval_whole(inp, bufs, g[0]);
                    continue;
                }
                // Take the group's outputs out so inputs can be shared immutably.
                let outs: Vec<Slot> = g.iter().map(|&k| inp.plan.instrs[k].outputs[0]).collect();
                let mut taken: Vec<Vec<f64>> = outs
                    .iter()
                    .map(|&s| std::mem::take(&mut bufs.slots[s]))
                    .collect();
                {
                    let shared = &bufs.slots;
                    let n_chunks = n.div_ceil(CHUNK);
                    // Split each output into chunks, then transpose so each task owns one chunk of every output.
                    let mut per_chunk: Vec<Vec<&mut [f64]>> =
                        (0..n_chunks).map(|_| Vec::with_capacity(g.len())).collect();
                    for t in taken.iter_mut() {
                        for (ci, ch) in t.chunks_mut(CHUNK).enumerate() {
                            per_chunk[ci].push(ch);
                        }
                    }
                    per_chunk
                        .into_par_iter()
                        .enumerate()
                        .for_each(|(ci, mut outs_chunk)| {
                            let lo = ci * CHUNK;
                            for (j, &k) in g.iter().enumerate() {
                                let ins = &inp.plan.instrs[k];
                                // Inputs produced earlier in this group are read from the chunk-local outputs.
                                let (done, rest) = outs_chunk.split_at_mut(j);
                                let local = |s: Slot| -> Option<&[f64]> {
                                    outs.iter().take(j).position(|&o| o == s).map(|p| &*done[p])
                                };
                                kernel(inp, ins, shared, &local, lo, rest[0]);
                            }
                        });
                }
                for (s, t) in outs.iter().zip(taken) {
                    bufs.slots[*s] = t;
                }
            }
        }
    }
}

fn eval_whole(inp: &EvalInputs, bufs: &mut Buffers, k: usize) {
    let ins = &inp.plan.instrs[k];
    if let Op::Brain(a) = ins.op {
        let mut outs: Vec<Vec<f64>> = ins
            .outputs
            .iter()
            .map(|&s| std::mem::take(&mut bufs.slots[s]))
            .collect();
        eval_brain(inp, &bufs.slots, a, &ins.inputs, &mut outs);
        for (s, o) in ins.outputs.iter().zip(outs) {
            bufs.slots[*s] = o;
        }
        return;
    }
    let s = ins.outputs[0];
    let mut out = std::mem::take(&mut bufs.slots[s]);
    kernel(inp, ins, &bufs.slots, &|_| None, 0, &mut out);
    bufs.slots[s] = out;
}

#[inline(always)]
fn at(v: &[f64], i: usize) -> f64 {
    if v.len() == 1 { v[0] } else { v[i] }
}

/// Evaluate one instruction for elements `lo .. lo + out.len()`.
fn kernel<'b>(
    inp: &EvalInputs,
    ins: &Instr,
    bufs: &'b [Vec<f64>],
    local: &dyn Fn(Slot) -> Option<&'b [f64]>,
    lo: usize,
    out: &mut [f64],
) {
    let n = out.len();
    let st = inp.state;
    let g = inp.grid;
    // Input accessor: chunk-local (index relative to lo) or global.
    let input = |j: usize| -> (&[f64], usize) {
        let s = ins.inputs[j];
        match local(s) {
            Some(l) => (l, 0),
            None => (&bufs[s][..], lo),
        }
    };
    macro_rules! unary {
        ($f:expr) => {{
            let (a, off) = input(0);
            for k in 0..n {
                out[k] = $f(at(a, off + k));
            }
        }};
    }
    macro_rules! binary {
        ($f:expr) => {{
            let (a, oa) = input(0);
            let (b, ob) = input(1);
            let (sa, sb) = (a.len() == 1, b.len() == 1);
            match (sa, sb) {
                (false, false) => {
                    for k in 0..n {
                        out[k] = $f(a[oa + k], b[ob + k]);
                    }
                }
                _ => {
                    for k in 0..n {
                        out[k] = $f(
                            if sa { a[0] } else { a[oa + k] },
                            if sb { b[0] } else { b[ob + k] },
                        );
                    }
                }
            }
        }};
    }
    macro_rules! ternary {
        ($f:expr) => {{
            let (a, oa) = input(0);
            let (b, ob) = input(1);
            let (c, oc) = input(2);
            for k in 0..n {
                out[k] = $f(at(a, oa + k), at(b, ob + k), at(c, oc + k));
            }
        }};
    }
    let b2f = |b: bool| if b { 1.0 } else { 0.0 };
    match &ins.op {
        Op::Const(v) => out.fill(*v),
        Op::Param(p) => out.fill(st.params[*p]),
        Op::ReadCell(f) => out.copy_from_slice(&st.cells[*f][lo..lo + n]),
        Op::ReadEntity(a, f) => {
            out.copy_from_slice(&st.entities[*a as usize].fields[*f][lo..lo + n])
        }
        Op::CandidateEntity(a, f) => {
            let c = inp
                .candidate
                .expect("candidate stage evaluated without candidate state");
            out.copy_from_slice(&c[*a as usize].fields[*f][lo..lo + n]);
        }
        Op::Trait(a, t) => {
            let e = &st.entities[*a as usize];
            for k in 0..n {
                out[k] = e.genome[(lo + k) * e.genome_len + t];
            }
        }
        Op::Builtin(a, b) => {
            let e = &st.entities[*a as usize];
            for k in 0..n {
                let i = lo + k;
                out[k] = match b {
                    Builtin::Age => e.age[i],
                    Builtin::Heading => e.heading[i],
                    Builtin::Cooldown => e.cooldown[i],
                    Builtin::Generation => e.lineage[i].generation as f64,
                };
            }
        }
        Op::Forcing(f) => {
            let v = &inp.forcing[*f];
            if v.len() == 1 {
                out.fill(v[0]);
            } else {
                out.copy_from_slice(&v[lo..lo + n]);
            }
        }
        Op::Region(r) => out.copy_from_slice(&st.regions[*r][lo..lo + n]),
        Op::CellArea => out.fill(g.cell_area()),
        Op::CellSize => out.fill(g.cell_size),
        Op::IsBoundary => {
            for k in 0..n {
                out[k] = b2f(g.is_boundary(lo + k));
            }
        }
        Op::Add => binary!(|a: f64, b: f64| a + b),
        Op::Sub => binary!(|a: f64, b: f64| a - b),
        Op::Mul => binary!(|a: f64, b: f64| a * b),
        Op::SafeDiv => ternary!(|a: f64, b: f64, f: f64| if b == 0.0 { f } else { a / b }),
        Op::Neg => unary!(|a: f64| -a),
        Op::Abs => unary!(|a: f64| a.abs()),
        Op::Min => binary!(|a: f64, b: f64| if b < a { b } else { a }),
        Op::Max => binary!(|a: f64, b: f64| if b > a { b } else { a }),
        Op::Clamp => ternary!(|x: f64, lo: f64, hi: f64| if x < lo {
            lo
        } else if x > hi {
            hi
        } else {
            x
        }),
        Op::Lt => binary!(|a: f64, b: f64| b2f(a < b)),
        Op::Le => binary!(|a: f64, b: f64| b2f(a <= b)),
        Op::Gt => binary!(|a: f64, b: f64| b2f(a > b)),
        Op::Ge => binary!(|a: f64, b: f64| b2f(a >= b)),
        Op::And => binary!(|a: f64, b: f64| b2f(a != 0.0 && b != 0.0)),
        Op::Or => binary!(|a: f64, b: f64| b2f(a != 0.0 || b != 0.0)),
        Op::Not => unary!(|a: f64| b2f(a == 0.0)),
        Op::Select => ternary!(|c: f64, a: f64, b: f64| if c != 0.0 { a } else { b }),
        Op::Lerp => ternary!(|a: f64, b: f64, t: f64| a + (b - a) * t),
        Op::Exp => unary!(|a: f64| a.exp()),
        Op::Ln => unary!(|a: f64| a.ln()),
        Op::Sin => unary!(|a: f64| a.sin()),
        Op::Cos => unary!(|a: f64| a.cos()),
        Op::Tanh => unary!(|a: f64| a.tanh()),
        Op::Pow => binary!(|a: f64, b: f64| a.powf(b)),
        Op::Curve(pts) => unary!(|x: f64| curve(pts, x)),
        Op::Relabel => unary!(|a: f64| a),
        Op::Laplacian => {
            let (a, _) = input(0);
            let inv = 1.0 / (g.cell_size * g.cell_size);
            for k in 0..n {
                let c = lo + k;
                let v = a[c];
                let mut s = 0.0;
                for (e, is_a) in g.incident(c).into_iter().flatten() {
                    let (ea, eb) = g.edge_cells(e);
                    let o = if is_a { eb } else { ea };
                    s += a[o] - v;
                }
                out[k] = s * inv;
            }
        }
        Op::GradX | Op::GradZ => {
            let (a, _) = input(0);
            let w = g.width;
            let dx = g.cell_size;
            let is_x = matches!(ins.op, Op::GradX);
            for k in 0..n {
                let c = lo + k;
                let (x, z) = (c % w, c / w);
                let (len, pos) = if is_x { (w, x) } else { (g.height, z) };
                let step = if is_x { 1 } else { w };
                out[k] = if len < 2 {
                    0.0
                } else if pos == 0 {
                    (a[c + step] - a[c]) / dx
                } else if pos + 1 == len {
                    (a[c] - a[c - step]) / dx
                } else {
                    (a[c + step] - a[c - step]) / (2.0 * dx)
                };
            }
        }
        Op::NeighborSum(r) | Op::NeighborMean(r) => {
            let (a, _) = input(0);
            let r = *r as isize;
            let (w, h) = (g.width as isize, g.height as isize);
            let mean = matches!(ins.op, Op::NeighborMean(_));
            for k in 0..n {
                let c = (lo + k) as isize;
                let (x, z) = (c % w, c / w);
                let mut s = 0.0;
                let mut cnt = 0.0;
                for dz in -r..=r {
                    let zz = z + dz;
                    if zz < 0 || zz >= h {
                        continue;
                    }
                    for dx in -r..=r {
                        let xx = x + dx;
                        if xx < 0 || xx >= w || (dx == 0 && dz == 0) {
                            continue;
                        }
                        s += a[(zz * w + xx) as usize];
                        cnt += 1.0;
                    }
                }
                out[k] = if mean {
                    if cnt > 0.0 { s / cnt } else { 0.0 }
                } else {
                    s
                };
            }
        }
        Op::RegionMean(r) | Op::RegionSum(r) => {
            let (a, _) = input(0);
            let m = &st.regions[*r];
            let mut s = KahanSum::default();
            let mut w = KahanSum::default();
            for c in 0..g.cells() {
                if m[c] != 0.0 {
                    s.add(m[c] * at(a, c));
                    w.add(m[c]);
                }
            }
            out[0] = if matches!(ins.op, Op::RegionSum(_)) {
                s.value()
            } else if w.value() > 0.0 {
                s.value() / w.value()
            } else {
                0.0
            };
        }
        Op::EdgeFrom | Op::EdgeTo => {
            let (a, _) = input(0);
            let from = matches!(ins.op, Op::EdgeFrom);
            if a.len() == 1 {
                out.fill(a[0]);
            } else {
                for k in 0..n {
                    let (ea, eb) = g.edge_cells(lo + k);
                    out[k] = a[if from { ea } else { eb }];
                }
            }
        }
        Op::Sample => {
            let (a, _) = input(0);
            let Dom::Entities(ar) = ins.dom else {
                unreachable!()
            };
            let sp = &inp.spatial[ar as usize];
            for k in 0..n {
                out[k] = at(a, sp.cell_of[lo + k] as usize);
            }
        }
        Op::SampleOffset { forward, lateral } => {
            let (a, _) = input(0);
            let Dom::Entities(ar) = ins.dom else {
                unreachable!()
            };
            let e = &st.entities[ar as usize];
            for k in 0..n {
                let i = lo + k;
                let (s, c) = e.heading[i].sin_cos();
                let px = e.x[i] + forward * c - lateral * s;
                let pz = e.z[i] + forward * s + lateral * c;
                out[k] = at(a, g.cell_at(px, pz));
            }
        }
        Op::Crowding(r) => {
            let Dom::Entities(ar) = ins.dom else {
                unreachable!()
            };
            let sp = &inp.spatial[ar as usize];
            let r = *r as isize;
            let (w, h) = (g.width as isize, g.height as isize);
            let cap = inp.max_neighbors;
            for k in 0..n {
                let i = lo + k;
                let c = sp.cell_of[i] as isize;
                let (x, z) = (c % w, c / w);
                let mut cnt = 0usize;
                'outer: for dz in -r..=r {
                    let zz = z + dz;
                    if zz < 0 || zz >= h {
                        continue;
                    }
                    for dx in -r..=r {
                        let xx = x + dx;
                        if xx < 0 || xx >= w {
                            continue;
                        }
                        for &m in sp.in_cell((zz * w + xx) as usize) {
                            if m as usize != i {
                                cnt += 1;
                                if cnt >= cap {
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
                out[k] = cnt as f64;
            }
        }
        Op::Random(stream) => {
            let tick = st.tick;
            match ins.dom {
                Dom::Entities(a) => {
                    let ids = &st.entities[a as usize].ids;
                    for k in 0..n {
                        out[k] = rng::draw(inp.seed, tick, *stream, ids[lo + k], 0);
                    }
                }
                _ => {
                    for k in 0..n {
                        out[k] = rng::draw(inp.seed, tick, *stream, (lo + k) as u64, 0);
                    }
                }
            }
        }
        Op::Receipt { effect, leg, form } => {
            let r = inp
                .receipts
                .expect("receipt stage evaluated without receipts");
            let alpha = &r.alpha[*effect];
            match form {
                ReceiptForm::Fraction => out.copy_from_slice(&alpha[lo..lo + n]),
                ReceiptForm::Amount | ReceiptForm::Rate => {
                    let req = &r.requested[*effect][*leg];
                    for k in 0..n {
                        let v = alpha[lo + k] * req[lo + k];
                        out[k] = if *form == ReceiptForm::Rate {
                            v / inp.dt
                        } else {
                            v
                        };
                    }
                }
            }
        }
        Op::Brain(_) => unreachable!("brain evaluated separately"),
    }
}

pub fn curve(pts: &[(f64, f64)], x: f64) -> f64 {
    if x <= pts[0].0 {
        return pts[0].1;
    }
    let last = pts[pts.len() - 1];
    if x >= last.0 {
        return last.1;
    }
    for w in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * ((x - x0) / (x1 - x0));
        }
    }
    last.1
}
