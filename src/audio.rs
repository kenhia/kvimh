//! The only module that talks to a sound device: playback of test tones and
//! clips, and capture of whatever the PC is currently playing.
//!
//! Safety lives here too. Every sample written to the device is clamped to
//! the render ceiling, and a test tone can never be asked for above
//! `MAX_TEST_DBFS`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

use crate::dsp::db_to_lin;
use crate::dsp::tone::PulsedTone;
use crate::profile::{Ear, MAX_TEST_DBFS};
use crate::render::{CEILING_DB, Clip};

enum Source {
    Silence,
    Tone {
        tone: PulsedTone,
        level: f32,
        target: f32,
        ear: Option<Ear>,
    },
    Clip {
        clip: Arc<Clip>,
        pos: usize,
        fade: f32,
    },
}

struct State {
    source: Source,
    sr: f32,
}

impl State {
    /// The next stereo frame, before the output clamp.
    fn next_frame(&mut self) -> Option<(f32, f32)> {
        match &mut self.source {
            Source::Silence => None,
            Source::Tone {
                tone,
                level,
                target,
                ear,
            } => {
                *level += (*target - *level) * 0.002;
                let s = tone.next_sample() * *level;
                Some(match ear {
                    Some(Ear::Left) => (s, 0.0),
                    Some(Ear::Right) => (0.0, s),
                    None => (s, s),
                })
            }
            Source::Clip { clip, pos, fade } => {
                if *pos >= clip.len() {
                    return None;
                }
                *fade = (*fade + 1.0 / (0.01 * self.sr)).min(1.0);
                let frame = (clip.left[*pos] * *fade, clip.right[*pos] * *fade);
                *pos += 1;
                Some(frame)
            }
        }
    }
}

pub struct Engine {
    _stream: cpal::Stream,
    device: cpal::Device,
    state: Arc<Mutex<State>>,
    position: Arc<AtomicUsize>,
    playing: Arc<AtomicBool>,
    pub sample_rate: u32,
    pub device_name: String,
}

