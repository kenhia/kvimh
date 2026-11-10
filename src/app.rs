//! The window: setup, the hearing check, the listen-and-tweak screen and the
//! blind test.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use kvimh::audio::{self, Engine, Recorder};
use kvimh::dsp::spectral::Lowering;
use kvimh::profile::{self, Ear, MAX_TEST_DBFS, Profile, Threshold};
use kvimh::render::{self, Clip, Settings};
use kvimh::wav;

use crate::check::Check;

const RECORD_SECS: f32 = 15.0;
const REFERENCE_DBFS: f32 = -40.0;
const LEFT: Color32 = Color32::from_rgb(70, 130, 220);
const RIGHT: Color32 = Color32::from_rgb(220, 80, 70);

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Setup,
    Check,
    Listen,
    Blind,
}

struct Variant {
    name: &'static str,
    settings: Settings,
    clip: Arc<Clip>,
    rating: u8,
}

struct Blind {
    variants: Vec<Variant>,
    revealed: bool,
}

pub struct App {
    tab: Tab,
    status: String,
    devices: Vec<String>,
    engine: Option<Engine>,
    profile: Option<Profile>,
    check: Option<Check>,
    // clips
    clips: Vec<PathBuf>,
    clip_path: Option<PathBuf>,
    clip: Option<Arc<Clip>>,
    recorder: Option<(Recorder, f64)>,
    // listen
    settings: Settings,
    processed: Option<(Settings, Arc<Clip>)>,
    // blind test
    blind: Option<Blind>,
    summary: Vec<(String, f32, usize)>,
}

