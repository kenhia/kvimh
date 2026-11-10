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

    pub fn rms(&self) -> f32 {
        let n = (self.left.len() + self.right.len()).max(1) as f64;
        let sum: f64 = self
            .left
            .iter()
            .chain(&self.right)
            .map(|v| (*v as f64).powi(2))
            .sum();
        (sum / n).sqrt() as f32
    }

    pub fn scale(&mut self, gain: f32) {
        self.left
            .iter_mut()
            .chain(self.right.iter_mut())
            .for_each(|v| *v *= gain);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub lowering: Lowering,
    /// Fraction of the estimated loss given back as EQ gain (0 = no EQ).
    pub eq_strength: f32,
    pub max_eq_gain_db: f32,
    pub compress: bool,
    /// Leave the band around the matched tinnitus pitch un-boosted.
    #[serde(default)]
    pub tinnitus_guard: bool,
}

impl Settings {
    /// No processing at all: the original through the limiter.
    pub const BYPASS: Settings = Settings {
        lowering: Lowering::Off,
        eq_strength: 0.0,
        max_eq_gain_db: 20.0,
        compress: false,
        tinnitus_guard: false,
    };

    pub fn is_bypass(&self) -> bool {
        self.lowering == Lowering::Off && self.eq_strength <= 0.0 && !self.compress
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lowering: Lowering::Off,
            eq_strength: 0.5,
            max_eq_gain_db: 20.0,
            compress: true,
            tinnitus_guard: false,
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
                let points = p.eq_points(
                    ear,
                    settings.eq_strength,
                    settings.max_eq_gain_db,
                    settings.tinnitus_guard,
                );
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
            out.scale((before / after).min(4.0));
        }
    }
    Limiter::new(sr, CEILING_DB, 80.0).process_block(&mut out.left);
    Limiter::new(sr, CEILING_DB, 80.0).process_block(&mut out.right);
    out
}

