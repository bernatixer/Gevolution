//! Genomes, the fixed-topology brain backend, and mutation.
//!
//! Genome layout: [traits..., W1 (hidden x inputs), b1 (hidden), W2 (outputs x hidden), b2 (outputs)].

use crate::compiler::ArchInfo;
use crate::eval::EvalInputs;
use crate::ir::Slot;
use crate::rng;
use rayon::prelude::*;

pub const MUTATION_VERSION: u16 = 1;
pub const BRAIN_BACKEND: &str = "mlp.tanh.v1";

/// Boundary for replacing the fixed network later (e.g. with a bounded graph genome).
pub trait BrainBackend {
    fn parameter_count(&self) -> usize;
    fn eval(&self, params: &[f64], inputs: &[f64], hidden: &mut [f64], outputs: &mut [f64]);
}

pub struct Mlp {
    pub inputs: usize,
    pub hidden: usize,
    pub outputs: usize,
}

impl BrainBackend for Mlp {
    fn parameter_count(&self) -> usize {
        self.inputs * self.hidden + self.hidden + self.hidden * self.outputs + self.outputs
    }
    #[inline]
    fn eval(&self, p: &[f64], x: &[f64], h: &mut [f64], y: &mut [f64]) {
        let (ni, nh, no) = (self.inputs, self.hidden, self.outputs);
        let (w1, rest) = p.split_at(ni * nh);
        let (b1, rest) = rest.split_at(nh);
        let (w2, b2) = rest.split_at(nh * no);
        for j in 0..nh {
            let mut s = b1[j];
            let row = &w1[j * ni..(j + 1) * ni];
            for i in 0..ni {
                s += row[i] * x[i];
            }
            h[j] = s.tanh();
        }
        for k in 0..no {
            let mut s = b2[k];
            let row = &w2[k * nh..(k + 1) * nh];
            for j in 0..nh {
                s += row[j] * h[j];
            }
            y[k] = s.tanh();
        }
    }
}

pub fn mlp_of(a: &ArchInfo) -> Mlp {
    Mlp {
        inputs: a.brain.inputs,
        hidden: a.brain.hidden,
        outputs: a.brain.outputs,
    }
}

/// Batched brain evaluation for every entity of an archetype.
pub fn eval_brain(
    inp: &EvalInputs,
    bufs: &[Vec<f64>],
    arch: u16,
    inputs: &[Slot],
    outs: &mut [Vec<f64>],
) {
    let ai = &inp.plan.schema.archetypes[arch as usize];
    let e = &inp.state.entities[arch as usize];
    let net = mlp_of(ai);
    let nt = ai.traits.len();
    let m = e.len();
    let no = net.outputs;
    let mut flat = vec![0.0; m * no];
    let body = |i: usize, y: &mut [f64]| {
        let mut x = [0.0f64; 64];
        let mut h = [0.0f64; 128];
        for (j, &s) in inputs.iter().enumerate() {
            let b = &bufs[s];
            x[j] = if b.len() == 1 { b[0] } else { b[i] };
        }
        let g = e.genome_of(i);
        net.eval(&g[nt..], &x[..net.inputs], &mut h[..net.hidden], y);
    };
    if m > 256 {
        flat.par_chunks_mut(no)
            .enumerate()
            .for_each(|(i, y)| body(i, y));
    } else {
        flat.chunks_mut(no)
            .enumerate()
            .for_each(|(i, y)| body(i, y));
    }
    for (k, o) in outs.iter_mut().enumerate() {
        for i in 0..m {
            o[i] = flat[i * no + k];
        }
    }
}

pub const STREAM_GENOME_INIT: u64 = 0x6E0_0001;
pub const STREAM_MUTATION: u64 = 0x6E0_0002;
pub const STREAM_MUTATION_SIZE: u64 = 0x6E0_0003;

/// Random genome within declared initialization ranges.
pub fn random_genome(a: &ArchInfo, seed: u64, id: u64) -> Vec<f64> {
    let mut g = Vec::with_capacity(a.genome_len());
    for (k, t) in a.traits.iter().enumerate() {
        let u = rng::draw(seed, 0, STREAM_GENOME_INIT, id, k as u64);
        g.push(t.init_min + (t.init_max - t.init_min) * u);
    }
    let s = a.brain.init_scale;
    for k in 0..a.brain_params() {
        let u = rng::draw(seed, 0, STREAM_GENOME_INIT, id, (a.traits.len() + k) as u64);
        g.push((u * 2.0 - 1.0) * s);
    }
    g
}

/// Bounds of genome scalar k.
pub fn bounds(a: &ArchInfo, k: usize) -> (f64, f64) {
    if k < a.traits.len() {
        (a.traits[k].min, a.traits[k].max)
    } else {
        (-a.brain.weight_bound, a.brain.weight_bound)
    }
}

/// Offspring genome: each scalar mutates with the declared probability by a
/// bounded perturbation scaled to its range, keyed by the child's id.
pub fn mutate(a: &ArchInfo, parent: &[f64], seed: u64, tick: u64, child_id: u64) -> Vec<f64> {
    let mut g = parent.to_vec();
    let p = a.mutation.probability;
    for (k, v) in g.iter_mut().enumerate() {
        if rng::draw(seed, tick, STREAM_MUTATION, child_id, k as u64) < p {
            let (lo, hi) = bounds(a, k);
            let d = rng::normalish(seed, tick, STREAM_MUTATION_SIZE, child_id, k as u64)
                * a.mutation.scale
                * (hi - lo);
            *v = (*v + d).clamp(lo, hi);
        }
    }
    g
}

pub fn genome_valid(a: &ArchInfo, g: &[f64]) -> bool {
    g.len() == a.genome_len()
        && g.iter().enumerate().all(|(k, v)| {
            let (lo, hi) = bounds(a, k);
            v.is_finite() && *v >= lo && *v <= hi
        })
}
