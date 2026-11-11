//! The word-recognition test: closed sets of look-alike words, spoken by
//! synthetic voices the listener has never heard, each played once.
//!
//! Preference ratings on a familiar clip measure familiarity. This measures
//! what the project is for: which word was it?

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dsp::db_to_lin;
use crate::profile;
use crate::render::Clip;
use crate::wav;

/// Each set differs in one consonant, mostly the ones that live at
/// 2–8 kHz: s, sh, f, th, t, k, p, ch, h.
pub const SETS: [[&str; 6]; 12] = [
    ["sip", "ship", "tip", "chip", "zip", "hip"],
    ["fin", "thin", "sin", "pin", "tin", "shin"],
    ["sat", "fat", "hat", "cat", "pat", "chat"],
    ["sea", "she", "tea", "key", "fee", "he"],
    ["so", "show", "toe", "foe", "though", "go"],
    ["seat", "sheet", "feet", "heat", "cheat", "peat"],
    ["bus", "buff", "but", "buck", "bush", "bug"],
    ["pass", "path", "pat", "pack", "pad", "pan"],
    ["rice", "right", "ripe", "ride", "rhyme", "rise"],
    ["sake", "shake", "fake", "take", "cake", "bake"],
    ["sore", "shore", "four", "tore", "core", "chore"],
    ["sigh", "shy", "thigh", "tie", "pie", "high"],
];

/// Level every word is normalised to, in dBFS RMS.
pub const WORD_RMS_DB: f32 = -26.0;

pub fn words_dir() -> PathBuf {
    let dir = profile::data_dir().join("words");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn all_words() -> Vec<&'static str> {
    let mut words: Vec<&str> = SETS.iter().flatten().copied().collect();
    words.sort();
    words.dedup();
    words
}

