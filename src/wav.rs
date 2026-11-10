//! WAV files in and out, and a plain resampler.

use std::path::Path;

use crate::render::Clip;

pub fn load(path: &Path) -> Result<Clip, String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample.clamp(8, 32) - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        }
    };
    Ok(from_interleaved(&samples, channels, spec.sample_rate))
}

/// First two channels become left and right; mono is copied to both.
pub fn from_interleaved(samples: &[f32], channels: usize, sr: u32) -> Clip {
    let channels = channels.max(1);
    let frames = samples.chunks_exact(channels);
    let left = frames.clone().map(|f| f[0]).collect();
    let right = frames
        .map(|f| f[if channels > 1 { 1 } else { 0 }])
        .collect();
    Clip { sr, left, right }
}

pub fn save(path: &Path, clip: &Clip) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: clip.sr,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for (l, r) in clip.left.iter().zip(&clip.right) {
        w.write_sample(*l)
            .and_then(|_| w.write_sample(*r))
            .map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

/// Linear-interpolation resampling. Good enough to audition a clip on a
/// device running at another rate; not what live processing should use.
pub fn resample(clip: &Clip, sr: u32) -> Clip {
    if clip.sr == sr || clip.is_empty() {
        return Clip { sr, ..clip.clone() };
    }
    let step = clip.sr as f64 / sr as f64;
    let len = (clip.len() as f64 / step) as usize;
    let one = |x: &[f32]| -> Vec<f32> {
        (0..len)
            .map(|i| {
                let pos = i as f64 * step;
                let (a, t) = (pos as usize, (pos.fract()) as f32);
                let b = (a + 1).min(x.len() - 1);
                x[a] + (x[b] - x[a]) * t
            })
            .collect()
    };
    Clip {
        sr,
        left: one(&clip.left),
        right: one(&clip.right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trip_and_mono_fan_out() {
        let dir = std::env::temp_dir().join(format!("kvimh-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        let clip = Clip {
            sr: 48_000,
            left: vec![0.0, 0.25, -0.5],
            right: vec![0.1, -0.1, 0.0],
        };
        save(&path, &clip).unwrap();
        assert_eq!(load(&path).unwrap(), clip);
        std::fs::remove_dir_all(&dir).unwrap();

        let mono = from_interleaved(&[0.1, 0.2, 0.3], 1, 44_100);
        assert_eq!(mono.left, mono.right);
        assert_eq!(mono.len(), 3);
    }

    #[test]
    fn resample_changes_length_by_the_rate_ratio() {
        let clip = Clip {
            sr: 24_000,
            left: vec![0.0; 24_000],
            right: vec![0.0; 24_000],
        };
        let up = resample(&clip, 48_000);
        assert_eq!((up.sr, up.len()), (48_000, 48_000));
        assert_eq!(resample(&clip, 24_000).len(), 24_000);
    }
}
