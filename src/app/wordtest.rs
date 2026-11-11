//! The word-test screen: one word, heard once, pick it from six.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, RichText};
use kvimh::profile;
use kvimh::render::{self, Settings};
use kvimh::words::{self, Answer, Session, Trial};

use super::App;

const PER_RENDERING: usize = 12;
/// Background noise choices: label and word-to-noise ratio in dB.
const NOISE: [(&str, Option<f32>); 3] = [
    ("Quiet", None),
    ("Some background noise", Some(6.0)),
    ("A lot of background noise", Some(0.0)),
];
const KEYS: [egui::Key; 6] = [
    egui::Key::Num1,
    egui::Key::Num2,
    egui::Key::Num3,
    egui::Key::Num4,
    egui::Key::Num5,
    egui::Key::Num6,
];

type Prepared = Arc<Mutex<Option<Result<Vec<String>, String>>>>;

struct Run {
    plan: Vec<Trial>,
    names: Vec<&'static str>,
    settings: Vec<Settings>,
    snr: Option<f32>,
    idx: usize,
    answers: Vec<Answer>,
    play_at: f64,
    played: bool,
    rng: fastrand::Rng,
}

pub(super) struct Words {
    voices: Vec<String>,
    preparing: Option<Prepared>,
    noise: usize,
    run: Option<Run>,
    sessions: Vec<Session>,
    last: Option<Vec<Answer>>,
}

impl Words {
    pub(super) fn load() -> Words {
        Words {
            voices: words::voices(),
            preparing: None,
            noise: 0,
            run: None,
            sessions: words::load_sessions(),
            last: None,
        }
    }

    /// Drop a test in progress without saving it.
    pub(super) fn abandon(&mut self) {
        self.run = None;
    }
}

fn percent(correct: usize, total: usize) -> String {
    if total == 0 {
        "—".into()
    } else {
        format!(
            "{:.0}% ({correct}/{total})",
            100.0 * correct as f32 / total as f32
        )
    }
}

fn score_rows(ui: &mut egui::Ui, answers: &[Answer]) {
    for (name, correct, total) in words::scores(answers) {
        ui.label(format!("{:>14}   {name}", percent(correct, total)));
    }
}

impl App {
    fn start_word_test(&mut self, now: f64) {
        self.stop();
        let presets = render::presets(self.profile.as_ref());
        let mut rng = fastrand::Rng::new();
        let w = &mut self.words;
        w.last = None;
        w.run = Some(Run {
            plan: words::plan(PER_RENDERING, presets.len(), w.voices.len(), &mut rng),
            names: presets.iter().map(|p| p.0).collect(),
            settings: presets.iter().map(|p| p.1).collect(),
            snr: NOISE[w.noise].1,
            idx: 0,
            answers: Vec::new(),
            play_at: now + 1.5,
            played: false,
            rng,
        });
    }

