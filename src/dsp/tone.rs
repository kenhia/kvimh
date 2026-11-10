//! Pulsed test tones for the hearing check. A pulsed tone is easier to tell
//! apart from steady tinnitus than a continuous one.

use std::f32::consts::{PI, TAU};

const ON_MS: f32 = 240.0;
const OFF_MS: f32 = 160.0;
const RAMP_MS: f32 = 25.0;

/// Unit-amplitude pulsed sine. The caller applies the level.
pub struct PulsedTone {
    step: f32,
    phase: f32,
    pos: usize,
    on: usize,
    period: usize,
    ramp: usize,
}

impl PulsedTone {
    pub fn new(hz: f32, sample_rate: f32) -> Self {
        let n = |ms: f32| (ms * 0.001 * sample_rate) as usize;
        Self {
            step: TAU * hz / sample_rate,
            phase: 0.0,
            pos: 0,
            on: n(ON_MS),
            period: n(ON_MS + OFF_MS),
            ramp: n(RAMP_MS).max(1),
        }
    }

    /// Seconds for one on/off cycle.
    pub fn period_secs() -> f32 {
        (ON_MS + OFF_MS) * 0.001
    }

    fn envelope(&self) -> f32 {
        let p = self.pos;
        if p >= self.on {
            0.0
        } else if p < self.ramp {
            0.5 - 0.5 * (PI * p as f32 / self.ramp as f32).cos()
        } else if p >= self.on - self.ramp {
            0.5 - 0.5 * (PI * (self.on - p) as f32 / self.ramp as f32).cos()
        } else {
            1.0
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        let y = self.envelope() * self.phase.sin();
        self.phase = (self.phase + self.step) % TAU;
        self.pos = (self.pos + 1) % self.period;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulses_start_and_end_silent_and_stay_in_range() {
        let sr = 48_000.0;
        let mut t = PulsedTone::new(1000.0, sr);
        let period = (PulsedTone::period_secs() * sr) as usize;
        let x: Vec<f32> = (0..period * 3).map(|_| t.next_sample()).collect();
        assert!(x.iter().all(|v| v.abs() <= 1.0));
        // No click: the first sample of each pulse is ~0 and the gap is silent.
        assert!(x[0].abs() < 1e-3 && x[period].abs() < 1e-3);
        assert!(x[period - 100..period].iter().all(|v| *v == 0.0));
        // And the middle of a pulse is at full level.
        let mid = &x[4800..5800];
        assert!(mid.iter().fold(0f32, |m, v| m.max(v.abs())) > 0.99);
    }
}