/// Voices that have a recording of every word, by folder name.
pub fn voices_in(dir: &Path) -> Vec<String> {
    let mut voices: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|e| {
            all_words()
                .iter()
                .all(|w| e.path().join(format!("{w}.wav")).is_file())
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    voices.sort();
    voices
}

pub fn voices() -> Vec<String> {
    voices_in(&words_dir())
}

/// The PowerShell script that speaks every word with every installed voice
/// into `dir/<voice>/<word>.wav`. Writes files only; nothing is played.
fn synth_script(dir: &Path) -> String {
    let quoted = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let words: Vec<String> = all_words().iter().map(|w| quoted(w)).collect();
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         Add-Type -AssemblyName System.Speech\n\
         $out = {out}\n\
         $words = @({words})\n\
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer\n\
         foreach ($v in $s.GetInstalledVoices()) {{\n\
           if (-not $v.Enabled) {{ continue }}\n\
           $name = $v.VoiceInfo.Name\n\
           $dir = Join-Path $out ($name -replace '[^A-Za-z0-9]+', '-')\n\
           New-Item -ItemType Directory -Force $dir | Out-Null\n\
           $s.SelectVoice($name)\n\
           foreach ($w in $words) {{\n\
             $f = Join-Path $dir \"$w.wav\"\n\
             if (-not (Test-Path $f)) {{ $s.SetOutputToWaveFile($f); $s.Speak($w); $s.SetOutputToNull() }}\n\
           }}\n\
         }}\n",
        out = quoted(&dir.to_string_lossy()),
        words = words.join(", "),
    )
}

/// Make the word recordings with Windows' built-in speech synthesis. Slow
/// (tens of seconds) the first time, instant afterwards. Returns the voices
/// that are ready.
pub fn prepare() -> Result<Vec<String>, String> {
    prepare_in(&words_dir())
}

pub fn prepare_in(dir: &Path) -> Result<Vec<String>, String> {
    use std::process::{Command, Stdio};

    // -EncodedCommand takes the script as base64 of UTF-16LE: no quoting
    // layers and no temp file. Windows PowerShell 5.1 specifically, because
    // that is where System.Speech lives.
    let utf16: Vec<u8> = synth_script(dir)
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
        &base64(&utf16),
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd
        .output()
        .map_err(|e| format!("could not start Windows PowerShell: {e}"))?;
    let voices = voices_in(dir);
    if voices.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        let first = err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no voices were produced");
        return Err(format!("speech synthesis failed: {first}"));
    }
    Ok(voices)
}

/// Load one word at the playback rate: silence trimmed, level normalised.
pub fn load_word(voice: &str, word: &str, sr: u32) -> Result<Clip, String> {
    let clip = wav::load(&words_dir().join(voice).join(format!("{word}.wav")))?;
    Ok(normalise(&trim_silence(&wav::resample(&clip, sr))))
}

/// Drop leading and trailing silence, keeping 20 ms either side.
pub fn trim_silence(clip: &Clip) -> Clip {
    let peak = clip
        .left
        .iter()
        .chain(&clip.right)
        .fold(0f32, |m, v| m.max(v.abs()));
    let floor = peak * db_to_lin(-45.0);
    let loud = |i: usize| clip.left[i].abs() > floor || clip.right[i].abs() > floor;
    let Some(first) = (0..clip.len()).find(|&i| loud(i)) else {
        return clip.clone();
    };
    let last = (0..clip.len()).rfind(|&i| loud(i)).unwrap_or(first);
    let margin = clip.sr as usize / 50;
    let (a, b) = (
        first.saturating_sub(margin),
        (last + margin + 1).min(clip.len()),
    );
    Clip {
        sr: clip.sr,
        left: clip.left[a..b].to_vec(),
        right: clip.right[a..b].to_vec(),
    }
}

/// Scale to `WORD_RMS_DB`, so every word is presented at the same level.
pub fn normalise(clip: &Clip) -> Clip {
    let mut out = clip.clone();
    let rms = out.rms();
    if rms > 1e-6 {
        out.scale(db_to_lin(WORD_RMS_DB) / rms);
    }
    out
}

/// Put the word in steady background noise at `snr_db`, with 0.25 s of
/// noise before and after. The noise is pink (equal energy per octave, a
/// rough stand-in for a room full of talk) and the same in both ears.
pub fn in_noise(word: &Clip, snr_db: f32, rng: &mut fastrand::Rng) -> Clip {
    let pad = word.sr as usize / 4;
    let len = word.len() + 2 * pad;
    let mut noise = pink(len, rng);
    let noise_rms = (noise.iter().map(|v| v * v).sum::<f32>() / len.max(1) as f32).sqrt();
    let gain = word.rms() * db_to_lin(-snr_db) / noise_rms.max(1e-9);
    // Fade the noise in and out so it does not click.
    let fade = (word.sr as usize / 50).max(1);
    for (i, v) in noise.iter_mut().enumerate() {
        let edge = i.min(len - 1 - i).min(fade) as f32 / fade as f32;
        *v *= gain * edge;
    }
    let mut out = Clip {
        sr: word.sr,
        left: noise.clone(),
        right: noise,
    };
    for i in 0..word.len() {
        out.left[pad + i] += word.left[i];
        out.right[pad + i] += word.right[i];
    }
    out
}

/// Pink noise by Paul Kellet's filter over white noise.
fn pink(len: usize, rng: &mut fastrand::Rng) -> Vec<f32> {
    let mut b = [0f32; 7];
    (0..len)
        .map(|_| {
            let white = rng.f32() * 2.0 - 1.0;
            b[0] = 0.99886 * b[0] + white * 0.0555179;
            b[1] = 0.99332 * b[1] + white * 0.0750759;
            b[2] = 0.96900 * b[2] + white * 0.153852;
            b[3] = 0.86650 * b[3] + white * 0.3104856;
            b[4] = 0.55000 * b[4] + white * 0.5329522;
            b[5] = -0.7616 * b[5] - white * 0.016898;
            let y = b.iter().sum::<f32>() + white * 0.5362;
            b[6] = white * 0.115926;
            y * 0.11
        })
        .collect()
}

/// One trial: which word, which voice, which rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trial {
    pub set: usize,
    pub word: usize,
    pub voice: usize,
    pub rendering: usize,
}

impl Trial {
    pub fn target(&self) -> &'static str {
        SETS[self.set][self.word]
    }

    pub fn choices(&self) -> [&'static str; 6] {
        SETS[self.set]
    }
}

/// A session of `per_rendering * renderings` trials. Every rendering gets
/// the same number of trials, the same sets and the same voices as far as
/// the counts allow, in shuffled order, and no two trials in a row come
/// from the same set (so a word is never heard twice back to back).
pub fn plan(
    per_rendering: usize,
    renderings: usize,
    voices: usize,
    rng: &mut fastrand::Rng,
) -> Vec<Trial> {
    let voices = voices.max(1);
    let mut trials = Vec::with_capacity(per_rendering * renderings);
    for rendering in 0..renderings {
        // Rotate the starting set and voice per rendering so they spread.
        let mut sets: Vec<usize> = (0..SETS.len()).collect();
        rng.shuffle(&mut sets);
        for i in 0..per_rendering {
            trials.push(Trial {
                set: sets[i % sets.len()],
                word: rng.usize(0..6),
                voice: (i + rendering) % voices,
                rendering,
            });
        }
    }
    rng.shuffle(&mut trials);
    // Break up neighbours from the same set: swap the second of the pair
    // with any trial elsewhere, where neither lands next to its own set.
    let n = trials.len();
    let fits = |t: &[Trial], pos: usize, set: usize| {
        (pos == 0 || t[pos - 1].set != set) && (pos + 1 >= t.len() || t[pos + 1].set != set)
    };
    for i in 1..n {
        if trials[i].set != trials[i - 1].set {
            continue;
        }
        let swap = (0..n).find(|&j| {
            j.abs_diff(i) > 1 && fits(&trials, i, trials[j].set) && fits(&trials, j, trials[i].set)
        });
        if let Some(j) = swap {
            trials.swap(i, j);
        }
    }
    trials
}

/// Standard base64, for handing PowerShell its script as one argument.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// One answered trial, as saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub word: String,
    pub chosen: String,
    pub voice: String,
    pub rendering: String,
}

