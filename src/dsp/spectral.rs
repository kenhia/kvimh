//! Frequency lowering and the hearing-profile EQ, done together in one
//! short-time Fourier transform.
//!
//! Each frame is analysed into frequency bins. Bins below the lowering
//! cutoff are passed through bit-for-bit; bins above it are moved to a lower
//! frequency (phase-vocoder style, so a tone stays a tone). A per-bin gain
//! curve — the EQ — is applied on the way out.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use super::db_to_lin;

/// How high frequencies are brought down into the audible range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Lowering {
    Off,
    /// Squeeze everything above `cutoff_hz` toward it: an input at `f` comes
    /// out at `cutoff * (f / cutoff)^(1 / ratio)`. Nothing above the cutoff
    /// stays where it was.
    Compress {
        cutoff_hz: f32,
        ratio: f32,
    },
    /// Copy the band `src_lo_hz..src_hi_hz` down to `f * factor` and mix it
    /// in at `mix_db`. The original spectrum is left in place.
    Transpose {
        src_lo_hz: f32,
        src_hi_hz: f32,
        factor: f32,
        mix_db: f32,
    },
}

impl Lowering {
    /// Where an input frequency ends up, for display.
    pub fn map_hz(&self, hz: f32) -> f32 {
        match *self {
            Lowering::Compress { cutoff_hz, ratio } if hz > cutoff_hz => {
                cutoff_hz * (hz / cutoff_hz).powf(1.0 / ratio.max(1.0))
            }
            Lowering::Transpose {
                src_lo_hz,
                src_hi_hz,
                factor,
                ..
            } if hz >= src_lo_hz && hz <= src_hi_hz => hz * factor,
            _ => hz,
        }
    }
}

/// Frame length for a sample rate: about 21 ms, a power of two.
pub fn frame_len_for(sample_rate: f32) -> usize {
    let mut n = 1024;
    while (n as f32) < sample_rate * 0.02 {
        n *= 2;
    }
    n
}

pub struct Spectral {
    n: usize,
    hop: usize,
    sr: f32,
    fft: Arc<dyn RealToComplex<f32>>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    window: Vec<f32>,
    // streaming state
    in_buf: Vec<f32>,
    ola: Vec<f32>,
    pending: Vec<f32>,
    ready: Vec<f32>,
    idx: usize,
    // per-frame scratch
    time: Vec<f32>,
    spec: Vec<Complex32>,
    out: Vec<Complex32>,
    mag: Vec<f32>,
    phase: Vec<f32>,
    freq: Vec<f32>,
    energy: Vec<f32>,
    best_mag: Vec<f32>,
    best_freq: Vec<f32>,
    best_src: Vec<usize>,
    // phase-vocoder state
    prev_phase: Vec<f32>,
    synth_phase: Vec<f32>,
    prev_out_mag: Vec<f32>,
    // parameters
    lowering: Lowering,
    gain: Vec<f32>,
}

