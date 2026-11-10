//! The output limiter. This is the safety stage: every sample that reaches
//! the listener passes through one of these, and its output never exceeds
//! the ceiling.

use super::db_to_lin;

pub struct Limiter {
    ceiling: f32,
    release: f32,
    gain: f32,
}

impl Limiter {
    /// `ceiling_db` is in dBFS (0 = digital full scale) and is clamped to 0.
    pub fn new(sample_rate: f32, ceiling_db: f32, release_ms: f32) -> Self {
        let release_samples = (release_ms * 0.001 * sample_rate).max(1.0);
        Self {
            ceiling: db_to_lin(ceiling_db.min(0.0)),
            release: 1.0 - (-1.0 / release_samples).exp(),
            gain: 1.0,
        }
    }

    /// Gain drops instantly when a sample would exceed the ceiling and
    /// recovers slowly; the final clamp is the guarantee.
    pub fn process(&mut self, x: f32) -> f32 {
        if !x.is_finite() {
            return 0.0;
        }
        let a = x.abs();
        let needed = if a > self.ceiling {
            self.ceiling / a
        } else {
            1.0
        };
        let recovered = self.gain + (1.0 - self.gain) * self.release;
        self.gain = recovered.min(needed);
        (x * self.gain).clamp(-self.ceiling, self.ceiling)
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

    #[test]
    fn never_exceeds_ceiling() {
        let sr = 48_000.0;
        let ceiling = db_to_lin(-6.0);
        let mut lim = Limiter::new(sr, -6.0, 50.0);
        // Quiet, then a +20 dB over-full-scale burst, then spikes and garbage.
        let mut x = sine(1000.0, sr, 0.1, 4800);
        x.extend(sine(3000.0, sr, 10.0, 4800));
        x.extend([
            100.0,
            -100.0,
            f32::INFINITY,
            f32::NAN,
            f32::NEG_INFINITY,
            0.3,
        ]);
        for s in x {
            let y = lim.process(s);
            assert!(y.is_finite());
            assert!(y.abs() <= ceiling + 1e-6, "{y} exceeds ceiling {ceiling}");
        }
    }

    #[test]
    fn leaves_quiet_signal_alone() {
        let sr = 48_000.0;
        let mut lim = Limiter::new(sr, -3.0, 50.0);
        for s in sine(1000.0, sr, 0.25, 4800) {
            assert_eq!(lim.process(s), s);
        }
    }

    #[test]
    fn a_positive_ceiling_is_treated_as_full_scale() {
        let mut lim = Limiter::new(48_000.0, 12.0, 50.0);
        assert!(lim.process(5.0) <= 1.0);
    }
}