    pub(super) fn ui_words(&mut self, ui: &mut egui::Ui) {
        ui.heading("5 · Word test");
        let now = ui.input(|i| i.time);
        let Some(engine) = &self.engine else {
            ui.label("No sound device is open. Choose one on the Setup tab.");
            return;
        };
        let w = &mut self.words;

        // ----- a test in progress -----
        if let Some(run) = &mut w.run {
            let trial = run.plan[run.idx];
            if !run.played && now >= run.play_at {
                let voice = &w.voices[trial.voice % w.voices.len().max(1)];
                match words::load_word(voice, trial.target(), engine.sample_rate) {
                    Ok(word) => {
                        let heard = match run.snr {
                            Some(snr) => words::in_noise(&word, snr, &mut run.rng),
                            None => word,
                        };
                        let out = render::render(
                            &heard,
                            &run.settings[trial.rendering],
                            self.profile.as_ref(),
                        );
                        engine.play_clip(Arc::new(out), 0);
                        run.played = true;
                    }
                    Err(e) => {
                        self.status = format!("Could not load the word recordings: {e}");
                        w.run = None;
                        return;
                    }
                }
            }
            ui.ctx().request_repaint_after(Duration::from_millis(30));

            ui.label(format!("Word {} of {}", run.idx + 1, run.plan.len()));
            ui.add(
                egui::ProgressBar::new(run.idx as f32 / run.plan.len() as f32).desired_width(420.0),
            );
            ui.add_space(10.0);
            ui.label(RichText::new("Which word was it?").size(20.0));
            ui.weak(
                "Each word is said once. If you are not sure, guess — a guess is still an answer.",
            );
            ui.add_space(8.0);
            let mut chosen = None;
            ui.horizontal_wrapped(|ui| {
                for (i, word) in trial.choices().iter().enumerate() {
                    let label = RichText::new(format!("{}  {word}", i + 1)).size(22.0);
                    let button = egui::Button::new(label).min_size(egui::vec2(110.0, 54.0));
                    let pressed = ui.input(|inp| inp.key_pressed(KEYS[i]));
                    if ui.add_enabled(run.played, button).clicked() || (run.played && pressed) {
                        chosen = Some(*word);
                    }
                }
            });
            ui.weak("Click a word, or press its number.");
            ui.add_space(12.0);
            let stop = ui.button("Stop the test (nothing is saved)").clicked();

            if let Some(word) = chosen {
                run.answers.push(Answer {
                    word: trial.target().to_string(),
                    chosen: word.to_string(),
                    voice: w.voices[trial.voice % w.voices.len().max(1)].clone(),
                    rendering: run.names[trial.rendering].to_string(),
                });
                run.idx += 1;
                run.played = false;
                run.play_at = now + 0.9;
                if run.idx >= run.plan.len() {
                    engine.stop();
                    let session = Session {
                        taken_unix: profile::now_unix(),
                        device: engine.device_name.clone(),
                        snr_db: run.snr,
                        answers: std::mem::take(&mut run.answers),
                    };
                    self.status = match words::save_session(&session) {
                        Ok(()) => format!("Word test saved to {}", words::results_path().display()),
                        Err(e) => format!("Could not save the word test: {e}"),
                    };
                    w.last = Some(session.answers.clone());
                    w.sessions.push(session);
                    w.run = None;
                }
            } else if stop {
                engine.stop();
                w.run = None;
            }
            return;
        }

        // ----- idle -----
        ui.label(
            "You hear one word, once, in a voice you do not know, and pick it from six that differ by a \
             single consonant. Each word is processed one of five ways, chosen at random, and you are not \
             told which. This measures whether you got the word, not which version you prefer.",
        );
        ui.add_space(8.0);

        // The recordings are made once, with Windows' own speech synthesis.
        if let Some(slot) = w.preparing.clone() {
            let finished = slot.lock().ok().and_then(|mut s| s.take());
            match finished {
                Some(Ok(voices)) => {
                    w.voices = voices;
                    w.preparing = None;
                }
                Some(Err(e)) => {
                    self.status = e;
                    w.preparing = None;
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Making the word recordings. This takes up to a minute and makes no sound.");
                    });
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                    return;
                }
            }
        }
        if w.voices.is_empty() {
            ui.label(
                "First the words have to be recorded: about seventy short words in each of Windows' \
                 built-in voices, saved on this PC. It is done once.",
            );
            if ui
                .button(RichText::new("Make the word recordings").size(16.0))
                .clicked()
            {
                let slot: Prepared = Arc::new(Mutex::new(None));
                let theirs = slot.clone();
                std::thread::spawn(move || {
                    let result = words::prepare();
                    if let Ok(mut s) = theirs.lock() {
                        *s = Some(result);
                    }
                });
                w.preparing = Some(slot);
            }
            return;
        }

        ui.horizontal(|ui| {
            for (i, (label, _)) in NOISE.iter().enumerate() {
                ui.radio_value(&mut w.noise, i, *label);
            }
        });
        ui.weak(
            "Start in quiet. If you get nearly everything right, the test has nothing to measure — add \
             noise until some words become hard.",
        );
        ui.add_space(6.0);
        let total = PER_RENDERING * render::presets(self.profile.as_ref()).len();
        let start = ui
            .button(RichText::new(format!("Start ({total} words, about five minutes)")).size(16.0))
            .clicked();
        ui.weak(format!("Voices: {}", w.voices.join(", ")));

        if let Some(last) = &w.last {
            ui.add_space(12.0);
            ui.label(RichText::new("This session").strong());
            score_rows(ui, last);
        }
        if !w.sessions.is_empty() {
            let pick = |noisy: bool| -> Vec<Answer> {
                w.sessions
                    .iter()
                    .filter(|s| s.snr_db.is_some() == noisy)
                    .flat_map(|s| s.answers.clone())
                    .collect()
            };
            for (title, answers) in [
                ("All sessions in quiet", pick(false)),
                ("All sessions in noise", pick(true)),
            ] {
                if !answers.is_empty() {
                    ui.add_space(12.0);
                    ui.label(RichText::new(title).strong());
                    score_rows(ui, &answers);
                }
            }
            let all: Vec<Answer> = w.sessions.iter().flat_map(|s| s.answers.clone()).collect();
            let missed = words::confusions(&all, 8);
            if !missed.is_empty() {
                ui.add_space(12.0);
                ui.label(RichText::new("Most common mix-ups").strong());
                for (word, heard_as, count) in missed {
                    ui.label(format!("{count} ×   “{word}” heard as “{heard_as}”"));
                }
            }
            ui.add_space(8.0);
            ui.weak(
                "Twelve words per version is a small sample. In a single session, differences of less than \
                 about fifteen points mean nothing; look at the totals after a few sessions.",
            );
        }
        if start {
            self.start_word_test(now);
        }
    }
}