impl Spectral {
    pub fn new(sample_rate: f32) -> Self {
        let n = frame_len_for(sample_rate);
        let hop = n / 4;
        let bins = n / 2 + 1;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let ifft = planner.plan_fft_inverse(n);
        // Periodic Hann: with a hop of n/4, the squared window sums to 1.5.
        let window = (0..n)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos())
            .collect();
        Self {
            n,
            hop,
            sr: sample_rate,
            fft,
            ifft,
            window,
            in_buf: vec![0.0; n],
            ola: vec![0.0; n],
            pending: vec![0.0; hop],
            ready: vec![0.0; hop],
            idx: 0,
            time: vec![0.0; n],
            spec: vec![Complex32::ZERO; bins],
            out: vec![Complex32::ZERO; bins],
            mag: vec![0.0; bins],
            phase: vec![0.0; bins],
            freq: vec![0.0; bins],
            energy: vec![0.0; bins],
            best_mag: vec![0.0; bins],
            best_freq: vec![0.0; bins],
            best_src: vec![0; bins],
            prev_phase: vec![0.0; bins],
            synth_phase: vec![0.0; bins],
            prev_out_mag: vec![0.0; bins],
            lowering: Lowering::Off,
            gain: vec![1.0; bins],
        }
    }

    /// Output sample `i + latency()` corresponds to input sample `i`.
    pub fn latency(&self) -> usize {
        self.n
    }

    pub fn set_lowering(&mut self, lowering: Lowering) {
        self.lowering = lowering;
    }

    /// Set the EQ from `(hz, gain_db)` points, sorted by frequency.
    /// Interpolated on a log-frequency axis; flat beyond the end points.
    pub fn set_gain_curve(&mut self, points: &[(f32, f32)]) {
        let bin_hz = self.sr / self.n as f32;
        for (k, g) in self.gain.iter_mut().enumerate() {
            *g = db_to_lin(interp_log_hz(points, k as f32 * bin_hz));
        }
    }

    /// Process a block. `output` must be the same length as `input`.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        for (x, y) in input.iter().zip(output.iter_mut()) {
            self.pending[self.idx] = *x;
            *y = self.ready[self.idx];
            self.idx += 1;
            if self.idx == self.hop {
                self.idx = 0;
                self.advance();
            }
        }
    }

    /// Process a whole signal offline, with the latency removed.
    pub fn process_all(&mut self, input: &[f32]) -> Vec<f32> {
        let lat = self.latency();
        let mut padded = input.to_vec();
        padded.resize(input.len() + lat, 0.0);
        let mut out = vec![0.0; padded.len()];
        self.process(&padded, &mut out);
        out.drain(..lat);
        out
    }

    fn advance(&mut self) {
        let (n, hop) = (self.n, self.hop);
        self.in_buf.copy_within(hop.., 0);
        self.in_buf[n - hop..].copy_from_slice(&self.pending);

        for i in 0..n {
            self.time[i] = self.in_buf[i] * self.window[i];
        }
        // realfft only fails on wrong buffer lengths, which are fixed here.
        let _ = self.fft.process(&mut self.time, &mut self.spec);

        match self.lowering {
            Lowering::Off => self.out.copy_from_slice(&self.spec),
            Lowering::Compress { cutoff_hz, ratio } => self.compress(cutoff_hz, ratio),
            Lowering::Transpose {
                src_lo_hz,
                src_hi_hz,
                factor,
                mix_db,
            } => self.transpose(src_lo_hz, src_hi_hz, factor, mix_db),
        }

        for (o, g) in self.out.iter_mut().zip(&self.gain) {
            *o *= *g;
        }
        let last = self.out.len() - 1;
        self.out[0].im = 0.0;
        self.out[last].im = 0.0;
        let _ = self.ifft.process(&mut self.out, &mut self.time);

        // 1/n undoes the unnormalised FFT pair, 1/1.5 the squared windows.
        let scale = 1.0 / (n as f32 * 1.5);
        for i in 0..n {
            self.ola[i] += self.time[i] * self.window[i] * scale;
        }
        self.ready.copy_from_slice(&self.ola[..hop]);
        self.ola.copy_within(hop.., 0);
        self.ola[n - hop..].fill(0.0);
    }

    /// Magnitude, phase and true (between-bin) frequency of every bin.
    fn analyse(&mut self) {
        let expected = TAU * self.hop as f32 / self.n as f32;
        let to_bins = self.n as f32 / (TAU * self.hop as f32);
        for k in 0..self.spec.len() {
            let c = self.spec[k];
            let (mag, phase) = (c.norm(), c.arg());
            let mut d = phase - self.prev_phase[k] - k as f32 * expected;
            d -= TAU * (d / TAU).round();
            self.mag[k] = mag;
            self.phase[k] = phase;
            self.freq[k] = k as f32 + d * to_bins;
            self.prev_phase[k] = phase;
        }
        self.energy.fill(0.0);
        self.best_mag.fill(0.0);
    }

    /// Move the energy of input bin `k` to output frequency `f_bins`.
    fn deposit(&mut self, k: usize, f_bins: f32, lowest: usize) {
        let top = self.energy.len() - 1;
        let j = (f_bins.round() as usize).clamp(lowest, top);
        let m = self.mag[k];
        self.energy[j] += m * m;
        if m > self.best_mag[j] {
            self.best_mag[j] = m;
            self.best_freq[j] = f_bins;
            self.best_src[j] = k;
        }
    }

    /// The moved component landing in output bin `j`, or zero.
    fn synthesise(&mut self, j: usize) -> Complex32 {
        if self.energy[j] <= 0.0 {
            self.prev_out_mag[j] = 0.0;
            return Complex32::ZERO;
        }
        let m = self.energy[j].sqrt();
        if self.prev_out_mag[j] < 0.1 * m {
            // An onset: take the source's phase so the attack stays sharp.
            self.synth_phase[j] = self.phase[self.best_src[j]];
        } else {
            let advance = TAU * self.hop as f32 * self.best_freq[j] / self.n as f32;
            self.synth_phase[j] = wrap(self.synth_phase[j] + advance);
        }
        self.prev_out_mag[j] = m;
        Complex32::from_polar(m, self.synth_phase[j])
    }

    fn compress(&mut self, cutoff_hz: f32, ratio: f32) {
        self.analyse();
        let bins = self.spec.len();
        let kc = (cutoff_hz * self.n as f32 / self.sr).clamp(1.0, (bins - 1) as f32);
        let k0 = kc.ceil() as usize;
        let inv = 1.0 / ratio.max(1.0);
        self.out[..k0].copy_from_slice(&self.spec[..k0]);
        for k in k0..bins {
            let f = self.freq[k].max(kc);
            self.deposit(k, kc * (f / kc).powf(inv), k0);
        }
        for j in k0..bins {
            self.out[j] = self.synthesise(j);
        }
    }

    fn transpose(&mut self, lo_hz: f32, hi_hz: f32, factor: f32, mix_db: f32) {
        self.analyse();
        let bins = self.spec.len();
        let to_bin = self.n as f32 / self.sr;
        let lo = ((lo_hz * to_bin).ceil() as usize).min(bins - 1);
        let hi = ((hi_hz * to_bin).floor() as usize).min(bins - 1);
        let mix = db_to_lin(mix_db);
        self.out.copy_from_slice(&self.spec);
        for k in lo..=hi {
            let f = self.freq[k] * factor;
            self.deposit(k, f, 1);
        }
        for j in 1..bins {
            let moved = self.synthesise(j);
            self.out[j] += moved * mix;
        }
    }
}