/// Names of the output devices, default first.
pub fn output_devices() -> Vec<String> {
    let host = cpal::default_host();
    let mut names: Vec<String> = Vec::new();
    if let Some(name) = host.default_output_device().and_then(|d| name_of(&d)) {
        names.push(name);
    }
    if let Ok(devices) = host.output_devices() {
        for name in devices.filter_map(|d| name_of(&d)) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn name_of(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_string())
}

impl Engine {
    /// Open `device_name`, or the default output device.
    pub fn new(device_name: Option<&str>) -> Result<Engine, String> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(want) => host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| name_of(d).as_deref() == Some(want)),
            None => host.default_output_device(),
        }
        .ok_or("no such output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let sample_rate = config.sample_rate;

        let state = Arc::new(Mutex::new(State {
            source: Source::Silence,
            sr: sample_rate as f32,
        }));
        let position = Arc::new(AtomicUsize::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let shared = (state.clone(), position.clone(), playing.clone());
        let stream = match format {
            SampleFormat::F32 => output_stream::<f32>(&device, config, shared),
            SampleFormat::I16 => output_stream::<i16>(&device, config, shared),
            SampleFormat::I32 => output_stream::<i32>(&device, config, shared),
            other => Err(format!("unsupported sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        let device_name = name_of(&device).unwrap_or_else(|| "output".into());
        Ok(Engine {
            _stream: stream,
            device,
            state,
            position,
            playing,
            sample_rate,
            device_name,
        })
    }

    fn set_source(&self, source: Source) {
        if let Ok(mut st) = self.state.lock() {
            self.playing
                .store(!matches!(source, Source::Silence), Ordering::Relaxed);
            st.source = source;
        }
    }

    /// Start a pulsed tone in one ear (or both). The level is capped.
    pub fn play_tone(&self, hz: f32, dbfs: f32, ear: Option<Ear>) {
        let tone = PulsedTone::new(hz, self.sample_rate as f32);
        let target = db_to_lin(dbfs.min(MAX_TEST_DBFS));
        self.set_source(Source::Tone {
            tone,
            level: target,
            target,
            ear,
        });
    }

    /// Change the level of the tone that is playing, smoothly.
    pub fn set_tone_level(&self, dbfs: f32) {
        if let Ok(mut st) = self.state.lock()
            && let Source::Tone { target, .. } = &mut st.source
        {
            *target = db_to_lin(dbfs.min(MAX_TEST_DBFS));
        }
    }

    /// Play a clip from `pos` (in frames). The clip must be at the engine's
    /// sample rate.
    pub fn play_clip(&self, clip: Arc<Clip>, pos: usize) {
        self.position.store(pos, Ordering::Relaxed);
        self.set_source(Source::Clip {
            clip,
            pos,
            fade: 0.0,
        });
    }

    pub fn stop(&self) {
        self.set_source(Source::Silence);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Frame position of the clip being played.
    pub fn position(&self) -> usize {
        self.position.load(Ordering::Relaxed)
    }

    /// Start capturing what this device is playing.
    pub fn record_loopback(&self) -> Result<Recorder, String> {
        Recorder::start(&self.device)
    }
}

type Shared = (Arc<Mutex<State>>, Arc<AtomicUsize>, Arc<AtomicBool>);

fn output_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    shared: Shared,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let (state, position, playing) = shared;
    let channels = config.channels as usize;
    let ceiling = db_to_lin(CEILING_DB);
    let silence = T::from_sample(0.0f32);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                // If the UI holds the lock, play silence for this block
                // instead of waiting on it.
                let Ok(mut st) = state.try_lock() else {
                    data.fill(silence);
                    return;
                };
                for frame in data.chunks_mut(channels) {
                    frame.fill(silence);
                    let Some((l, r)) = st.next_frame() else {
                        if playing.swap(false, Ordering::Relaxed) {
                            st.source = Source::Silence;
                        }
                        continue;
                    };
                    let l = finite(l).clamp(-ceiling, ceiling);
                    let r = finite(r).clamp(-ceiling, ceiling);
                    if channels == 1 {
                        frame[0] = T::from_sample((l + r) * 0.5);
                    } else {
                        frame[0] = T::from_sample(l);
                        frame[1] = T::from_sample(r);
                    }
                }
                if let Source::Clip { pos, .. } = &st.source {
                    position.store(*pos, Ordering::Relaxed);
                }
            },
            |err| eprintln!("audio output: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

fn finite(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

/// A running capture of a playback device's output ("loopback").
pub struct Recorder {
    _stream: cpal::Stream,
    buf: Arc<Mutex<Vec<f32>>>,
    channels: usize,
    sr: u32,
}

impl Recorder {
    fn start(device: &cpal::Device) -> Result<Recorder, String> {
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let (channels, sr) = (config.channels as usize, config.sample_rate);
        let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
        let stream = match format {
            SampleFormat::F32 => input_stream::<f32>(device, config, buf.clone()),
            SampleFormat::I16 => input_stream::<i16>(device, config, buf.clone()),
            SampleFormat::I32 => input_stream::<i32>(device, config, buf.clone()),
            other => Err(format!("unsupported sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Recorder {
            _stream: stream,
            buf,
            channels,
            sr,
        })
    }

    pub fn seconds(&self) -> f32 {
        let samples = self.buf.lock().map(|b| b.len()).unwrap_or(0);
        samples as f32 / (self.channels.max(1) as f32 * self.sr as f32)
    }

    pub fn finish(self) -> Clip {
        let samples = self.buf.lock().map(|b| b.clone()).unwrap_or_default();
        crate::wav::from_interleaved(&samples, self.channels, self.sr)
    }
}

fn input_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    buf: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                if let Ok(mut b) = buf.lock() {
                    b.extend(data.iter().map(|s| f32::from_sample(*s)));
                }
            },
            |err| eprintln!("audio capture: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}
