//! The hearing profile: per-ear tone thresholds measured through one
//! headset, and what the processing derives from them.
//!
//! Thresholds are stored as measured, in dBFS on that headset at the volume
//! it was set to. They are not a clinical audiogram. The "loss" derived here
//! is an estimate relative to the listener's own low-frequency hearing.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::dsp::spectral::{Lowering, interp_log_hz};

/// Test frequencies, ascending.
pub const TEST_FREQS: [f32; 11] = [
    250.0, 500.0, 1_000.0, 1_500.0, 2_000.0, 3_000.0, 4_000.0, 6_000.0, 8_000.0, 10_000.0, 12_000.0,
];

/// The loudest a test tone may ever be played.
pub const MAX_TEST_DBFS: f32 = -10.0;

/// Approximate level of normal-hearing thresholds across frequency
/// (headphone reference thresholds, dB SPL). Only the shape matters here:
/// normal ears need more level at the extremes than around 1–4 kHz.
const REFERENCE: [(f32, f32); 13] = [
    (125.0, 45.0),
    (250.0, 25.5),
    (500.0, 11.5),
    (750.0, 8.0),
    (1_000.0, 7.0),
    (1_500.0, 6.5),
    (2_000.0, 9.0),
    (3_000.0, 10.0),
    (4_000.0, 9.5),
    (6_000.0, 15.5),
    (8_000.0, 13.0),
    (10_000.0, 22.0),
    (12_500.0, 28.0),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ear {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Threshold {
    pub hz: f32,
    /// Quietest level heard, or `None` if not heard even at `MAX_TEST_DBFS`.
    pub dbfs: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Seconds since the Unix epoch.
    pub taken_unix: u64,
    pub device: String,
    pub left: Vec<Threshold>,
    pub right: Vec<Threshold>,
}

impl Profile {
    pub fn ear(&self, ear: Ear) -> &[Threshold] {
        match ear {
            Ear::Left => &self.left,
            Ear::Right => &self.right,
        }
    }

    pub fn ear_mut(&mut self, ear: Ear) -> &mut Vec<Threshold> {
        match ear {
            Ear::Left => &mut self.left,
            Ear::Right => &mut self.right,
        }
    }

    /// Estimated loss per frequency in dB, sorted by frequency: 0 at the
    /// ear's best point at or below 1 kHz, growing where more level was
    /// needed than a normal ear's threshold shape would predict. A tone that
    /// was never heard counts as 10 dB past the loudest level tried.
    pub fn loss(&self, ear: Ear) -> Vec<(f32, f32)> {
        let mut rel: Vec<(f32, f32)> = self
            .ear(ear)
            .iter()
            .map(|t| {
                let level = t.dbfs.unwrap_or(MAX_TEST_DBFS + 10.0);
                (t.hz, level - interp_log_hz(&REFERENCE, t.hz))
            })
            .collect();
        rel.sort_by(|a, b| a.0.total_cmp(&b.0));
        let low = rel.iter().filter(|p| p.0 <= 1_000.0).map(|p| p.1);
        let anchor = low.fold(f32::INFINITY, f32::min);
        let anchor = if anchor.is_finite() {
            anchor
        } else {
            rel.iter().map(|p| p.1).fold(f32::INFINITY, f32::min)
        };
        rel.iter()
            .map(|&(hz, r)| (hz, (r - anchor).max(0.0)))
            .collect()
    }

    /// EQ points for one ear: `strength` of the estimated loss (0.5 is the
    /// classic half-gain rule), never more than `max_gain_db`.
    pub fn eq_points(&self, ear: Ear, strength: f32, max_gain_db: f32) -> Vec<(f32, f32)> {
        self.loss(ear)
            .into_iter()
            .map(|(hz, loss)| (hz, (loss * strength).clamp(0.0, max_gain_db.max(0.0))))
            .collect()
    }

    /// Highest frequency up to which both ears' average loss stays within
    /// `usable_db` — roughly where amplification stops being enough.
    pub fn usable_edge_hz(&self, usable_db: f32) -> Option<f32> {
        let (l, r) = (self.loss(Ear::Left), self.loss(Ear::Right));
        let mut edge = None;
        for &hz in &TEST_FREQS {
            let at = |v: &[(f32, f32)]| v.iter().find(|p| p.0 == hz).map(|p| p.1);
            let avg = match (at(&l), at(&r)) {
                (Some(a), Some(b)) => (a + b) / 2.0,
                (Some(a), None) | (None, Some(a)) => a,
                (None, None) => continue,
            };
            if avg > usable_db {
                break;
            }
            edge = Some(hz);
        }
        edge
    }

    /// A frequency-compression setting that folds everything up to 8 kHz
    /// into the range this listener can still use.
    pub fn suggest_compress(&self) -> Lowering {
        let edge = self
            .usable_edge_hz(50.0)
            .unwrap_or(2_000.0)
            .clamp(1_500.0, 6_000.0);
        let cutoff_hz = (edge * 0.6).clamp(1_200.0, 3_500.0);
        let ratio = ((8_000.0 / cutoff_hz).ln() / (edge / cutoff_hz).ln()).clamp(1.3, 4.0);
        Lowering::Compress { cutoff_hz, ratio }
    }

    pub fn load() -> Option<Profile> {
        let text = std::fs::read_to_string(data_dir().join("profile.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = data_dir().join("profile.json");
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(path)
    }
}

/// Where kvimh keeps the listener's own data (profile, clips, results).
/// Deliberately outside the repo: none of it belongs in a public tree.
pub fn data_dir() -> PathBuf {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kvimh");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sloping high-frequency loss: normal to 1.5 kHz, then falling away,
    /// with 10 and 12 kHz not heard at all.
    fn sloping() -> Profile {
        let extra = [0.0, 0.0, 0.0, 5.0, 20.0, 40.0, 55.0, 65.0, 70.0];
        let mut ear: Vec<Threshold> = TEST_FREQS
            .iter()
            .zip(extra)
            .map(|(&hz, e)| Threshold {
                hz,
                dbfs: Some(-90.0 + interp_log_hz(&REFERENCE, hz) + e),
            })
            .collect();
        ear.push(Threshold {
            hz: 10_000.0,
            dbfs: None,
        });
        ear.push(Threshold {
            hz: 12_000.0,
            dbfs: None,
        });
        Profile {
            taken_unix: 0,
            device: "test".into(),
            left: ear.clone(),
            right: ear,
        }
    }

    #[test]
    fn loss_is_zero_in_the_lows_and_follows_the_slope() {
        let loss = sloping().loss(Ear::Left);
        let at = |hz: f32| loss.iter().find(|p| p.0 == hz).unwrap().1;
        assert!(at(500.0).abs() < 0.01 && at(1_000.0).abs() < 0.01);
        assert!((at(3_000.0) - 40.0).abs() < 0.01);
        assert!((at(8_000.0) - 70.0).abs() < 0.01);
        // Not heard at the cap counts as at least as bad as the worst heard.
        assert!(at(12_000.0) > 40.0);
    }

    #[test]
    fn eq_is_capped_and_never_cuts() {
        let pts = sloping().eq_points(Ear::Right, 0.5, 20.0);
        assert!(pts.iter().all(|p| (0.0..=20.0).contains(&p.1)));
        let at = |hz: f32| pts.iter().find(|p| p.0 == hz).unwrap().1;
        assert_eq!(at(1_000.0), 0.0);
        assert!((at(2_000.0) - 10.0).abs() < 0.01);
        assert_eq!(at(8_000.0), 20.0);
    }

    #[test]
    fn suggestion_folds_8k_down_to_the_usable_edge() {
        let p = sloping();
        let edge = p.usable_edge_hz(50.0).unwrap();
        assert_eq!(edge, 3_000.0);
        let lowering = p.suggest_compress();
        let Lowering::Compress { cutoff_hz, ratio } = lowering else {
            panic!()
        };
        assert!(cutoff_hz < edge && ratio > 1.0);
        assert!((lowering.map_hz(8_000.0) - edge).abs() < 50.0);
        assert_eq!(lowering.map_hz(1_000.0), 1_000.0);
    }

    #[test]
    fn an_empty_profile_still_gives_a_sane_suggestion() {
        let Lowering::Compress { cutoff_hz, ratio } = Profile::default().suggest_compress() else {
            panic!()
        };
        assert!(cutoff_hz >= 1_200.0 && (1.3..=4.0).contains(&ratio));
    }
}
