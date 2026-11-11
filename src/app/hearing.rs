//! The hearing-check screen when no check is running: the chart, what it
//! shows, single-tone retests, and the tinnitus pitch match.

use eframe::egui::{self, RichText};
use kvimh::profile::{self, Ear, Tinnitus};

use super::{App, LEFT, RIGHT, audiogram};
use crate::check::Check;

/// A tinnitus pitch match in progress.
pub(super) struct Matching {
    ear: Ear,
    hz: f32,
    dbfs: f32,
    playing: bool,
}

const MATCH_FLOOR_DBFS: f32 = -95.0;

fn ear_name(ear: Ear) -> &'static str {
    if ear == Ear::Left { "Left" } else { "Right" }
}

fn khz(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{:.1} kHz", hz / 1_000.0)
    } else {
        format!("{hz:.0} Hz")
    }
}

impl App {
    fn start_check(&mut self, now: f64, tones: Option<Vec<(Ear, f32)>>) {
        let Some(engine) = &self.engine else {
            return;
        };
        engine.stop();
        self.check = Some(match tones {
            Some(tones) => Check::for_tones(now, &engine.device_name, tones),
            None => Check::new(now, &engine.device_name),
        });
    }

    pub(super) fn ui_check_idle(&mut self, ui: &mut egui::Ui, now: f64) {
        if self.matching.is_some() {
            self.ui_tinnitus_match(ui);
            return;
        }
        ui.label(
            "About eight minutes. Sit somewhere quiet, headset on the right way round, volume as you set \
             it on the Setup tab. The left ear is tested first, then the right, eleven tones each.",
        );
        ui.add_space(6.0);
        let label = if self.profile.is_some() {
            "Run the whole check again"
        } else {
            "Start the hearing check"
        };
        if ui.button(RichText::new(label).size(16.0)).clicked() {
            self.start_check(now, None);
        }
        ui.add_space(12.0);
        let Some(p) = self.profile.clone() else {
            ui.weak("No hearing check yet. Until there is one, the EQ stays flat.");
            return;
        };

        audiogram(ui, &p);
        ui.add_space(6.0);
        match (p.notch_band(), p.usable_edge_hz(50.0)) {
            (Some((lo, hi)), _) => {
                ui.label(format!(
                    "There is a dip between about {} and {}, with clearer hearing on both sides of it.",
                    khz(lo),
                    khz(hi)
                ));
            }
            (None, Some(edge)) if edge < 8_000.0 => {
                ui.label(format!(
                    "Usable hearing runs to about {}. Above that, making sounds louder is not enough, \
                     which is where frequency lowering comes in.",
                    khz(edge)
                ));
            }
            _ => {}
        }
        ui.weak(
            "This is a relative measurement on this headset at this volume, not a clinical audiogram. \
             \"Loss\" is measured against your own best hearing between 500 Hz and 2 kHz.",
        );

        // ----- retest -----
        ui.add_space(12.0);
        ui.label(RichText::new("Retest one tone").strong());
        let mut retest = None;
        for (ear, colour) in [(Ear::Left, LEFT), (Ear::Right, RIGHT)] {
            let odd = p.outliers(ear);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(ear_name(ear)).color(colour));
                for &hz in &profile::TEST_FREQS {
                    let short = if hz >= 1_000.0 {
                        // `{}` and not `{:.0}`: 1500 Hz is "1.5k", not "2k".
                        format!("{}k", hz / 1_000.0)
                    } else {
                        format!("{hz:.0}")
                    };
                    let text = if odd.contains(&hz) {
                        format!("⚠ {short}")
                    } else {
                        short
                    };
                    if ui.small_button(text).clicked() {
                        retest = Some((ear, hz));
                    }
                }
            });
            for hz in odd {
                ui.label(format!(
                    "⚠ {} ear, {}: heard much more easily than the tones either side. That is what a press \
                     on ringing instead of the tone looks like — worth a retest.",
                    ear_name(ear),
                    khz(hz)
                ));
            }
        }
        if let Some(tone) = retest {
            self.start_check(now, Some(vec![tone]));
        }

        // ----- tinnitus -----
        ui.add_space(12.0);
        ui.label(RichText::new("Tinnitus pitch (optional)").strong());
        ui.label(
            "If you have a steady ringing, you can match its pitch here. Ringing is often more than one \
             tone, so add as many as you hear in each ear. kvimh then knows which bands to be careful \
             with: the EQ can leave them alone instead of boosting them.",
        );
        let mut remove = None;
        for (ear, colour) in [(Ear::Left, LEFT), (Ear::Right, RIGHT)] {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(ear_name(ear)).color(colour));
                if p.tinnitus(ear).is_empty() {
                    ui.weak("no tones matched");
                }
                for (i, t) in p.tinnitus(ear).iter().enumerate() {
                    ui.label(format!("{} at {:.0} dB", khz(t.hz), t.dbfs));
                    if ui
                        .small_button("✖")
                        .on_hover_text("Remove this tone")
                        .clicked()
                    {
                        remove = Some((ear, i));
                    }
                    ui.separator();
                }
                let label = if p.tinnitus(ear).is_empty() {
                    "Match a tone"
                } else {
                    "Add another tone"
                };
                if ui.button(label).clicked() {
                    self.matching = Some(Matching {
                        ear,
                        hz: 4_000.0,
                        dbfs: MATCH_FLOOR_DBFS,
                        playing: false,
                    });
                }
            });
        }
        if let (Some((ear, i)), Some(profile)) = (remove, &mut self.profile) {
            profile.tinnitus_mut(ear).remove(i);
            self.status = match profile.save() {
                Ok(_) => "Tinnitus tone removed.".into(),
                Err(e) => format!("Could not save the change: {e}"),
            };
            self.processed = None;
            self.blind = None;
        }
    }

    fn ui_tinnitus_match(&mut self, ui: &mut egui::Ui) {
        let (Some(engine), Some(profile), Some(m)) =
            (&self.engine, &mut self.profile, &mut self.matching)
        else {
            self.matching = None;
            return;
        };
        let colour = if m.ear == Ear::Left { LEFT } else { RIGHT };
        ui.label(
            RichText::new(format!("{} ear — match the ringing", ear_name(m.ear)))
                .size(22.0)
                .color(colour),
        );
        ui.label(
            "A pulsing tone plays in this ear only. Move the pitch until the tone sits on top of your \
             ringing — at the right pitch it seems to blend into it. Start with the level at the bottom \
             and raise it only as far as you need to compare.",
        );
        ui.weak(
            "This tone is close to the sound you may find uncomfortable, so the level is limited to a \
             little above what you could just hear at that pitch. Stop whenever you like; take the \
             headset off if it bothers you.",
        );
        ui.add_space(8.0);

        let before = (m.hz, m.dbfs);
        ui.add(
            egui::Slider::new(&mut m.hz, 500.0..=12_000.0)
                .logarithmic(true)
                .step_by(10.0)
                .text("Hz — pitch"),
        );
        let cap = profile.tinnitus_match_cap(m.ear, m.hz);
        m.dbfs = m.dbfs.min(cap);
        ui.add(
            egui::Slider::new(&mut m.dbfs, MATCH_FLOOR_DBFS..=cap)
                .step_by(1.0)
                .text("level"),
        );
        if m.playing {
            if m.hz != before.0 {
                engine.play_tone(m.hz, m.dbfs.min(cap), Some(m.ear));
            } else if m.dbfs != before.1 {
                engine.set_tone_level(m.dbfs.min(cap));
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if !m.playing && ui.button(RichText::new("▶ Play").size(16.0)).clicked() {
                m.playing = true;
                engine.play_tone(m.hz, m.dbfs.min(cap), Some(m.ear));
            }
            if m.playing && ui.button(RichText::new("■ Stop").size(16.0)).clicked() {
                m.playing = false;
                engine.stop();
            }
        });
        ui.add_space(10.0);
        let mut done: Option<Tinnitus> = None;
        let mut cancel = false;
        ui.horizontal(|ui| {
            if ui.button("This matches a tone I hear — save it").clicked() {
                done = Some(Tinnitus {
                    hz: m.hz,
                    dbfs: m.dbfs,
                });
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        ui.weak("If you hear more than one tone in this ear, save this one and then add the next.");
        if let Some(tone) = done {
            engine.stop();
            profile.add_tinnitus(m.ear, tone);
            self.status = match profile.save() {
                Ok(_) => "Tinnitus tone saved.".into(),
                Err(e) => format!("Could not save the tinnitus tone: {e}"),
            };
            self.matching = None;
            self.processed = None;
            self.blind = None;
        } else if cancel {
            engine.stop();
            self.matching = None;
        }
    }
}