fn wrap(phase: f32) -> f32 {
    phase - TAU * ((phase + PI) / TAU).floor()
}

/// Linear interpolation of `(hz, value)` points on a log-frequency axis.
pub fn interp_log_hz(points: &[(f32, f32)], hz: f32) -> f32 {
    let Some(first) = points.first() else {
        return 0.0;
    };
    let last = points[points.len() - 1];
    if hz <= first.0 {
        return first.1;
    }
    if hz >= last.0 {
        return last.1;
    }
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if hz <= b.0 {
            let t = (hz / a.0).ln() / (b.0 / a.0).ln();
            return a.1 + (b.1 - a.1) * t;
        }
    }
    last.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::lin_to_db;
    use crate::dsp::testutil::{amplitude_at, peak_hz, sine};

    const SR: f32 = 48_000.0;

    fn settled(x: &[f32]) -> &[f32] {
        &x[9_600..x.len() - 9_600]
    }

    #[test]
    fn off_is_an_identity_after_the_latency() {
        let x: Vec<f32> = sine(440.0, SR, 0.3, 48_000)
            .iter()
            .zip(sine(5_300.0, SR, 0.2, 48_000))
            .map(|(a, b)| a + b)
            .collect();
        let y = Spectral::new(SR).process_all(&x);
        assert_eq!(y.len(), x.len());
        for i in 2_048..46_000 {
            assert!(
                (x[i] - y[i]).abs() < 1e-3,
                "sample {i}: {} vs {}",
                x[i],
                y[i]
            );
        }
    }

    #[test]
    fn compress_moves_a_high_tone_to_the_predicted_frequency() {
        let lowering = Lowering::Compress {
            cutoff_hz: 2_000.0,
            ratio: 2.0,
        };
        let mut s = Spectral::new(SR);
        s.set_lowering(lowering);
        let y = s.process_all(&sine(6_000.0, SR, 0.3, 48_000));
        let y = settled(&y);

        let want = lowering.map_hz(6_000.0); // 2000 * sqrt(3) = 3464 Hz
        assert!((want - 3_464.1).abs() < 1.0);
        let got = peak_hz(y, SR, 1_000.0, 8_000.0, 5.0);
        assert!(
            (got - want).abs() < 30.0,
            "tone landed at {got}, wanted {want}"
        );
        // Nothing is left where it started, and the level is about the same.
        assert!(lin_to_db(amplitude_at(y, 6_000.0, SR)) < -50.0);
        // RMS, not a correlation: the tone is not on the search grid.
        let rms = (y.iter().map(|v| v * v).sum::<f32>() / y.len() as f32).sqrt();
        let level = lin_to_db(rms * 2f32.sqrt()) - lin_to_db(0.3);
        assert!(level.abs() < 3.0, "level changed by {level} dB");
    }

    #[test]
    fn compress_leaves_content_below_the_cutoff_untouched() {
        let x: Vec<f32> = sine(1_000.0, SR, 0.3, 48_000)
            .iter()
            .zip(sine(6_000.0, SR, 0.2, 48_000))
            .map(|(a, b)| a + b)
            .collect();
        let mut s = Spectral::new(SR);
        s.set_lowering(Lowering::Compress {
            cutoff_hz: 2_000.0,
            ratio: 2.0,
        });
        let y = s.process_all(&x);
        let a = amplitude_at(settled(&y), 1_000.0, SR);
        assert!((a - 0.3).abs() < 0.003, "1 kHz came out at {a}");
    }

    #[test]
    fn transpose_adds_an_octave_down_copy_and_keeps_the_original() {
        let mut s = Spectral::new(SR);
        s.set_lowering(Lowering::Transpose {
            src_lo_hz: 4_000.0,
            src_hi_hz: 8_000.0,
            factor: 0.5,
            mix_db: 0.0,
        });
        let y = s.process_all(&sine(6_000.0, SR, 0.3, 48_000));
        let y = settled(&y);
        assert!((amplitude_at(y, 6_000.0, SR) - 0.3).abs() < 0.01);
        let got = peak_hz(y, SR, 2_000.0, 4_000.0, 5.0);
        assert!((got - 3_000.0).abs() < 30.0, "copy landed at {got}");
        // Original (0.3) plus a copy at about the same level: ~3 dB more power.
        let rms = (y.iter().map(|v| v * v).sum::<f32>() / y.len() as f32).sqrt();
        let total = lin_to_db(rms * 2f32.sqrt()) - lin_to_db(0.3);
        assert!(
            (1.0..5.0).contains(&total),
            "total level changed by {total} dB"
        );
    }

    #[test]
    fn gain_curve_boosts_only_where_it_says() {
        let mut s = Spectral::new(SR);
        s.set_gain_curve(&[
            (1_000.0, 0.0),
            (2_000.0, 0.0),
            (4_000.0, 12.0),
            (8_000.0, 12.0),
        ]);
        let lo = s.process_all(&sine(500.0, SR, 0.1, 48_000));
        let mut s2 = Spectral::new(SR);
        s2.set_gain_curve(&[
            (1_000.0, 0.0),
            (2_000.0, 0.0),
            (4_000.0, 12.0),
            (8_000.0, 12.0),
        ]);
        let hi = s2.process_all(&sine(6_000.0, SR, 0.1, 48_000));
        assert!((amplitude_at(settled(&lo), 500.0, SR) - 0.1).abs() < 0.002);
        let boost = lin_to_db(amplitude_at(settled(&hi), 6_000.0, SR)) - lin_to_db(0.1);
        assert!((boost - 12.0).abs() < 0.3, "boost was {boost} dB");
    }

    #[test]
    fn interpolation_is_flat_past_the_ends_and_log_between() {
        let pts = [(1_000.0, 0.0), (4_000.0, 20.0)];
        assert_eq!(interp_log_hz(&pts, 100.0), 0.0);
        assert_eq!(interp_log_hz(&pts, 9_000.0), 20.0);
        assert!((interp_log_hz(&pts, 2_000.0) - 10.0).abs() < 1e-4);
        assert_eq!(interp_log_hz(&[], 500.0), 0.0);
    }

    #[test]
    fn streaming_in_odd_block_sizes_matches_one_big_block() {
        let x = sine(3_000.0, SR, 0.2, 10_000);
        let mut a = Spectral::new(SR);
        a.set_lowering(Lowering::Compress {
            cutoff_hz: 1_500.0,
            ratio: 2.0,
        });
        let mut whole = vec![0.0; x.len()];
        a.process(&x, &mut whole);

        let mut b = Spectral::new(SR);
        b.set_lowering(Lowering::Compress {
            cutoff_hz: 1_500.0,
            ratio: 2.0,
        });
        let mut pieces = vec![0.0; x.len()];
        for (i, o) in x.chunks(37).zip(pieces.chunks_mut(37)) {
            b.process(i, o);
        }
        assert_eq!(whole, pieces);
    }
}