fn clips_dir() -> PathBuf {
    let dir = profile::data_dir().join("clips");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn list_clips() -> Vec<PathBuf> {
    let mut clips: Vec<PathBuf> = std::fs::read_dir(clips_dir())
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    clips.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")));
    clips.sort();
    clips
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A made-up sloping high-frequency loss, a little worse in the left ear.
fn demo_profile() -> Profile {
    let ear = |worse: f32| -> Vec<Threshold> {
        let levels = [
            -62.0, -74.0, -80.0, -76.0, -62.0, -44.0, -30.0, -22.0, -16.0,
        ];
        let mut v: Vec<Threshold> = profile::TEST_FREQS
            .iter()
            .zip(levels)
            .map(|(&hz, l): (&f32, f32)| Threshold {
                hz,
                dbfs: Some(l + if hz > 1_500.0 { worse } else { 0.0 }),
            })
            .collect();
        v.push(Threshold {
            hz: 10_000.0,
            dbfs: (worse == 0.0).then_some(-11.0),
        });
        v.push(Threshold {
            hz: 12_000.0,
            dbfs: None,
        });
        v
    };
    Profile {
        taken_unix: 0,
        device: "demo".into(),
        left: ear(5.0),
        right: ear(0.0),
    }
}

/// Average rating per rendering across every saved blind-test round.
fn load_summary() -> Vec<(String, f32, usize)> {
    let text =
        std::fs::read_to_string(profile::data_dir().join("results.jsonl")).unwrap_or_default();
    let mut totals: Vec<(String, f32, usize)> = Vec::new();
    for round in text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
    {
        for r in round["ratings"].as_array().into_iter().flatten() {
            let (Some(name), Some(rating)) = (r["name"].as_str(), r["rating"].as_f64()) else {
                continue;
            };
            match totals.iter_mut().find(|t| t.0 == name) {
                Some(t) => {
                    t.1 += rating as f32;
                    t.2 += 1;
                }
                None => totals.push((name.to_string(), rating as f32, 1)),
            }
        }
    }
    for t in &mut totals {
        t.1 /= t.2 as f32;
    }
    totals.sort_by(|a, b| b.1.total_cmp(&a.1));
    totals
}

impl App {
    /// `tab` picks the opening screen. `demo` swaps in a made-up hearing
    /// profile and a synthetic clip — for looking at the screens without
    /// anyone's real data; nothing from a demo run is loaded from disk.
    pub fn new(tab: &str, demo: bool) -> App {
        let mut app = App::load();
        app.tab = match tab {
            "check" => Tab::Check,
            "listen" => Tab::Listen,
            "blind" => Tab::Blind,
            _ => Tab::Setup,
        };
        if demo {
            app.profile = Some(demo_profile());
            let sr = app.engine.as_ref().map_or(48_000, |e| e.sample_rate);
            let sweep: Vec<f32> = (0..sr as usize * 3)
                .map(|i| {
                    let t = i as f32 / sr as f32;
                    0.1 * (std::f32::consts::TAU * (300.0 * t + 1_200.0 * t * t)).sin()
                })
                .collect();
            app.clip = Some(Arc::new(Clip {
                sr,
                left: sweep.clone(),
                right: sweep,
            }));
            app.clip_path = None;
            app.status = "Demo mode: made-up hearing profile and a test sweep.".into();
        }
        app
    }

    fn load() -> App {
        let (engine, status) = match Engine::new(None) {
            Ok(e) => (Some(e), String::new()),
            Err(e) => (None, format!("Could not open the sound device: {e}")),
        };
        // Kept settings win; otherwise start from what the hearing check suggests.
        let saved_profile = Profile::load();
        let settings = std::fs::read_to_string(profile::data_dir().join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| Settings {
                lowering: saved_profile
                    .as_ref()
                    .map_or(Settings::default().lowering, |p| p.suggest_compress()),
                ..Settings::default()
            });
        let mut app = App {
            tab: Tab::Setup,
            status,
            devices: audio::output_devices(),
            engine,
            profile: saved_profile,
            check: None,
            clips: list_clips(),
            clip_path: None,
            clip: None,
            recorder: None,
            settings,
            processed: None,
            blind: None,
            summary: load_summary(),
        };
        if let Some(first) = app.clips.last().cloned() {
            app.select_clip(first);
        }
        app
    }

    fn stop(&self) {
        if let Some(e) = &self.engine {
            e.stop();
        }
    }

    fn open_device(&mut self, name: &str) {
        self.stop();
        self.check = None;
        self.recorder = None;
        self.engine = None;
        match Engine::new(Some(name)) {
            Ok(e) => {
                self.engine = Some(e);
                self.status.clear();
                if let Some(path) = self.clip_path.clone() {
                    self.select_clip(path);
                }
            }
            Err(e) => self.status = format!("Could not open {name}: {e}"),
        }
    }

    fn select_clip(&mut self, path: PathBuf) {
        self.stop();
        self.processed = None;
        self.blind = None;
        let Some(sr) = self.engine.as_ref().map(|e| e.sample_rate) else {
            return;
        };
        match wav::load(&path) {
            Ok(mut clip) => {
                let max = clip.sr as usize * 60;
                clip.left.truncate(max);
                clip.right.truncate(max);
                self.clip = Some(Arc::new(wav::resample(&clip, sr)));
                self.clip_path = Some(path);
            }
            Err(e) => self.status = format!("Could not read {}: {e}", file_name(&path)),
        }
    }

    fn processed(&mut self) -> Option<Arc<Clip>> {
        let clip = self.clip.clone()?;
        if self.processed.as_ref().map(|p| p.0) != Some(self.settings) {
            let out = render::render(&clip, &self.settings, self.profile.as_ref());
            self.processed = Some((self.settings, Arc::new(out)));
        }
        self.processed.as_ref().map(|p| p.1.clone())
    }

    /// Play `clip`, carrying on from where the current clip had got to.
    fn play(&self, clip: Arc<Clip>, restart: bool) {
        if let Some(e) = &self.engine {
            let pos = if restart || !e.is_playing() || e.position() >= clip.len() {
                0
            } else {
                e.position()
            };
            e.play_clip(clip, pos);
        }
    }

    fn finish_recording(&mut self) {
        let Some((recorder, _)) = self.recorder.take() else {
            return;
        };
        let clip = recorder.finish();
        let peak = clip
            .left
            .iter()
            .chain(&clip.right)
            .fold(0f32, |m, v| m.max(v.abs()));
        if clip.seconds() < 1.0 || peak < 0.001 {
            self.status = "Nothing was playing, so nothing was captured.".into();
            return;
        }
        let path = clips_dir().join(format!("clip-{}.wav", profile::now_unix()));
        match wav::save(&path, &clip) {
            Ok(()) => {
                self.status = format!("Captured {:.0} s as {}", clip.seconds(), file_name(&path));
                self.clips = list_clips();
                self.select_clip(path);
            }
            Err(e) => self.status = format!("Could not save the clip: {e}"),
        }
    }

    fn take_dropped_files(&mut self, ui: &egui::Ui) {
        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for src in dropped {
            if !src
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
            {
                self.status = "Only .wav files can be added.".into();
                continue;
            }
            let dest = clips_dir().join(file_name(&src));
            match std::fs::copy(&src, &dest) {
                Ok(_) => {
                    self.clips = list_clips();
                    self.select_clip(dest);
                }
                Err(e) => self.status = format!("Could not copy {}: {e}", file_name(&src)),
            }
        }
    }

    // ----- Setup -----------------------------------------------------------

    fn ui_setup(&mut self, ui: &mut egui::Ui) {
        ui.heading("1 · Setup");
        ui.label("Pick the headphones you will listen through, then set their volume once.");
        ui.add_space(8.0);

        let current = self
            .engine
            .as_ref()
            .map(|e| e.device_name.clone())
            .unwrap_or_default();
        let mut chosen = current.clone();
        egui::ComboBox::from_label("Output device")
            .selected_text(&current)
            .width(380.0)
            .show_ui(ui, |ui| {
                for name in &self.devices {
                    ui.selectable_value(&mut chosen, name.clone(), name);
                }
            });
        if chosen != current {
            self.open_device(&chosen);
        }
        if let Some(e) = &self.engine {
            ui.weak(format!("{} Hz", e.sample_rate));
        }

        ui.add_space(12.0);
        ui.label(RichText::new("Set the volume").strong());
        ui.label(
            "Play the reference tone and turn the headset volume to where it is clearly audible and \
             comfortable, about like quiet speech. Then leave the volume alone: the hearing check and \
             everything measured from it only hold at this setting.",
        );
        ui.horizontal(|ui| {
            if ui.button("▶ Play reference tone").clicked()
                && let Some(e) = &self.engine
            {
                e.play_tone(1_000.0, REFERENCE_DBFS, None);
            }
            if ui.button("■ Stop").clicked() {
                self.stop();
            }
        });

        ui.add_space(12.0);
        ui.label(RichText::new("Safety").strong());
        ui.label(format!(
            "Test tones start far below hearing and rise slowly; they stop at {MAX_TEST_DBFS:.0} dB below \
             full scale. Everything this app plays passes a hard limit {:.0} dB below full scale. If \
             anything is ever uncomfortable, take the headset off — do not wait for it to stop.",
            -render::CEILING_DB
        ));
        ui.add_space(12.0);
        ui.weak(format!(
            "Your profile, clips and results are kept in {}",
            profile::data_dir().display()
        ));
    }

    // ----- Hearing check ---------------------------------------------------

    fn ui_check(&mut self, ui: &mut egui::Ui) {
        ui.heading("2 · Hearing check");
        let now = ui.input(|i| i.time);
        let Some(engine) = &self.engine else {
            ui.label("No sound device is open. Choose one on the Setup tab.");
            return;
        };

        if let Some(check) = &mut self.check {
            // Space must reach the check, not whichever button was clicked last.
            ui.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            let pressed = ui.input(|i| i.key_pressed(egui::Key::Space));
            check.tick(now, engine, pressed);
            ui.ctx().request_repaint_after(Duration::from_millis(20));

            if let Some((ear, hz)) = check.current() {
                let (done, total) = check.progress();
                let (name, colour) = if ear == Ear::Left {
                    ("LEFT", LEFT)
                } else {
                    ("RIGHT", RIGHT)
                };
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("{name} ear"))
                        .size(30.0)
                        .color(colour)
                        .strong(),
                );
                ui.label(format!("Tone {} of {total}  ·  {hz:.0} Hz", done + 1));
                ui.add(egui::ProgressBar::new(done as f32 / total as f32).desired_width(420.0));
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "Press SPACE the moment you hear a pulsing tone: beep · beep · beep",
                    )
                    .size(18.0),
                );
                ui.label(
                    "Each tone is played twice. It starts silent and rises slowly, so most of the time \
                     you will hear nothing. A steady ring is not the tone; wait for the pulse. If a tone \
                     never comes, do nothing and the check moves on by itself.",
                );
                if check.early_presses > 2 {
                    ui.weak(format!(
                        "{} presses came while no tone was playing. That is normal with tinnitus; they are ignored.",
                        check.early_presses
                    ));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Redo the previous tone").clicked() {
                        check.redo_previous(now, engine);
                    }
                    if ui.button("I can't hear this one — skip").clicked() {
                        check.skip(now, engine);
                    }
                });
            }

            let finished = check.finished();
            let cancelled = ui.button("Stop the check").clicked();
            if finished {
                let mut profile = self.check.take().map(|c| c.profile).unwrap_or_default();
                profile.taken_unix = profile::now_unix();
                self.status = match profile.save() {
                    Ok(path) => format!("Hearing check saved to {}", path.display()),
                    Err(e) => format!("Could not save the hearing check: {e}"),
                };
                if let Some(Lowering::Compress { .. }) = Some(self.settings.lowering) {
                    self.settings.lowering = profile.suggest_compress();
                }
                self.profile = Some(profile);
                self.processed = None;
                self.blind = None;
            } else if cancelled {
                engine.stop();
                self.check = None;
            }
            return;
        }

        ui.label(
            "About eight minutes. Sit somewhere quiet, headset on the right way round, volume as you set \
             it on the Setup tab. The left ear is tested first, then the right, eleven tones each.",
        );
        ui.add_space(6.0);
        let label = if self.profile.is_some() {
            "Run the check again"
        } else {
            "Start the hearing check"
        };
        if ui.button(RichText::new(label).size(16.0)).clicked() {
            engine.stop();
            self.check = Some(Check::new(now, &engine.device_name));
        }
        ui.add_space(12.0);
        match &self.profile {
            Some(p) => {
                audiogram(ui, p);
                ui.add_space(6.0);
                if let Some(edge) = p.usable_edge_hz(50.0) {
                    ui.label(format!(
                        "Usable hearing runs to about {:.1} kHz. Above that, making sounds louder is not \
                         enough, which is where frequency lowering comes in.",
                        edge / 1_000.0
                    ));
                }
                ui.weak(
                    "This is a relative measurement on this headset at this volume, not a clinical audiogram. \
                     \"Loss\" is measured against your own best low-frequency hearing.",
                );
            }
            None => {
                ui.weak("No hearing check yet. Until there is one, the EQ stays flat.");
            }
        }
    }

    // ----- Clips (shared by Listen and Blind) -------------------------------

    fn ui_clips(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|i| i.time);
        ui.horizontal(|ui| {
            let current = self
                .clip_path
                .as_deref()
                .map(file_name)
                .unwrap_or_else(|| "(none)".into());
            let mut chosen = self.clip_path.clone();
            egui::ComboBox::from_label("Clip")
                .selected_text(current)
                .width(260.0)
                .show_ui(ui, |ui| {
                    for path in &self.clips {
                        ui.selectable_value(&mut chosen, Some(path.clone()), file_name(path));
                    }
                });
            if chosen != self.clip_path
                && let Some(path) = chosen
            {
                self.select_clip(path);
            }
            if let Some(clip) = &self.clip {
                ui.weak(format!("{:.0} s", clip.seconds()));
            }
        });

        if self
            .recorder
            .as_ref()
            .is_some_and(|(r, started)| r.seconds() >= RECORD_SECS || now - started > 40.0)
        {
            self.finish_recording();
        }
        ui.horizontal(|ui| match &self.recorder {
            Some((r, _)) => {
                ui.add(
                    egui::ProgressBar::new(r.seconds() / RECORD_SECS)
                        .desired_width(260.0)
                        .text("capturing…"),
                );
                if ui.button("Stop now").clicked() {
                    self.finish_recording();
                }
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            None => {
                if ui
                    .button(format!("⏺ Capture {RECORD_SECS:.0} s of what is playing"))
                    .clicked()
                    && let Some(e) = &self.engine
                {
                    e.stop();
                    match e.record_loopback() {
                        Ok(r) => self.recorder = Some((r, now)),
                        Err(err) => self.status = format!("Could not start capturing: {err}"),
                    }
                }
                ui.weak("or drop a .wav file on this window");
            }
        });
        if self.clip.is_none() && self.recorder.is_none() {
            ui.label(
                "Start a video or podcast with a voice you find hard to follow, playing through this same \
                 headset, then press Capture. The clip stays on this PC.",
            );
        }
    }

    // ----- Listen ----------------------------------------------------------

    fn ui_listen(&mut self, ui: &mut egui::Ui) {
        ui.heading("3 · Listen and tweak");
        self.ui_clips(ui);
        ui.separator();

        let mut mode = match self.settings.lowering {
            Lowering::Off => 0,
            Lowering::Compress { .. } => 1,
            Lowering::Transpose { .. } => 2,
        };
        let before = mode;
        ui.label(RichText::new("Frequency lowering").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut mode, 0, "Off");
            ui.radio_value(&mut mode, 1, "Compression (squeeze the highs down)");
            ui.radio_value(
                &mut mode,
                2,
                "Transposition (copy the highs an octave down)",
            );
        });
        if mode != before {
            self.settings.lowering = match mode {
                0 => Lowering::Off,
                1 => self
                    .profile
                    .as_ref()
                    .map(|p| p.suggest_compress())
                    .unwrap_or(Settings::default().lowering),
                _ => Lowering::Transpose {
                    src_lo_hz: 4_000.0,
                    src_hi_hz: 9_000.0,
                    factor: 0.5,
                    mix_db: 0.0,
                },
            };
        }
        match &mut self.settings.lowering {
            Lowering::Off => {}
            Lowering::Compress { cutoff_hz, ratio } => {
                ui.add(
                    egui::Slider::new(cutoff_hz, 1_000.0..=5_000.0)
                        .step_by(50.0)
                        .text("Hz — untouched below this"),
                );
                ui.add(
                    egui::Slider::new(ratio, 1.0..=4.0)
                        .step_by(0.1)
                        .text("squeeze (1 = none)"),
                );
            }
            Lowering::Transpose {
                src_lo_hz,
                src_hi_hz,
                mix_db,
                ..
            } => {
                ui.add(
                    egui::Slider::new(src_lo_hz, 2_000.0..=6_000.0)
                        .step_by(100.0)
                        .text("Hz — copy from"),
                );
                ui.add(
                    egui::Slider::new(src_hi_hz, 6_000.0..=12_000.0)
                        .step_by(100.0)
                        .text("Hz — copy up to"),
                );
                ui.add(
                    egui::Slider::new(mix_db, -12.0..=6.0)
                        .step_by(1.0)
                        .text("dB — level of the copy"),
                );
            }
        }
        if self.settings.lowering != Lowering::Off {
            let l = self.settings.lowering;
            let moved =
                |hz: f32| format!("{:.0} k → {:.1} k", hz / 1_000.0, l.map_hz(hz) / 1_000.0);
            ui.weak(format!(
                "Where sounds end up:  {}   {}   {}",
                moved(4_000.0),
                moved(6_000.0),
                moved(8_000.0)
            ));
            if ui.button("Set from my hearing check").clicked()
                && let Some(p) = &self.profile
            {
                self.settings.lowering = p.suggest_compress();
            }
        }

        ui.add_space(8.0);
        ui.label(RichText::new("Loudness shaping").strong());
        ui.add_enabled(
            self.profile.is_some(),
            egui::Slider::new(&mut self.settings.eq_strength, 0.0..=1.0)
                .step_by(0.05)
                .text("EQ from the hearing check (0 = off, 0.5 = usual)"),
        );
        ui.add_enabled(
            self.profile.is_some(),
            egui::Slider::new(&mut self.settings.max_eq_gain_db, 0.0..=30.0)
                .step_by(1.0)
                .text("dB — most the EQ may add"),
        );
        ui.checkbox(
            &mut self.settings.compress,
            "Bring quiet sounds up (compression)",
        );

        ui.separator();
        if self.clip.is_none() {
            ui.label("Capture or add a clip above to listen.");
            return;
        }
        ui.horizontal(|ui| {
            if ui.button(RichText::new("▶ Original").size(16.0)).clicked()
                && let Some(c) = self.clip.clone()
            {
                self.play(Arc::new(render::render(&c, &Settings::BYPASS, None)), false);
            }
            if ui.button(RichText::new("▶ Processed").size(16.0)).clicked()
                && let Some(c) = self.processed()
            {
                self.play(c, false);
            }
            if ui.button(RichText::new("■ Stop").size(16.0)).clicked() {
                self.stop();
            }
        });
        ui.weak("Switching while it plays carries on from the same spot, so you can flip back and forth on one word.");
        if let (Some(e), Some(c)) = (&self.engine, &self.clip)
            && e.is_playing()
        {
            ui.add(
                egui::ProgressBar::new(e.position() as f32 / c.len().max(1) as f32)
                    .desired_width(420.0),
            );
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        ui.add_space(6.0);
        if ui.button("Keep these settings").clicked() {
            let saved = serde_json::to_string_pretty(&self.settings)
                .map_err(|e| e.to_string())
                .and_then(|t| {
                    std::fs::write(profile::data_dir().join("settings.json"), t)
                        .map_err(|e| e.to_string())
                });
            self.status = match saved {
                Ok(()) => "Settings kept.".into(),
                Err(e) => format!("Could not save the settings: {e}"),
            };
        }
    }

    // ----- Blind test ------------------------------------------------------

    fn start_round(&mut self) {
        let Some(clip) = self.clip.clone() else {
            return;
        };
        self.stop();
        let profile = self.profile.as_ref();
        let mut variants: Vec<Variant> = render::presets(profile)
            .into_iter()
            .map(|(name, settings)| Variant {
                name,
                settings,
                clip: Arc::new(render::render(&clip, &settings, profile)),
                rating: 0,
            })
            .collect();
        fastrand::shuffle(&mut variants);
        self.blind = Some(Blind {
            variants,
            revealed: false,
        });
    }

    fn save_round(&mut self) {
        let Some(blind) = &self.blind else {
            return;
        };
        let ratings: Vec<serde_json::Value> = blind
            .variants
            .iter()
            .map(|v| serde_json::json!({ "name": v.name, "rating": v.rating, "settings": v.settings }))
            .collect();
        let round = serde_json::json!({
            "taken_unix": profile::now_unix(),
            "clip": self.clip_path.as_deref().map(file_name),
            "device": self.engine.as_ref().map(|e| e.device_name.clone()),
            "ratings": ratings,
        });
        let path = profile::data_dir().join("results.jsonl");
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        text.push_str(&round.to_string());
        text.push('\n');
        self.status = match std::fs::write(&path, text) {
            Ok(()) => format!("Round saved to {}", path.display()),
            Err(e) => format!("Could not save the round: {e}"),
        };
        self.summary = load_summary();
    }

    fn ui_blind(&mut self, ui: &mut egui::Ui) {
        ui.heading("4 · Blind test");
        ui.label(
            "The same clip rendered five ways, in a shuffled order, all at the same loudness. Rate how \
             easy the words are to follow, then reveal which was which.",
        );
        self.ui_clips(ui);
        ui.separator();
        if self.clip.is_none() {
            return;
        }
        if self.blind.is_none() {
            if ui
                .button(RichText::new("Start a round").size(16.0))
                .clicked()
            {
                self.start_round();
            }
        } else {
            let mut play: Option<Arc<Clip>> = None;
            let mut reveal = false;
            if let Some(blind) = &mut self.blind {
                for (i, v) in blind.variants.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let letter = (b'A' + i as u8) as char;
                        if ui
                            .button(RichText::new(format!("▶ {letter}")).size(16.0))
                            .clicked()
                        {
                            play = Some(v.clip.clone());
                        }
                        ui.label("hard");
                        for score in 1..=5u8 {
                            ui.add_enabled_ui(!blind.revealed, |ui| {
                                ui.radio_value(&mut v.rating, score, score.to_string());
                            });
                        }
                        ui.label("easy");
                        if blind.revealed {
                            ui.label(RichText::new(v.name).strong());
                        }
                    });
                }
                let all_rated = blind.variants.iter().all(|v| v.rating > 0);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if !blind.revealed
                        && ui
                            .add_enabled(all_rated, egui::Button::new("Reveal and save"))
                            .clicked()
                    {
                        blind.revealed = true;
                        reveal = true;
                    }
                    if !blind.revealed && !all_rated {
                        ui.weak("Rate all five to reveal.");
                    }
                });
            }
            if let Some(clip) = play {
                self.play(clip, false);
            }
            if reveal {
                self.save_round();
            }
            ui.horizontal(|ui| {
                if ui.button("■ Stop").clicked() {
                    self.stop();
                }
                if ui.button("New round").clicked() {
                    self.start_round();
                }
            });
        }
        if let Some(e) = &self.engine
            && e.is_playing()
        {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

        if !self.summary.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("All rounds so far").strong());
            for (name, average, rounds) in &self.summary {
                ui.label(format!("{average:.1} / 5   {name}   ({rounds} rounds)"));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.take_dropped_files(ui);
        egui::CentralPanel::default().show(ui, |ui| {
            let before = self.tab;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Setup, "1 · Setup");
                ui.selectable_value(&mut self.tab, Tab::Check, "2 · Hearing check");
                ui.selectable_value(&mut self.tab, Tab::Listen, "3 · Listen");
                ui.selectable_value(&mut self.tab, Tab::Blind, "4 · Blind test");
            });
            if self.tab != before {
                // Leaving a screen silences it and abandons a check in progress.
                self.stop();
                self.check = None;
            }
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                match self.tab {
                    Tab::Setup => self.ui_setup(ui),
                    Tab::Check => self.ui_check(ui),
                    Tab::Listen => self.ui_listen(ui),
                    Tab::Blind => self.ui_blind(ui),
                }
                if !self.status.is_empty() {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.label(&self.status);
                }
            });
        });
    }
}

