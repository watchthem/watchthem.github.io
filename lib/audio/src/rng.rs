//! Tiny self-contained PRNG for audio synthesis (the noise oscillator, and later the
//! music sequencer's controlled randomness). Deliberately independent of
//! `macroquad::rand`/quad-rand's global generator: that generator is seeded per-episode
//! from `HCG_SEED`/`Control::seed()` for deterministic gameplay replay (see root
//! CLAUDE.md's "Native CLI flags" section), and drawing from it here to render audio
//! would shift every subsequent gameplay draw, breaking `--once`/`HCG_SEED`
//! reproducibility for no audible benefit — nothing about gameplay depends on what an
//! audio RNG generates.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    /// SplitMix64 — small and fast, good enough for noise/jitter, not cryptographic.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0.0, 1.0)`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.next_f32() * (hi - lo)
    }

    /// Uniform index in `0..len`. Panics if `len == 0`.
    pub fn index(&mut self, len: usize) -> usize {
        (self.next_f32() * len as f32) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_diverges() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn next_f32_in_unit_range() {
        let mut r = Rng::new(7);
        for _ in 0..1000 {
            let v = r.next_f32();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn index_in_bounds() {
        let mut r = Rng::new(9);
        for _ in 0..1000 {
            assert!(r.index(5) < 5);
        }
    }
}