/// The renderings compared in the blind test and the word test, built from
/// the shape of the measured hearing:
///
/// - the original;
/// - EQ at the usual half of the estimated loss, and at a fuller 80 %;
/// - if a tinnitus pitch was matched, the half-gain EQ with the band around
///   that pitch left alone — otherwise frequency compression;
/// - the notch band copied down to where hearing is clear (or, with no
///   notch, the top octave copied down).
pub fn presets(profile: Option<&Profile>) -> Vec<(&'static str, Settings)> {
    let half = Settings::default();
    let has_tinnitus =
        profile.is_some_and(|p| p.tinnitus_left.is_some() || p.tinnitus_right.is_some());
    let fourth = if has_tinnitus {
        (
            "EQ, sparing the tinnitus pitch",
            Settings {
                tinnitus_guard: true,
                ..half
            },
        )
    } else {
        let lowering = profile
            .map(|p| p.suggest_compress())
            .unwrap_or(Lowering::Compress {
                cutoff_hz: 2_000.0,
                ratio: 2.0,
            });
        ("Frequency compression", Settings { lowering, ..half })
    };
    let copy = profile.and_then(|p| p.suggest_notch_copy());
    let fifth = match copy {
        Some(lowering) => ("Notch band copied down", Settings { lowering, ..half }),
        None => (
            "Highs copied an octave down",
            Settings {
                lowering: Lowering::Transpose {
                    src_lo_hz: 4_000.0,
                    src_hi_hz: 9_000.0,
                    factor: 0.5,
                    mix_db: 0.0,
                },
                ..half
            },
        ),
    };
    vec![
        ("Original", Settings::BYPASS),
        ("EQ, half gain", half),
        (
            "EQ, fuller gain",
            Settings {
                eq_strength: 0.8,
                max_eq_gain_db: 25.0,
                ..half
            },
        ),
        fourth,
        fifth,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::db_to_lin;
    use crate::dsp::testutil::{amplitude_at, sine};
    use crate::profile::{Threshold, Tinnitus};

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

    /// Good hearing except a dip at 3 and 4 kHz.
    fn notched_profile() -> Profile {
        let ear: Vec<Threshold> = crate::profile::TEST_FREQS
            .iter()
            .map(|&hz| Threshold {
                hz,
                dbfs: Some(if hz == 3_000.0 || hz == 4_000.0 {
                    -50.0
                } else {
                    -80.0
                }),
            })
            .collect();
        Profile {
            left: ear.clone(),
            right: ear,
            ..Profile::default()
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
        let profile = notched_profile();
        for p in [None, Some(&profile)] {
            for (name, s) in presets(p) {
                let out = render(&hot, &s, p);
                assert_eq!(out.len(), hot.len(), "{name}");
                let peak = out
                    .left
                    .iter()
                    .chain(&out.right)
                    .fold(0f32, |m, v| m.max(v.abs()));
                assert!(peak <= ceiling, "{name} peaked at {peak}");
            }
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
    fn compression_moves_the_6k_tone_down() {
        let c = clip(0.1);
        let lowering = Lowering::Compress {
            cutoff_hz: 2_000.0,
            ratio: 2.0,
        };
        let s = Settings {
            lowering,
            compress: false,
            eq_strength: 0.0,
            ..Settings::default()
        };
        let out = render(&c, &s, None);
        let body = &out.left[9_600..38_400];
        assert!(amplitude_at(body, 6_000.0, 48_000.0) < 0.001);
        assert!(amplitude_at(body, lowering.map_hz(6_000.0), 48_000.0) > 0.01);
    }

    #[test]
    fn presets_follow_the_measured_shape() {
        let names = |p: Option<&Profile>| presets(p).into_iter().map(|x| x.0).collect::<Vec<_>>();
        assert_eq!(names(None).len(), 5);
        assert!(names(None).contains(&"Frequency compression"));

        let mut p = notched_profile();
        let with_notch = presets(Some(&p));
        let (_, copy) = with_notch
            .iter()
            .find(|x| x.0 == "Notch band copied down")
            .expect("notch preset");
        let Lowering::Transpose {
            src_lo_hz,
            src_hi_hz,
            ..
        } = copy.lowering
        else {
            panic!()
        };
        assert!(src_lo_hz < 3_000.0 && src_hi_hz > 4_000.0);

        p.tinnitus_right = Some(Tinnitus {
            hz: 4_000.0,
            dbfs: -45.0,
        });
        let guarded = presets(Some(&p));
        assert!(
            guarded
                .iter()
                .any(|x| x.0 == "EQ, sparing the tinnitus pitch" && x.1.tinnitus_guard)
        );
        assert!(!guarded.iter().any(|x| x.0 == "Frequency compression"));
    }

    #[test]
    fn the_guard_changes_only_the_ear_with_a_tinnitus_match() {
        let mut p = notched_profile();
        p.tinnitus_right = Some(Tinnitus {
            hz: 3_500.0,
            dbfs: -45.0,
        });
        let sr = 48_000;
        let tone = sine(3_500.0, sr as f32, 0.05, 48_000);
        let c = Clip {
            sr,
            left: tone.clone(),
            right: tone,
        };
        let base = Settings {
            compress: false,
            ..Settings::default()
        };
        let open = render(&c, &base, Some(&p));
        let guarded = render(
            &c,
            &Settings {
                tinnitus_guard: true,
                ..base
            },
            Some(&p),
        );
        let level = |x: &[f32]| amplitude_at(&x[9_600..38_400], 3_500.0, 48_000.0);
        // Loudness matching rescales both ears together, so compare the
        // balance between ears: guarded, the right ear loses its boost.
        let balance = |c: &Clip| level(&c.right) / level(&c.left);
        assert!((balance(&open) - 1.0).abs() < 0.02);
        assert!(balance(&guarded) < 0.5, "balance {}", balance(&guarded));
    }

    #[test]
    fn settings_saved_before_the_guard_existed_still_load() {
        let old = r#"{"lowering":"Off","eq_strength":0.5,"max_eq_gain_db":20.0,"compress":true}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert!(!s.tinnitus_guard);
    }
}
