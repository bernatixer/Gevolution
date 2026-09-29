//! Prescribed, seeded external forcing (the initial weather model).
//!
//! Weather is an external process: it supplies forcing values each tick and
//! does not claim to simulate a closed water cycle.

use crate::compiler::Schema;
use crate::rng;
use crate::schema::{Signal, WeatherDecl};

pub struct Weather {
    /// Per forcing index: the signal and whether it is a cell field.
    signals: Vec<Option<(Signal, bool, u64)>>,
}

impl Weather {
    pub fn new(decl: &WeatherDecl, schema: &Schema) -> Result<Weather, String> {
        for k in decl.signals.keys() {
            if schema.forcing(k).is_none() {
                return Err(format!(
                    "weather signal {k} does not match a declared forcing"
                ));
            }
        }
        let signals = schema
            .forcings
            .iter()
            .map(|f| {
                decl.signals
                    .get(&f.id)
                    .map(|s| (s.clone(), f.cells, rng::hash_str(&f.id)))
            })
            .collect();
        Ok(Weather { signals })
    }

    pub fn missing(&self, schema: &Schema) -> Vec<String> {
        schema
            .forcings
            .iter()
            .zip(&self.signals)
            .filter(|(_, s)| s.is_none())
            .map(|(f, _)| f.id.clone())
            .collect()
    }

    /// Uniform value of a signal at simulated time t.
    pub fn uniform(sig: &Signal, seed: u64, stream: u64, t: f64) -> f64 {
        let mut v = sig.mean;
        if sig.amplitude != 0.0 && sig.period > 0.0 {
            v += sig.amplitude * (std::f64::consts::TAU * t / sig.period + sig.phase).sin();
        }
        if sig.noise != 0.0 && sig.noise_interval > 0.0 {
            let u = t / sig.noise_interval;
            let k = u.floor();
            let f = u - k;
            let a = rng::draw(seed, k as u64, stream, 0, 0) * 2.0 - 1.0;
            let b = rng::draw(seed, k as u64 + 1, stream, 0, 0) * 2.0 - 1.0;
            let s = f * f * (3.0 - 2.0 * f);
            v += sig.noise * (a + (b - a) * s);
        }
        if let Some(lo) = sig.min {
            v = v.max(lo);
        }
        if let Some(hi) = sig.max {
            v = v.min(hi);
        }
        v
    }

    /// Fill forcing buffers for simulated time t.
    pub fn fill(&self, out: &mut Vec<Vec<f64>>, seed: u64, t: f64, elevation_norm: &[f64]) {
        out.resize_with(self.signals.len(), Vec::new);
        for (i, s) in self.signals.iter().enumerate() {
            let buf = &mut out[i];
            match s {
                None => {
                    buf.clear();
                    buf.push(0.0);
                }
                Some((sig, false, stream)) => {
                    buf.clear();
                    buf.push(Self::uniform(sig, seed, *stream, t));
                }
                Some((sig, true, stream)) => {
                    let u = Self::uniform(sig, seed, *stream, t);
                    buf.resize(elevation_norm.len(), 0.0);
                    for (b, e) in buf.iter_mut().zip(elevation_norm) {
                        let mut v = u + sig.elevation_coefficient * e * sig.mean;
                        if let Some(lo) = sig.min {
                            v = v.max(lo);
                        }
                        if let Some(hi) = sig.max {
                            v = v.min(hi);
                        }
                        *b = v;
                    }
                }
            }
        }
    }
}