/// The hearing check drawn the way an audiogram is: frequency across, loss
/// downward, left ear blue crosses, right ear red circles.
fn audiogram(ui: &mut egui::Ui, profile: &Profile) {
    const MAX_LOSS: f32 = 100.0;
    let (response, painter) = ui.allocate_painter(egui::vec2(560.0, 280.0), egui::Sense::hover());
    let rect = response.rect.shrink2(egui::vec2(44.0, 24.0));
    let grid = ui.visuals().weak_text_color().gamma_multiply(0.4);
    let text = ui.visuals().text_color();
    let font = egui::FontId::proportional(11.0);
    let x = |hz: f32| rect.left() + rect.width() * (hz / 200.0).ln() / (14_000.0f32 / 200.0).ln();
    let y = |loss: f32| rect.top() + rect.height() * (loss.min(MAX_LOSS) / MAX_LOSS);

    for hz in [250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 12_000.0] {
        painter.line_segment(
            [
                egui::pos2(x(hz), rect.top()),
                egui::pos2(x(hz), rect.bottom()),
            ],
            (1.0, grid),
        );
        let label = if hz >= 1_000.0 {
            format!("{:.0}k", hz / 1_000.0)
        } else {
            format!("{hz:.0}")
        };
        painter.text(
            egui::pos2(x(hz), rect.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            label,
            font.clone(),
            text,
        );
    }
    for loss in [0.0, 20.0, 40.0, 60.0, 80.0, 100.0] {
        painter.line_segment(
            [
                egui::pos2(rect.left(), y(loss)),
                egui::pos2(rect.right(), y(loss)),
            ],
            (1.0, grid),
        );
        painter.text(
            egui::pos2(rect.left() - 6.0, y(loss)),
            egui::Align2::RIGHT_CENTER,
            format!("{loss:.0} dB"),
            font.clone(),
            text,
        );
    }
    painter.text(
        rect.left_top() - egui::vec2(0.0, 18.0),
        egui::Align2::LEFT_TOP,
        "estimated loss ↓",
        font.clone(),
        text,
    );

    for (ear, colour) in [(Ear::Left, LEFT), (Ear::Right, RIGHT)] {
        let loss = profile.loss(ear);
        let unheard = |hz: f32| {
            profile
                .ear(ear)
                .iter()
                .any(|t| t.hz == hz && t.dbfs.is_none())
        };
        let points: Vec<egui::Pos2> = loss
            .iter()
            .map(|&(hz, l)| egui::pos2(x(hz), y(l)))
            .collect();
        for pair in points.windows(2) {
            painter.line_segment([pair[0], pair[1]], (1.5, colour));
        }
        for (&(hz, _), &p) in loss.iter().zip(&points) {
            if unheard(hz) {
                painter.text(
                    p,
                    egui::Align2::CENTER_CENTER,
                    "▼",
                    egui::FontId::proportional(14.0),
                    colour,
                );
            } else if ear == Ear::Left {
                let d = 4.0;
                painter.line_segment(
                    [p + egui::vec2(-d, -d), p + egui::vec2(d, d)],
                    (2.0, colour),
                );
                painter.line_segment(
                    [p + egui::vec2(-d, d), p + egui::vec2(d, -d)],
                    (2.0, colour),
                );
            } else {
                painter.circle_stroke(p, 4.5, (2.0, colour));
            }
        }
    }
    painter.text(
        rect.right_top() + egui::vec2(-150.0, -18.0),
        egui::Align2::LEFT_TOP,
        "× left",
        font.clone(),
        LEFT,
    );
    painter.text(
        rect.right_top() + egui::vec2(-100.0, -18.0),
        egui::Align2::LEFT_TOP,
        "○ right",
        font.clone(),
        RIGHT,
    );
    painter.text(
        rect.right_top() + egui::vec2(-44.0, -18.0),
        egui::Align2::LEFT_TOP,
        "▼ not heard",
        font,
        text,
    );
}
