//! The hearing profile: per-ear tone thresholds measured through one
//! headset, an optional tinnitus pitch match, and what the processing
//! derives from them.
//!
//! Thresholds are stored as measured, in dBFS on that headset at the volume
//! it was set to. They are not a clinical audiogram. The "loss" derived here
//! is an estimate relative to the listener's own mid-frequency hearing.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::dsp::spectral::{Lowering, interp_log_hz};

/// Test frequencies, ascending.
pub const TEST_FREQS: [f32; 11] = [
    250.0, 500.0, 1_000.0, 1_500.0, 2_000.0, 3_000.0, 4_000.0, 6_000.0, 8_000.0, 10_000.0, 12_000.0,
];

/// The loudest a test tone may ever be played.
pub const MAX_TEST_DBFS: f32 = -10.0;

/// How far above the listener's own threshold a tinnitus-match tone may go.
/// Kept small on purpose: a tone at the tinnitus pitch is the sound most
/// likely to be uncomfortable.
pub const TINNITUS_MATCH_HEADROOM_DB: f32 = 15.0;

/// Half-width of the tinnitus guard, as a frequency ratio (a third of an
/// octave either side of the matched pitch).
const GUARD_RATIO: f32 = 1.26;

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

/// The pitch (and rough loudness) of a steady tinnitus, as matched by ear.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tinnitus {
    pub hz: f32,
    pub dbfs: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Seconds since the Unix epoch.
    pub taken_unix: u64,
    pub device: String,
    pub left: Vec<Threshold>,
    pub right: Vec<Threshold>,
    /// Matched tinnitus tones, any number per ear, sorted by pitch.
    #[serde(default, deserialize_with = "tones")]
    pub tinnitus_left: Vec<Tinnitus>,
    #[serde(default, deserialize_with = "tones")]
    pub tinnitus_right: Vec<Tinnitus>,
}

