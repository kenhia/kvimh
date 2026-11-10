//! The processing core. Nothing in here does I/O.

pub mod compressor;
pub mod limiter;
pub mod spectral;
pub mod tone;

/// Decibels to a linear amplitude factor.
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Linear amplitude to decibels, floored at -180 dB so silence stays finite.
pub fn lin_to_db(x: f32) -> f32 {
    20.0 * x.abs().max(1e-9).log10()
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::f32::consts::TAU;

    pub fn sine(hz: f32, sr: f32, amp: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| amp * (TAU * hz * i as f32 / sr).sin())
            .collect()
    }

    /// Amplitude of the `hz` component of `x` (Goertzel-style correlation).
    pub fn amplitude_at(x: &[f32], hz: f32, sr: f32) -> f32 {
        let (mut re, mut im) = (0f64, 0f64);
        for (i, &v) in x.iter().enumerate() {
            let ph = std::f64::consts::TAU * hz as f64 * i as f64 / sr as f64;
            re += v as f64 * ph.cos();
            im += v as f64 * ph.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    /// Frequency in `lo..hi` (stepped by `step`) where `x` has most energy.
    pub fn peak_hz(x: &[f32], sr: f32, lo: f32, hi: f32, step: f32) -> f32 {
        let mut best = (lo, 0f32);
        let mut f = lo;
        while f <= hi {
            let a = amplitude_at(x, f, sr);
            if a > best.1 {
                best = (f, a);
            }
            f += step;
        }
        best.0
    }
}
