//! Offline rendering of a clip through the processing chain:
//! frequency lowering + EQ → compressor → loudness match → limiter.

use serde::{Deserialize, Serialize};

use crate::dsp::compressor::Compressor;
use crate::dsp::limiter::Limiter;
use crate::dsp::spectral::{Lowering, Spectral};
use crate::profile::{Ear, Profile};

/// Nothing rendered here ever exceeds this, in dBFS.
pub const CEILING_DB: f32 = -3.0;

/// A stereo clip. Both channels are always the same length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    pub sr: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Clip {
    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    pub fn seconds(&self) -> f32 {
        self.len() as f32 / self.sr.max(1) as f32
    }

    fn rms(&self) -> f32 {
        let n = (self.left.len() + self.right.len()).max(1) as f64;
        let sum: f64 = self
            .left
            .iter()
            .chain(&self.right)
            .map(|v| (*v as f64).powi(2))
            .sum();
        (sum / n).sqrt() as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub lowering: Lowering,
    /// Fraction of the estimated loss given back as EQ gain (0 = no EQ).
    pub eq_strength: f32,
    pub max_eq_gain_db: f32,
    pub compress: bool,
}

impl Settings {
    /// No processing at all: the original through the limiter.
    pub const BYPASS: Settings = Settings {
        lowering: Lowering::Off,
        eq_strength: 0.0,
        max_eq_gain_db: 20.0,
        compress: false,
    };

    pub fn is_bypass(&self) -> bool {
        self.lowering == Lowering::Off && self.eq_strength <= 0.0 && !self.compress
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lowering: Lowering::Compress {
                cutoff_hz: 2_000.0,
                ratio: 2.0,
            },
            eq_strength: 0.5,
            max_eq_gain_db: 20.0,
            compress: true,
        }
    }
}

/// Render `clip` with `settings`. Without a profile the EQ is flat. The
/// result is matched to the original's loudness, so comparing renderings is
/// a comparison of clarity and not of volume.
pub fn render(clip: &Clip, settings: &Settings, profile: Option<&Profile>) -> Clip {
    let sr = clip.sr as f32;
    let mut out = clip.clone();
    if !settings.is_bypass() {
        for (ear, chan) in [(Ear::Left, &mut out.left), (Ear::Right, &mut out.right)] {
            let mut spectral = Spectral::new(sr);
            spectral.set_lowering(settings.lowering);
            if let (Some(p), true) = (profile, settings.eq_strength > 0.0) {
                let points = p.eq_points(ear, settings.eq_strength, settings.max_eq_gain_db);
                spectral.set_gain_curve(&points);
            }
            *chan = spectral.process_all(chan);
            if settings.compress {
                Compressor::speech(sr).process_block(chan);
            }
        }
        // Match loudness, but never add more than 12 dB (a near-silent clip
        // must not be turned into noise).
        let (before, after) = (clip.rms(), out.rms());
        if after > 1e-6 {
            let g = (before / after).min(4.0);
            out.left
                .iter_mut()
                .chain(out.right.iter_mut())
                .for_each(|v| *v *= g);
        }
    }
    Limiter::new(sr, CEILING_DB, 80.0).process_block(&mut out.left);
    Limiter::new(sr, CEILING_DB, 80.0).process_block(&mut out.right);
    out
}

/// The renderings compared in the blind test.
pub fn presets(profile: Option<&Profile>) -> Vec<(&'static str, Settings)> {
    let auto = profile.map(|p| p.suggest_compress());
    let (cutoff, ratio) = match auto {
        Some(Lowering::Compress { cutoff_hz, ratio }) => (cutoff_hz, ratio),
        _ => (2_000.0, 2.0),
    };
    let base = Settings::default();
    let with = |lowering| Settings { lowering, ..base };
    vec![
        ("Original", Settings::BYPASS),
        ("EQ and compression only", with(Lowering::Off)),
        (
            "Gentle frequency compression",
            with(Lowering::Compress {
                cutoff_hz: (cutoff * 1.3).min(4_000.0),
                ratio: 1.0 + (ratio - 1.0) * 0.5,
            }),
        ),
        (
            "Strong frequency compression",
            with(Lowering::Compress {
                cutoff_hz: cutoff,
                ratio,
            }),
        ),
        (
            "Transposition (octave down)",
            with(Lowering::Transpose {
                src_lo_hz: 4_000.0,
                src_hi_hz: 9_000.0,
                factor: 0.5,
                mix_db: 0.0,
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::db_to_lin;
    use crate::dsp::testutil::{amplitude_at, sine};

    fn clip(amp: f32) -> Clip {
        let sr = 48_000;
        let x: Vec<f32> = sine(300.0, sr as f32, amp, 48_000)
            .iter()
            .zip(sine(6_000.0, sr as f32, amp * 0.3, 48_000))
            .map(|(a, b)| a + b)
            .collect();
        Clip {
            sr,
            left: x.clone(),
            right: x,
        }
    }

    #[test]
    fn bypass_of_a_quiet_clip_is_the_clip() {
        let c = clip(0.2);
        assert_eq!(render(&c, &Settings::BYPASS, None), c);
    }

    #[test]
    fn every_preset_stays_under_the_ceiling_even_for_a_hot_clip() {
        let hot = clip(3.0);
        let ceiling = db_to_lin(CEILING_DB) + 1e-6;
        for (name, s) in presets(None) {
            let out = render(&hot, &s, None);
            assert_eq!(out.len(), hot.len(), "{name}");
            let peak = out
                .left
                .iter()
                .chain(&out.right)
                .fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak <= ceiling, "{name} peaked at {peak}");
        }
    }

    #[test]
    fn processed_renderings_are_loudness_matched_to_the_original() {
        let c = clip(0.1);
        for (name, s) in presets(None) {
            let out = render(&c, &s, None);
            let ratio = out.rms() / c.rms();
            assert!((0.9..1.1).contains(&ratio), "{name} rms ratio {ratio}");
        }
    }

    #[test]
    fn lowering_preset_moves_the_6k_tone_down() {
        let c = clip(0.1);
        let s = Settings {
            compress: false,
            eq_strength: 0.0,
            ..Settings::default()
        };
        let out = render(&c, &s, None);
        let body = &out.left[9_600..38_400];
        assert!(amplitude_at(body, 6_000.0, 48_000.0) < 0.001);
        assert!(amplitude_at(body, s.lowering.map_hz(6_000.0), 48_000.0) > 0.01);
    }
}
