//! A plain feed-forward compressor. Fast enough (syllabic) that quiet
//! consonants come up relative to the loud vowels around them.

use super::{db_to_lin, lin_to_db};

pub struct Compressor {
    threshold_db: f32,
    slope: f32,
    attack: f32,
    release: f32,
    env_db: f32,
}

impl Compressor {
    pub fn new(
        sample_rate: f32,
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
    ) -> Self {
        let coeff = |ms: f32| 1.0 - (-1.0 / (ms * 0.001 * sample_rate).max(1.0)).exp();
        Self {
            threshold_db,
            slope: 1.0 - 1.0 / ratio.max(1.0),
            attack: coeff(attack_ms),
            release: coeff(release_ms),
            env_db: -120.0,
        }
    }

    /// The speech setting used by the renderer.
    pub fn speech(sample_rate: f32) -> Self {
        Self::new(sample_rate, -35.0, 2.5, 3.0, 80.0)
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let level = lin_to_db(x);
        let c = if level > self.env_db {
            self.attack
        } else {
            self.release
        };
        self.env_db += (level - self.env_db) * c;
        let over = self.env_db - self.threshold_db;
        if over > 0.0 {
            x * db_to_lin(-over * self.slope)
        } else {
            x
        }
    }

    pub fn process_block(&mut self, buf: &mut [f32]) {
        for s in buf {
            *s = self.process(*s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::sine;

    fn settled_peak(amp: f32) -> f32 {
        let sr = 48_000.0;
        let mut c = Compressor::new(sr, -30.0, 3.0, 3.0, 80.0);
        let mut x = sine(1000.0, sr, amp, 48_000);
        c.process_block(&mut x);
        x[24_000..].iter().fold(0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn narrows_the_gap_between_loud_and_quiet() {
        let quiet = settled_peak(db_to_lin(-40.0));
        let loud = settled_peak(db_to_lin(-10.0));
        // Below threshold: untouched.
        assert!((lin_to_db(quiet) + 40.0).abs() < 0.5);
        // 30 dB apart going in, clearly less coming out, and never louder.
        let gap = lin_to_db(loud) - lin_to_db(quiet);
        assert!(gap < 22.0 && gap > 8.0, "gap {gap}");
        assert!(loud <= db_to_lin(-10.0));
    }
}