impl Answer {
    pub fn correct(&self) -> bool {
        self.word == self.chosen
    }
}

/// A finished session, one line of `word-results.jsonl`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub taken_unix: u64,
    pub device: String,
    /// Background-noise level relative to the word; `None` = quiet.
    pub snr_db: Option<f32>,
    pub answers: Vec<Answer>,
}

/// Correct and total per rendering, in order of first appearance.
pub fn scores(answers: &[Answer]) -> Vec<(String, usize, usize)> {
    let mut out: Vec<(String, usize, usize)> = Vec::new();
    for a in answers {
        let row = match out.iter().position(|r| r.0 == a.rendering) {
            Some(i) => &mut out[i],
            None => {
                out.push((a.rendering.clone(), 0, 0));
                out.last_mut().expect("just pushed")
            }
        };
        row.1 += a.correct() as usize;
        row.2 += 1;
    }
    out
}

/// The most common mistakes as `("ship", "sip", count)`: heard-as pairs.
pub fn confusions(answers: &[Answer], top: usize) -> Vec<(String, String, usize)> {
    let mut out: Vec<(String, String, usize)> = Vec::new();
    for a in answers.iter().filter(|a| !a.correct()) {
        match out.iter_mut().find(|r| r.0 == a.word && r.1 == a.chosen) {
            Some(r) => r.2 += 1,
            None => out.push((a.word.clone(), a.chosen.clone(), 1)),
        }
    }
    out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    out.truncate(top);
    out
}

pub fn results_path() -> PathBuf {
    profile::data_dir().join("word-results.jsonl")
}

