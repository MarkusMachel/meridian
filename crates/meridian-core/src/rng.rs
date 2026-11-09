//! xoshiro256** seeded with splitmix64. Pure integer math, so the sequence
//! is identical on every platform. State is exposed for hashing and saving.

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn from_seed(seed: u64) -> Rng {
        let mut sm = seed;
        Rng {
            s: [
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
            ],
        }
    }

    pub fn from_state(s: [u64; 4]) -> Rng {
        Rng { s }
    }

    pub fn state(&self) -> [u64; 4] {
        self.s
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform value in `0..bound` using rejection sampling (no modulo bias).
    /// Panics if `bound` is zero.
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "Rng::below(0)");
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let v = self.next_u64();
            if v >= threshold {
                return v % bound;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::from_seed(42);
        let mut b = Rng::from_seed(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_different_sequence() {
        assert_ne!(Rng::from_seed(1).next_u64(), Rng::from_seed(2).next_u64());
    }

    #[test]
    fn golden_values_are_stable() {
        // If this fails, the generator changed and every saved run breaks.
        let mut r = Rng::from_seed(0);
        let got = [r.next_u64(), r.next_u64(), r.next_u64()];
        assert_eq!(got, GOLDEN);
    }
    const GOLDEN: [u64; 3] = [11091344671253066420, 13793997310169335082, 1900383378846508768];

    #[test]
    fn below_in_range_and_state_round_trips() {
        let mut r = Rng::from_seed(7);
        for _ in 0..1000 {
            assert!(r.below(10) < 10);
        }
        let mut copy = Rng::from_state(r.state());
        assert_eq!(r.next_u64(), copy.next_u64());
    }
}
