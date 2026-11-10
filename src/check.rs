//! The hearing check as a state machine, separate from the window so it can
//! be tested with a fake clock and a fake tone player.
//!
//! For each ear and frequency a pulsed tone starts far below anything
//! audible and rises in steps until the listener presses the key. That is
//! done twice: a fast first pass in 5 dB steps to find the neighbourhood,
//! then a slow pass in 2 dB steps starting 15 dB below it. The slow pass is
//! the recorded threshold.

use kvimh::profile::{Ear, MAX_TEST_DBFS, Profile, Threshold};

pub const START_DBFS: f32 = -90.0;
/// Seconds spent at each level: two pulses.
pub const STEP_SECS: f64 = 0.8;
const COARSE_STEP: f32 = 5.0;
const FINE_STEP: f32 = 2.0;

/// What the check needs from the sound device.
pub trait TonePlayer {
    fn play_tone(&self, hz: f32, dbfs: f32, ear: Option<Ear>);
    fn set_tone_level(&self, dbfs: f32);
    fn stop(&self);
}

impl TonePlayer for kvimh::audio::Engine {
    fn play_tone(&self, hz: f32, dbfs: f32, ear: Option<Ear>) {
        kvimh::audio::Engine::play_tone(self, hz, dbfs, ear)
    }
    fn set_tone_level(&self, dbfs: f32) {
        kvimh::audio::Engine::set_tone_level(self, dbfs)
    }
    fn stop(&self) {
        kvimh::audio::Engine::stop(self)
    }
}

pub struct Check {
    queue: Vec<(Ear, f32)>,
    idx: usize,
    fine_pass: bool,
    level: f32,
    step: f32,
    sounding: bool,
    wait_until: f64,
    next_step_at: f64,
    coarse_hit: Option<f32>,
    pub early_presses: u32,
    pub profile: Profile,
}

impl Check {
    pub fn new(now: f64, device: &str) -> Check {
        // Start each ear where hearing is usually best, so the first tones
        // teach the listener what to listen for.
        let order = [
            1_000.0, 1_500.0, 2_000.0, 3_000.0, 4_000.0, 6_000.0, 8_000.0, 10_000.0, 12_000.0,
            500.0, 250.0,
        ];
        let queue = [Ear::Left, Ear::Right]
            .into_iter()
            .flat_map(|ear| order.map(|hz| (ear, hz)))
            .collect();
        let mut check = Check {
            queue,
            idx: 0,
            fine_pass: false,
            level: START_DBFS,
            step: COARSE_STEP,
            sounding: false,
            wait_until: 0.0,
            next_step_at: 0.0,
            coarse_hit: None,
            early_presses: 0,
            profile: Profile {
                device: device.to_string(),
                ..Profile::default()
            },
        };
        check.begin_pass(now, START_DBFS, COARSE_STEP);
        check
    }