pub fn load_sessions() -> Vec<Session> {
    std::fs::read_to_string(results_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn save_session(session: &Session) -> Result<(), String> {
    let mut text = std::fs::read_to_string(results_path()).unwrap_or_default();
    text.push_str(&serde_json::to_string(session).map_err(|e| e.to_string())?);
    text.push('\n');
    std::fs::write(results_path(), text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_set_is_six_distinct_words() {
        for set in SETS {
            let mut words = set.to_vec();
            words.sort();
            words.dedup();
            assert_eq!(words.len(), 6, "{set:?}");
            assert!(
                set.iter()
                    .all(|w| w.chars().all(|c| c.is_ascii_lowercase()))
            );
        }
    }

    #[test]
    fn a_plan_is_balanced_and_never_repeats_a_set_back_to_back() {
        for seed in 0..50 {
            let mut rng = fastrand::Rng::with_seed(seed);
            let trials = plan(12, 5, 2, &mut rng);
            assert_eq!(trials.len(), 60);
            for r in 0..5 {
                let mine: Vec<&Trial> = trials.iter().filter(|t| t.rendering == r).collect();
                assert_eq!(mine.len(), 12);
                // Each rendering sees every set once and both voices equally.
                let mut sets: Vec<usize> = mine.iter().map(|t| t.set).collect();
                sets.sort();
                assert_eq!(sets, (0..12).collect::<Vec<_>>());
                assert_eq!(mine.iter().filter(|t| t.voice == 0).count(), 6);
            }
            for pair in trials.windows(2) {
                assert_ne!(pair[0].set, pair[1].set, "seed {seed}");
            }
        }
    }

    #[test]
    fn trimming_and_normalising_give_a_tight_clip_at_the_target_level() {
        let sr = 22_050;
        let mut x = vec![0.0; sr];
        x.extend((0..sr / 2).map(|i| 0.4 * (i as f32 * 0.3).sin()));
        x.extend(vec![0.0; sr]);
        let clip = Clip {
            sr: sr as u32,
            left: x.clone(),
            right: x,
        };
        let tight = trim_silence(&clip);
        assert!((tight.seconds() - 0.54).abs() < 0.01, "{}", tight.seconds());
        let level = crate::dsp::lin_to_db(normalise(&tight).rms());
        assert!((level - WORD_RMS_DB).abs() < 0.1, "{level}");
        // Silence stays silence instead of being amplified.
        let silent = Clip {
            sr: 8_000,
            left: vec![0.0; 100],
            right: vec![0.0; 100],
        };
        assert_eq!(normalise(&trim_silence(&silent)), silent);
    }

    #[test]
    fn noise_is_added_at_the_asked_level_around_the_word() {
        let sr = 48_000u32;
        let x: Vec<f32> = (0..sr as usize / 2)
            .map(|i| 0.1 * (i as f32 * 0.2).sin())
            .collect();
        let word = Clip {
            sr,
            left: x.clone(),
            right: x,
        };
        let mut rng = fastrand::Rng::with_seed(7);
        let mixed = in_noise(&word, 6.0, &mut rng);
        assert_eq!(mixed.len(), word.len() + sr as usize / 2);
        assert_eq!(mixed.left, mixed.right);
        // The lead-in is noise alone, 6 dB under the word.
        let lead = &mixed.left[2_000..10_000];
        let lead_rms = (lead.iter().map(|v| v * v).sum::<f32>() / lead.len() as f32).sqrt();
        let snr = crate::dsp::lin_to_db(word.rms()) - crate::dsp::lin_to_db(lead_rms);
        assert!((snr - 6.0).abs() < 3.0, "snr {snr}");
        assert_eq!(mixed.left[0], 0.0);
    }

    #[test]
    fn base64_matches_the_standard_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
    }

    #[test]
    fn scoring_counts_per_rendering_and_ranks_confusions() {
        let a = |word: &str, chosen: &str, rendering: &str| Answer {
            word: word.into(),
            chosen: chosen.into(),
            voice: "v".into(),
            rendering: rendering.into(),
        };
        let answers = vec![
            a("ship", "sip", "Original"),
            a("sip", "sip", "Original"),
            a("ship", "sip", "EQ"),
            a("thin", "fin", "EQ"),
            a("tea", "tea", "EQ"),
        ];
        assert_eq!(
            scores(&answers),
            vec![("Original".to_string(), 1, 2), ("EQ".to_string(), 1, 3)]
        );
        assert_eq!(
            confusions(&answers, 5),
            vec![
                ("ship".to_string(), "sip".to_string(), 2),
                ("thin".to_string(), "fin".to_string(), 1)
            ]
        );
    }

    #[test]
    fn the_synthesis_script_names_every_word_once_and_quotes_the_path() {
        let script = synth_script(Path::new(r"C:\it's here\words"));
        assert!(script.contains(r"$out = 'C:\it''s here\words'"));
        assert_eq!(script.matches("'ship'").count(), 1);
        assert_eq!(script.matches("'pat'").count(), 1);
        assert!(script.contains("SetOutputToWaveFile"));
        assert!(!script.contains("SetOutputToDefaultAudioDevice"));
    }
}
