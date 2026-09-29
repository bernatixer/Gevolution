//! Versioned keyed randomness and stable hashing.
//!
//! A draw is a pure function of (seed, tick, stream, element, index), so
//! scheduling, inspection, or unrelated effects never advance another stream.

pub const RNG_VERSION: &str = "splitmix64-keyed.v1";

#[inline]
pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
pub fn key(seed: u64, tick: u64, stream: u64, element: u64, index: u64) -> u64 {
    let mut h = mix(seed ^ 0x5EED);
    h = mix(h ^ tick);
    h = mix(h ^ stream);
    h = mix(h ^ element);
    mix(h ^ index)
}

/// Uniform in [0, 1) with 53 bits of precision.
#[inline]
pub fn unit(h: u64) -> f64 {
    (h >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

#[inline]
pub fn draw(seed: u64, tick: u64, stream: u64, element: u64, index: u64) -> f64 {
    unit(key(seed, tick, stream, element, index))
}

/// Approximately normal (Irwin-Hall with 4 terms, scaled to unit variance), bounded in [-2*sqrt(3), 2*sqrt(3)].
pub fn normalish(seed: u64, tick: u64, stream: u64, element: u64, index: u64) -> f64 {
    let mut s = 0.0;
    for k in 0..4 {
        s += draw(seed, tick, stream, element, index * 4 + k);
    }
    (s - 2.0) * (3.0f64).sqrt()
}

pub struct Fnv(u64);

impl Fnv {
    pub fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    #[inline]
    pub fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        self.write(&v.to_le_bytes());
    }
    #[inline]
    pub fn write_f64(&mut self, v: f64) {
        self.write_u64(v.to_bits());
    }
    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
}

pub fn hash_str(s: &str) -> u64 {
    let mut h = Fnv::new();
    h.write(s.as_bytes());
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyed_draws_are_pure_and_distinct() {
        assert_eq!(draw(1, 2, 3, 4, 0), draw(1, 2, 3, 4, 0));
        assert_ne!(draw(1, 2, 3, 4, 0), draw(1, 2, 3, 5, 0));
        assert_ne!(draw(1, 2, 3, 4, 0), draw(1, 2, 4, 4, 0));
        let mean: f64 = (0..10_000).map(|i| draw(9, 0, 0, i, 0)).sum::<f64>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02);
    }
}