    pub fn current(&self) -> Option<(Ear, f32)> {
        self.queue.get(self.idx).copied()
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.idx, self.queue.len())
    }

    pub fn finished(&self) -> bool {
        self.idx >= self.queue.len()
    }

    /// A silent gap of unpredictable length, then the tone starts rising.
    fn begin_pass(&mut self, now: f64, start: f32, step: f32) {
        self.sounding = false;
        self.level = start;
        self.step = step;
        self.wait_until = now + 0.8 + fastrand::f64() * 1.4;
    }

    fn record(&mut self, now: f64, dbfs: Option<f32>) {
        if let Some((ear, hz)) = self.current() {
            self.profile.ear_mut(ear).push(Threshold { hz, dbfs });
        }
        self.idx += 1;
        self.fine_pass = false;
        self.coarse_hit = None;
        self.begin_pass(now, START_DBFS, COARSE_STEP);
    }

    /// Advance the check. `pressed` is true on the frame the key went down.
    pub fn tick(&mut self, now: f64, player: &dyn TonePlayer, pressed: bool) {
        let Some((ear, hz)) = self.current() else {
            return;
        };
        if !self.sounding {
            if pressed {
                self.early_presses += 1;
            }
            if now >= self.wait_until {
                self.sounding = true;
                self.next_step_at = now + STEP_SECS;
                player.play_tone(hz, self.level, Some(ear));
            }
            return;
        }
        if pressed {
            player.stop();
            if self.fine_pass {
                self.record(now, Some(self.level));
            } else {
                self.coarse_hit = Some(self.level);
                self.fine_pass = true;
                self.begin_pass(now, (self.level - 15.0).max(START_DBFS - 5.0), FINE_STEP);
            }
        } else if now >= self.next_step_at {
            let next = self.level + self.step;
            if next > MAX_TEST_DBFS {
                // Reached the loudest allowed level without a press. On the
                // slow pass, fall back to what the fast pass found.
                player.stop();
                self.record(now, self.coarse_hit);
            } else {
                self.level = next;
                self.next_step_at = now + STEP_SECS;
                player.set_tone_level(next);
            }
        }
    }

    /// Record the current tone as not heard and move on.
    pub fn skip(&mut self, now: f64, player: &dyn TonePlayer) {
        player.stop();
        self.record(now, None);
    }

    /// Go back and test the previous tone again.
    pub fn redo_previous(&mut self, now: f64, player: &dyn TonePlayer) {
        player.stop();
        if self.idx > 0 && !self.fine_pass && !self.sounding {
            self.idx -= 1;
            let ear = self.queue[self.idx].0;
            self.profile.ear_mut(ear).pop();
        }
        self.fine_pass = false;
        self.coarse_hit = None;
        self.begin_pass(now, START_DBFS, COARSE_STEP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records the loudest level asked for and whether a tone is sounding.
    #[derive(Default)]
    struct Fake {
        level: RefCell<Option<f32>>,
        loudest: RefCell<f32>,
    }

    impl Fake {
        fn set(&self, dbfs: f32) {
            *self.level.borrow_mut() = Some(dbfs);
            let mut l = self.loudest.borrow_mut();
            *l = l.max(dbfs);
        }
    }

    impl TonePlayer for Fake {
        fn play_tone(&self, _hz: f32, dbfs: f32, _ear: Option<Ear>) {
            self.set(dbfs)
        }
        fn set_tone_level(&self, dbfs: f32) {
            self.set(dbfs)
        }
        fn stop(&self) {
            *self.level.borrow_mut() = None;
        }
    }

    /// Run a whole check for a listener who presses as soon as the tone is at
    /// or above `threshold(ear, hz)`, or never if that is `None`.
    fn run(threshold: impl Fn(Ear, f32) -> Option<f32>) -> (Profile, f32) {
        let fake = Fake {
            loudest: RefCell::new(f32::MIN),
            ..Fake::default()
        };
        let mut check = Check::new(0.0, "fake");
        let mut now = 0.0;
        while !check.finished() {
            now += 0.05;
            let (ear, hz) = check.current().unwrap();
            let audible = match (*fake.level.borrow(), threshold(ear, hz)) {
                (Some(level), Some(t)) => level >= t,
                _ => false,
            };
            check.tick(now, &fake, audible);
            assert!(now < 3_600.0, "check never finished");
        }
        (check.profile, fake.loudest.into_inner())
    }

    #[test]
    fn finds_each_threshold_to_within_a_fine_step() {
        let truth =
            |ear: Ear, hz: f32| Some(if ear == Ear::Left { -62.0 } else { -41.0 } + hz / 1_000.0);
        let (profile, _) = run(truth);
        assert_eq!((profile.left.len(), profile.right.len()), (11, 11));
        for ear in [Ear::Left, Ear::Right] {
            for t in profile.ear(ear) {
                let want = truth(ear, t.hz).unwrap();
                let got = t.dbfs.expect("heard");
                assert!(
                    got >= want && got < want + FINE_STEP + 0.01,
                    "{ear:?} {}: {got} vs {want}",
                    t.hz
                );
            }
        }
    }

    #[test]
    fn unheard_tones_are_recorded_as_none_and_never_exceed_the_cap() {
        let (profile, loudest) = run(|_, hz| if hz >= 8_000.0 { None } else { Some(-50.0) });
        assert!(loudest <= MAX_TEST_DBFS, "played a tone at {loudest} dBFS");
        for t in profile.left.iter().chain(&profile.right) {
            assert_eq!(t.dbfs.is_none(), t.hz >= 8_000.0, "{} Hz", t.hz);
        }
    }

    #[test]
    fn every_tone_starts_at_the_quiet_end() {
        let fake = Fake::default();
        let mut check = Check::new(0.0, "fake");
        let mut now = 0.0;
        while fake.level.borrow().is_none() {
            now += 0.05;
            check.tick(now, &fake, false);
        }
        assert_eq!(*fake.level.borrow(), Some(START_DBFS));
    }

    #[test]
    fn redo_discards_the_previous_answer() {
        let fake = Fake::default();
        let mut check = Check::new(0.0, "fake");
        check.skip(1.0, &fake);
        assert_eq!((check.progress().0, check.profile.left.len()), (1, 1));
        check.redo_previous(2.0, &fake);
        assert_eq!((check.progress().0, check.profile.left.len()), (0, 0));
    }
}
