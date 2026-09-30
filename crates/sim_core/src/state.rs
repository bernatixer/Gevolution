//! Authoritative world state: structure-of-arrays cell fields and entity tables.

use crate::rng::Fnv;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// Randomly initialized genome.
    Random,
    /// Authored starter genome: a labeled control fixture, not an evolved result.
    Authored,
    /// Born in the running world.
    Born,
    /// Added by a player intervention.
    Intervention,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lineage {
    pub parent: u64,
    pub root: u64,
    pub generation: u32,
    pub birth_tick: u64,
    pub birth_x: f64,
    pub birth_z: f64,
    pub origin: Origin,
    pub offspring: u32,
    pub mutation_version: u16,
}

/// Dense entity storage for one archetype, sorted by stable id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entities {
    pub ids: Vec<u64>,
    pub x: Vec<f64>,
    pub z: Vec<f64>,
    pub heading: Vec<f64>,
    pub age: Vec<f64>,
    pub cooldown: Vec<f64>,
    /// One column per declared entity field.
    pub fields: Vec<Vec<f64>>,
    /// Row-major genome matrix: len * genome_len.
    pub genome: Vec<f64>,
    pub genome_len: usize,
    pub lineage: Vec<Lineage>,
}

impl Entities {
    pub fn new(field_count: usize, genome_len: usize) -> Entities {
        Entities {
            fields: vec![vec![]; field_count],
            genome_len,
            ..Default::default()
        }
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn genome_of(&self, i: usize) -> &[f64] {
        &self.genome[i * self.genome_len..(i + 1) * self.genome_len]
    }
    #[allow(clippy::too_many_arguments)]
    pub fn push(&mut self, id: u64, x: f64, z: f64, heading: f64, fields: &[f64], genome: &[f64], lineage: Lineage, cooldown: f64) {
        debug_assert!(self.ids.last().is_none_or(|&l| l < id), "entity ids must stay sorted");
        self.ids.push(id);
        self.x.push(x);
        self.z.push(z);
        self.heading.push(heading);
        self.age.push(0.0);
        self.cooldown.push(cooldown);
        for (col, v) in self.fields.iter_mut().zip(fields) {
            col.push(*v);
        }
        self.genome.extend_from_slice(genome);
        self.lineage.push(lineage);
    }
    /// Keep rows where `keep[i]`; preserves id order.
    pub fn retain(&mut self, keep: &[bool]) {
        fn filt<T: Clone>(v: &mut Vec<T>, keep: &[bool]) {
            let mut i = 0;
            v.retain(|_| {
                let k = keep[i];
                i += 1;
                k
            });
        }
        filt(&mut self.ids, keep);
        filt(&mut self.x, keep);
        filt(&mut self.z, keep);
        filt(&mut self.heading, keep);
        filt(&mut self.age, keep);
        filt(&mut self.cooldown, keep);
        for c in &mut self.fields {
            filt(c, keep);
        }
        filt(&mut self.lineage, keep);
        let g = self.genome_len;
        let mut out = Vec::with_capacity(self.genome.len());
        for (i, k) in keep.iter().enumerate() {
            if *k {
                out.extend_from_slice(&self.genome[i * g..(i + 1) * g]);
            }
        }
        self.genome = out;
    }
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.ids.binary_search(&id).ok()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub tick: u64,
    /// One column per schema cell field, by index.
    pub cells: Vec<Vec<f64>>,
    /// Region coverage masks in [0, 1].
    pub regions: Vec<Vec<f64>>,
    pub entities: Vec<Entities>,
    /// Current parameter values, by plan parameter index.
    pub params: Vec<f64>,
    /// Cumulative accepted amounts by external account.
    pub account_in: Vec<f64>,
    pub account_out: Vec<f64>,
    /// Cumulative explicit interventions and bounded roundoff corrections, by resource.
    pub intervention_in: Vec<f64>,
    pub intervention_out: Vec<f64>,
    pub roundoff: Vec<f64>,
    pub next_entity_id: u64,
}

impl State {
    /// Stable hash of all authoritative state bits.
    pub fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.write_u64(self.tick);
        for col in self.cells.iter().chain(&self.regions) {
            h.write_u64(col.len() as u64);
            for v in col {
                h.write_f64(*v);
            }
        }
        for e in &self.entities {
            h.write_u64(e.len() as u64);
            for i in 0..e.len() {
                h.write_u64(e.ids[i]);
                for v in [e.x[i], e.z[i], e.heading[i], e.age[i], e.cooldown[i]] {
                    h.write_f64(v);
                }
                for c in &e.fields {
                    h.write_f64(c[i]);
                }
                let l = &e.lineage[i];
                h.write_u64(l.parent);
                h.write_u64(l.offspring as u64);
            }
            for v in &e.genome {
                h.write_f64(*v);
            }
        }
        for v in self.params.iter().chain(&self.account_in).chain(&self.account_out) {
            h.write_f64(*v);
        }
        for v in self.intervention_in.iter().chain(&self.intervention_out).chain(&self.roundoff) {
            h.write_f64(*v);
        }
        h.write_u64(self.next_entity_id);
        h.finish()
    }
}

/// Neumaier compensated summation in a fixed order.
#[derive(Clone, Copy, Default, Debug)]
pub struct KahanSum {
    sum: f64,
    c: f64,
}

impl KahanSum {
    #[inline]
    pub fn add(&mut self, v: f64) {
        let t = self.sum + v;
        if self.sum.abs() >= v.abs() {
            self.c += (self.sum - t) + v;
        } else {
            self.c += (v - t) + self.sum;
        }
        self.sum = t;
    }
    pub fn value(&self) -> f64 {
        self.sum + self.c
    }
}

pub fn compensated_sum(v: &[f64]) -> f64 {
    let mut k = KahanSum::default();
    for x in v {
        k.add(*x);
    }
    k.value()
}
