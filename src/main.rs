//! kvimh — hearing-assist audio for Windows.
//!
//! `kvimh` opens the window (`--tab <name>` to open on a screen, `--demo`
//! for made-up data). `kvimh render <in.wav> <out-dir>` writes every
//! blind-test rendering of a clip to WAV files without touching a sound
//! device. `kvimh devices` lists the output devices and checks each one
//! opens, without playing anything.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod check;

use std::path::Path;

use eframe::egui;
use kvimh::profile::Profile;
use kvimh::{render, wav};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("render") {
        let code = match (args.get(2), args.get(3)) {
            (Some(input), Some(out_dir)) => {
                match render_all(Path::new(input), Path::new(out_dir)) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("kvimh: {e}");
                        1
                    }
                }
            }
            _ => {
                eprintln!("usage: kvimh render <in.wav> <out-dir>");
                2
            }
        };
        std::process::exit(code);
    }

    if args.get(1).map(String::as_str) == Some("devices") {
        devices();
        return Ok(());
    }

    if args.get(1).map(String::as_str) == Some("words") {
        // Make the word-test recordings (files only, no sound) and report.
        match kvimh::words::prepare() {
            Ok(voices) => println!("word recordings ready: {}", voices.join(", ")),
            Err(e) => {
                eprintln!("kvimh: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // `--tab check|listen|blind|words` opens on that screen; `--demo` shows the
    // screens with made-up data instead of the listener's own.
    let demo = args.iter().any(|a| a == "--demo");
    let tab = args
        .iter()
        .position(|a| a == "--tab")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("kvimh")
            .with_inner_size([780.0, 720.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "kvimh",
        options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(&tab, demo)))),
    )
}

/// List the output devices and check, silently, that each can be opened for
/// playback and for capturing what it plays.
fn devices() {
    for name in kvimh::audio::output_devices() {
        match kvimh::audio::Engine::new(Some(&name)) {
            Ok(engine) => {
                let capture = match engine.record_loopback() {
                    Ok(_) => "capture ok".to_string(),
                    Err(e) => format!("capture FAILED: {e}"),
                };
                println!("{name}: {} Hz, playback ok, {capture}", engine.sample_rate);
            }
            Err(e) => println!("{name}: FAILED to open: {e}"),
        }
    }
}

fn render_all(input: &Path, out_dir: &Path) -> Result<(), String> {
    let clip = wav::load(input)?;
    let profile = Profile::load();
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    for (i, (name, settings)) in render::presets(profile.as_ref()).iter().enumerate() {
        let slug: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let path = out_dir.join(format!("{i}-{slug}.wav"));
        wav::save(&path, &render::render(&clip, settings, profile.as_ref()))?;
        println!("{}", path.display());
    }
    Ok(())
}