/// Reads a list of tinnitus tones, and also what earlier versions saved:
/// a single tone, or nothing.
fn tones<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Tinnitus>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Many(Vec<Tinnitus>),
        One(Tinnitus),
    }
    Ok(match Option::<Saved>::deserialize(d)? {
        Some(Saved::Many(v)) => v,
        Some(Saved::One(t)) => vec![t],
        None => Vec::new(),
    })
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

    pub fn tinnitus(&self, ear: Ear) -> &[Tinnitus] {
        match ear {
            Ear::Left => &self.tinnitus_left,
            Ear::Right => &self.tinnitus_right,
        }
    }

    pub fn tinnitus_mut(&mut self, ear: Ear) -> &mut Vec<Tinnitus> {
        match ear {
            Ear::Left => &mut self.tinnitus_left,
            Ear::Right => &mut self.tinnitus_right,
        }
    }

    pub fn has_tinnitus(&self) -> bool {
        !self.tinnitus_left.is_empty() || !self.tinnitus_right.is_empty()
    }

    /// Add a matched tone, keeping the ear's tones sorted by pitch. Ringing
    /// is often more than one tone, so an ear can hold several.
    pub fn add_tinnitus(&mut self, ear: Ear, tone: Tinnitus) {
        let list = self.tinnitus_mut(ear);
        list.push(tone);
        list.sort_by(|a, b| a.hz.total_cmp(&b.hz));
    }

    /// Replace (or add) the threshold for one tone.
    pub fn set_threshold(&mut self, ear: Ear, threshold: Threshold) {
        let list = self.ear_mut(ear);
        match list.iter_mut().find(|t| t.hz == threshold.hz) {
            Some(t) => *t = threshold,
            None => list.push(threshold),
        }
    }

    /// Heard thresholds for one ear as `(hz, dbfs)`, sorted by frequency.
    fn heard(&self, ear: Ear) -> Vec<(f32, f32)> {
        let mut v: Vec<(f32, f32)> = self
            .ear(ear)
            .iter()
            .filter_map(|t| t.dbfs.map(|d| (t.hz, d)))
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    }

    /// The measured threshold at any frequency, interpolated between the
    /// tones that were heard.
    pub fn threshold_at(&self, ear: Ear, hz: f32) -> Option<f32> {
        let heard = self.heard(ear);
        (!heard.is_empty()).then(|| interp_log_hz(&heard, hz))
    }

    /// The loudest a tinnitus-match tone may be at `hz` in this ear.
    pub fn tinnitus_match_cap(&self, ear: Ear, hz: f32) -> f32 {
        match self.threshold_at(ear, hz) {
            Some(t) => (t + TINNITUS_MATCH_HEADROOM_DB).min(MAX_TEST_DBFS),
            None => -40.0,
        }
    }

    /// Tones heard much more easily than both neighbours — 18 dB or more
    /// below their average. That is what a key pressed on tinnitus instead of
    /// the tone looks like, so it is worth a retest. The opposite shape, one
    /// tone worse than both neighbours, is not flagged: that is what a real
    /// notch looks like.
    pub fn outliers(&self, ear: Ear) -> Vec<f32> {
        self.heard(ear)
            .windows(3)
            .filter(|w| {
                let (a, x, b) = (w[0].1, w[1].1, w[2].1);
                x < a && x < b && (a + b) / 2.0 - x >= 18.0
            })
            .map(|w| w[1].0)
            .collect()
    }

    /// Estimated loss per frequency in dB, sorted by frequency: 0 around the
    /// ear's best mid-frequency hearing, growing where more level was needed
    /// than a normal ear's threshold shape would predict. A tone that was
    /// never heard counts as 10 dB past the loudest level tried.
    ///
    /// The zero point is the median of the three best tones between 500 Hz
    /// and 2 kHz. The lowest tones are left out of it because a headset's
    /// bass response and seal vary too much to anchor on.
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

        let mut mids: Vec<f32> = rel
            .iter()
            .filter(|p| (500.0..=2_000.0).contains(&p.0))
            .map(|p| p.1)
            .collect();
        mids.sort_by(f32::total_cmp);
        let anchor = match mids.len() {
            0 => rel.iter().map(|p| p.1).fold(f32::INFINITY, f32::min),
            1 | 2 => mids[0],
            _ => mids[1],
        };
        rel.iter()
            .map(|&(hz, r)| (hz, (r - anchor).max(0.0)))
            .collect()
    }

    /// Loss averaged over both ears, per test frequency.
    fn average_loss(&self) -> Vec<(f32, f32)> {
        let (l, r) = (self.loss(Ear::Left), self.loss(Ear::Right));
        let at = |v: &[(f32, f32)], hz: f32| v.iter().find(|p| p.0 == hz).map(|p| p.1);
        TEST_FREQS
            .iter()
            .filter_map(|&hz| match (at(&l, hz), at(&r, hz)) {
                (Some(a), Some(b)) => Some((hz, (a + b) / 2.0)),
                (Some(a), None) | (None, Some(a)) => Some((hz, a)),
                (None, None) => None,
            })
            .collect()
    }

    /// EQ points for one ear: `strength` of the estimated loss (0.5 is the
    /// classic half-gain rule), never more than `max_gain_db`, never a cut.
    ///
    /// With `guard`, and a tinnitus match for this ear, the EQ adds nothing
    /// within a third of an octave of the matched pitch: boosting there
    /// boosts the sound most likely to be uncomfortable.
    pub fn eq_points(
        &self,
        ear: Ear,
        strength: f32,
        max_gain_db: f32,
        guard: bool,
    ) -> Vec<(f32, f32)> {
        let mut points: Vec<(f32, f32)> = self
            .loss(ear)
            .into_iter()
            .map(|(hz, loss)| (hz, (loss * strength).clamp(0.0, max_gain_db.max(0.0))))
            .collect();
        if !guard || points.is_empty() {
            return points;
        }
        // Carve one gap per matched tone. Each carve reads the curve as the
        // previous one left it, so guards that overlap merge cleanly.
        for t in self.tinnitus(ear) {
            let (lo, hi) = (t.hz / GUARD_RATIO, t.hz * GUARD_RATIO);
            let (inner_lo, inner_hi) = (t.hz / GUARD_RATIO.sqrt(), t.hz * GUARD_RATIO.sqrt());
            let (gain_lo, gain_hi) = (interp_log_hz(&points, lo), interp_log_hz(&points, hi));
            points.retain(|p| p.0 < lo || p.0 > hi);
            points.extend([
                (lo, gain_lo),
                (inner_lo, 0.0),
                (inner_hi, 0.0),
                (hi, gain_hi),
            ]);
            points.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        points
    }

    /// Highest frequency up to which both ears' average loss stays within
    /// `usable_db` — roughly where amplification stops being enough.
    pub fn usable_edge_hz(&self, usable_db: f32) -> Option<f32> {
        let mut edge = None;
        for (hz, loss) in self.average_loss() {
            if loss > usable_db {
                break;
            }
            edge = Some(hz);
        }
        edge
    }

    /// A band of loss with better hearing on both sides, as `(lo_hz, hi_hz)`:
    /// the worst point at or below 8 kHz (if it is at least 15 dB down), the
    /// neighbours that are nearly as bad, and half a step either side.
    pub fn notch_band(&self) -> Option<(f32, f32)> {
        let loss: Vec<(f32, f32)> = self
            .average_loss()
            .into_iter()
            .filter(|p| p.0 <= 8_000.0)
            .collect();
        let worst = (0..loss.len()).max_by(|&a, &b| loss[a].1.total_cmp(&loss[b].1))?;
        let depth = loss[worst].1;
        // A notch needs a floor on both sides: skip a loss that just keeps
        // falling to the top of the range.
        if depth < 15.0 || worst == 0 || worst + 1 >= loss.len() {
            return None;
        }
        let in_notch = |i: usize| loss[i].1 >= 0.6 * depth;
        let (mut first, mut last) = (worst, worst);
        while first > 1 && in_notch(first - 1) {
            first -= 1;
        }
        while last + 2 < loss.len() && in_notch(last + 1) {
            last += 1;
        }
        if !in_notch(first) || in_notch(first - 1) || in_notch(last + 1) {
            return None;
        }
        Some((
            (loss[first - 1].0 * loss[first].0).sqrt(),
            (loss[last].0 * loss[last + 1].0).sqrt(),
        ))
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

    /// Copy the notch band down so its top lands at its own bottom edge —
    /// just below the notch, where hearing is clear. The original stays.
    pub fn suggest_notch_copy(&self) -> Option<Lowering> {
        let (lo, hi) = self.notch_band()?;
        Some(Lowering::Transpose {
            src_lo_hz: lo,
            src_hi_hz: hi,
            factor: (lo / hi).clamp(0.5, 0.85),
            mix_db: -3.0,
        })
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

    /// Build both ears from "dB worse than a normal ear" per test frequency;
    /// `None` means not heard.
    fn profile_from(extra: [Option<f32>; 11]) -> Profile {
        let ear: Vec<Threshold> = TEST_FREQS
            .iter()
            .zip(extra)
            .map(|(&hz, e)| Threshold {
                hz,
                dbfs: e.map(|e| -90.0 + interp_log_hz(&REFERENCE, hz) + e),
            })
            .collect();
        Profile {
            device: "test".into(),
            left: ear.clone(),
            right: ear,
            ..Profile::default()
        }
    }

    /// Normal to 1.5 kHz, then falling away; 10 and 12 kHz not heard.
    fn sloping() -> Profile {
        let s = Some;
        profile_from([
            s(0.0),
            s(0.0),
            s(0.0),
            s(5.0),
            s(20.0),
            s(40.0),
            s(55.0),
            s(65.0),
            s(70.0),
            None,
            None,
        ])
    }

    /// A dip at 3–4 kHz with good hearing on both sides.
    fn notched() -> Profile {
        let s = Some;
        profile_from([
            s(8.0),
            s(0.0),
            s(0.0),
            s(2.0),
            s(0.0),
            s(28.0),
            s(32.0),
            s(6.0),
            s(10.0),
            s(8.0),
            s(40.0),
        ])
    }

    fn at(points: &[(f32, f32)], hz: f32) -> f32 {
        points.iter().find(|p| p.0 == hz).unwrap().1
    }

    #[test]
    fn loss_is_zero_in_the_mids_and_follows_the_slope() {
        let loss = sloping().loss(Ear::Left);
        assert!(at(&loss, 500.0).abs() < 0.01 && at(&loss, 1_000.0).abs() < 0.01);
        assert!((at(&loss, 3_000.0) - 40.0).abs() < 0.01);
        assert!((at(&loss, 8_000.0) - 70.0).abs() < 0.01);
        // Not heard at the cap counts as at least as bad as the worst heard.
        assert!(at(&loss, 12_000.0) > 40.0);
    }

    #[test]
    fn a_weak_bass_reading_does_not_move_the_zero_point() {
        // 250 Hz reads 8 dB poor (headset seal); the mids still define zero.
        let loss = notched().loss(Ear::Left);
        assert!((at(&loss, 250.0) - 8.0).abs() < 0.01);
        assert!((at(&loss, 4_000.0) - 32.0).abs() < 0.01);
        assert_eq!(at(&loss, 1_000.0), 0.0);
    }

    #[test]
    fn eq_is_capped_and_never_cuts() {
        let pts = sloping().eq_points(Ear::Right, 0.5, 20.0, false);
        assert!(pts.iter().all(|p| (0.0..=20.0).contains(&p.1)));
        assert_eq!(at(&pts, 1_000.0), 0.0);
        assert!((at(&pts, 2_000.0) - 10.0).abs() < 0.01);
        assert_eq!(at(&pts, 8_000.0), 20.0);
    }

    #[test]
    fn tinnitus_guard_removes_the_boost_around_the_matched_pitch_only() {
        let mut p = notched();
        p.add_tinnitus(
            Ear::Right,
            Tinnitus {
                hz: 3_800.0,
                dbfs: -40.0,
            },
        );
        let open = p.eq_points(Ear::Right, 0.5, 20.0, false);
        let guarded = p.eq_points(Ear::Right, 0.5, 20.0, true);
        assert!(interp_log_hz(&open, 3_800.0) > 12.0);
        assert_eq!(interp_log_hz(&guarded, 3_800.0), 0.0);
        assert_eq!(interp_log_hz(&guarded, 3_500.0), 0.0);
        // Outside the guard the curve is what it was.
        for hz in [1_000.0, 2_000.0, 2_900.0, 5_000.0, 8_000.0, 12_000.0] {
            let (a, b) = (interp_log_hz(&open, hz), interp_log_hz(&guarded, hz));
            assert!((a - b).abs() < 0.01, "{hz} Hz: {a} vs {b}");
        }
        // The other ear has no match, so its guard does nothing.
        assert_eq!(
            p.eq_points(Ear::Left, 0.5, 20.0, true),
            p.eq_points(Ear::Left, 0.5, 20.0, false)
        );
    }

    #[test]
    fn notch_is_found_with_its_edges_and_a_copy_lands_below_it() {
        let p = notched();
        let (lo, hi) = p.notch_band().expect("a notch");
        assert!(
            (lo - 2_449.5).abs() < 1.0 && (hi - 4_899.0).abs() < 1.0,
            "{lo}..{hi}"
        );
        let Some(copy @ Lowering::Transpose { factor, .. }) = p.suggest_notch_copy() else {
            panic!()
        };
        assert!((0.5..=0.85).contains(&factor));
        assert!((copy.map_hz(hi) - lo).abs() < 1.0);
        assert!(copy.map_hz(3_500.0) < lo);
    }

    #[test]
    fn a_slope_or_a_flat_ear_is_not_a_notch() {
        assert_eq!(sloping().notch_band(), None);
        assert_eq!(profile_from([Some(0.0); 11]).notch_band(), None);
        assert_eq!(Profile::default().notch_band(), None);
    }

    #[test]
    fn a_tone_out_of_line_with_both_neighbours_is_flagged() {
        let mut p = notched();
        assert!(
            p.outliers(Ear::Left).is_empty(),
            "{:?}",
            p.outliers(Ear::Left)
        );
        // 6 kHz suddenly 30 dB better than the tones either side of it.
        let good = p.threshold_at(Ear::Left, 6_000.0).unwrap() - 30.0;
        p.set_threshold(
            Ear::Left,
            Threshold {
                hz: 6_000.0,
                dbfs: Some(good),
            },
        );
        assert_eq!(p.outliers(Ear::Left), vec![6_000.0]);
        assert!(p.outliers(Ear::Right).is_empty());
    }

    #[test]
    fn set_threshold_replaces_in_place() {
        let mut p = notched();
        p.set_threshold(
            Ear::Left,
            Threshold {
                hz: 4_000.0,
                dbfs: None,
            },
        );
        assert_eq!(p.left.len(), 11);
        assert_eq!(p.left.iter().find(|t| t.hz == 4_000.0).unwrap().dbfs, None);
    }

    #[test]
    fn tinnitus_match_tone_is_capped_just_above_threshold() {
        let p = notched();
        let threshold = p.threshold_at(Ear::Left, 4_000.0).unwrap();
        assert_eq!(
            p.tinnitus_match_cap(Ear::Left, 4_000.0),
            threshold + TINNITUS_MATCH_HEADROOM_DB
        );
        // Never above the global tone cap, and conservative with no data.
        let deaf = profile_from([Some(75.0); 11]);
        assert_eq!(deaf.tinnitus_match_cap(Ear::Left, 1_000.0), MAX_TEST_DBFS);
        assert_eq!(
            Profile::default().tinnitus_match_cap(Ear::Left, 1_000.0),
            -40.0
        );
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

    #[test]
    fn a_profile_saved_before_tinnitus_matching_still_loads() {
        let old = r#"{"taken_unix":1,"device":"d","left":[{"hz":1000.0,"dbfs":-60.0}],"right":[]}"#;
        let p: Profile = serde_json::from_str(old).unwrap();
        assert!(!p.has_tinnitus());
    }

    #[test]
    fn a_profile_with_one_tone_per_ear_loads_as_lists_and_round_trips() {
        // What the single-tone version wrote: an object, or null.
        let old = r#"{"taken_unix":1,"device":"d","left":[],"right":[],
            "tinnitus_left":null,"tinnitus_right":{"hz":5000.0,"dbfs":-40.0}}"#;
        let mut p: Profile = serde_json::from_str(old).unwrap();
        assert!(p.tinnitus_left.is_empty());
        assert_eq!(
            p.tinnitus_right,
            vec![Tinnitus {
                hz: 5_000.0,
                dbfs: -40.0
            }]
        );

        p.add_tinnitus(
            Ear::Right,
            Tinnitus {
                hz: 3_000.0,
                dbfs: -50.0,
            },
        );
        let hz: Vec<f32> = p.tinnitus(Ear::Right).iter().map(|t| t.hz).collect();
        assert_eq!(hz, vec![3_000.0, 5_000.0]);
        let again: Profile = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(again, p);
    }

    #[test]
    fn every_matched_tone_gets_its_own_guard() {
        let mut p = profile_from([Some(40.0); 11]);
        // A flat 40 dB "loss" everywhere would be anchored to zero, so make
        // the mids good and everything from 3 kHz up poor.
        for t in p.right.iter_mut().filter(|t| t.hz <= 2_000.0) {
            t.dbfs = t.dbfs.map(|d| d - 40.0);
        }
        p.add_tinnitus(
            Ear::Right,
            Tinnitus {
                hz: 8_500.0,
                dbfs: -36.0,
            },
        );
        p.add_tinnitus(
            Ear::Right,
            Tinnitus {
                hz: 5_200.0,
                dbfs: -36.0,
            },
        );
        let open = p.eq_points(Ear::Right, 0.5, 20.0, false);
        let guarded = p.eq_points(Ear::Right, 0.5, 20.0, true);
        for hz in [5_200.0, 8_500.0] {
            assert!(interp_log_hz(&open, hz) > 15.0);
            assert_eq!(interp_log_hz(&guarded, hz), 0.0, "{hz} Hz");
        }
        // Between and outside the two guards the boost is untouched.
        for hz in [3_500.0, 6_650.0, 11_500.0] {
            let (a, b) = (interp_log_hz(&open, hz), interp_log_hz(&guarded, hz));
            assert!((a - b).abs() < 0.01, "{hz} Hz: {a} vs {b}");
        }
    }
}
