# 001 — Hearing check and A/B listening test

korg proposal 4510 · work items 4506 (hearing check), 4507 (DSP core),
4508 (A/B test) · branch `001-hearing-check-ab-test`

## Goal

One small offline desktop tool that answers a question before any live-audio
work starts: **does frequency lowering help this listener, and with which
settings?** No drivers, no change to the PC's audio setup.

## What shipped

A single window, four screens, in the order you use them:

1. **Setup** — pick the output device, play a reference tone, set the headset
   volume once.
2. **Hearing check** — per ear, eleven pulsed tones from 250 Hz to 12 kHz.
   Each rises from far below audibility until Space is pressed, twice: a fast
   pass in 5 dB steps, then a slow pass in 2 dB steps from 15 dB below the
   first press. Saved as a profile and drawn as an audiogram-style chart.
3. **Listen** — capture 15 s of whatever the PC is playing (or drop in a
   WAV), then flip between original and processed at the same spot while
   moving the sliders.
4. **Blind test** — the clip rendered five ways, shuffled, loudness-matched;
   rate each 1–5, reveal, and the round is appended to a results file with a
   running average per rendering.

Two command-line helpers that make no sound: `kvimh devices` (list outputs,
check each opens for playback and capture) and `kvimh render <in.wav> <dir>`
(write all five renderings to WAV). `--tab <name>` and `--demo` open the
window on a given screen with made-up data.

## Decisions

- **GUI: egui/eframe.** One native exe, no web stack, immediate-mode suits a
  tool that is mostly a state machine with a few buttons.
- **Audio I/O: cpal over WASAPI.** Also gives loopback capture for free
  (an input stream opened on an output device).
- **Frequency lowering is a phase vocoder in an STFT** (1024-sample frames at
  48 kHz, 75 % overlap, Hann analysis and synthesis). Bins below the cutoff
  are copied through untouched — that is tested to 1 % — and only bins above
  it are moved. Chosen over a filterbank because one transform does lowering
  *and* the EQ (a per-bin gain curve), and the mapping is exact enough to
  test: a 6 kHz tone with cutoff 2 kHz and ratio 2 must land at 3464 Hz.
  - *Compression*: `f_out = cutoff · (f / cutoff)^(1/ratio)`.
  - *Transposition*: copy a band down by a factor (an octave by default) and
    mix it in; the original stays.
  - Cost: 21 ms of latency, and some smearing of sharp attacks. An onset rule
    (take the source bin's phase when a bin goes from silent to sounding)
    reduces the smearing. Whether it is good enough is a listening question.
- **The EQ comes from the hearing check**, as a fraction of the estimated
  loss (0.5 = the half-gain rule), capped (20 dB default), never a cut.
  "Loss" is relative: measured threshold minus the shape of a normal ear's
  threshold curve, zeroed at the ear's best point at or below 1 kHz. That
  assumes the lows are near-normal and ignores the headset's own frequency
  response. It is a starting point for the sliders, not a prescription.
- **Suggested lowering from the profile**: find where average loss passes
  50 dB (the "usable edge"), put the cutoff at 60 % of that, and pick the
  ratio that folds 8 kHz down to the edge.
- **Loudness matching.** Every rendering is scaled to the original's RMS
  before the limiter, so the blind test compares clarity and not volume.
- **Clips come from the listener's own PC**, captured by loopback, and live
  in the user data folder with the profile and results. Nothing personal and
  nothing copyrighted enters this public repo.
- **The hearing check is a state machine behind a `TonePlayer` trait**, so it
  is tested with a fake clock and a fake player — including that no tone is
  ever requested above the cap.

## Safety

- Test tones start at −90 dBFS and stop at −10 dBFS; the cap is enforced in
  the audio layer, not just by the caller, and asserted in the check's tests.
- Every sample written to the device is clamped to −3 dBFS in the output
  callback, whatever produced it. Rendered clips also pass the limiter.
- Gaps between tones are of random length and presses during a gap are
  ignored (and counted), since tinnitus invites false presses.

## Verified

- `just check`: fmt, clippy with warnings denied, 26 tests (22 DSP/profile/
  render/WAV, 4 hearing-check).
- The window opens and all four screens render (screenshots in demo mode).
- `kvimh devices` on the dev machine: all four outputs open for playback and
  for capture.

## Not verified — this is the listener's part

Nothing here has been **heard**. No tone or clip was played during the build.
Still to be confirmed by ear:

- the hearing check end to end on the real headset;
- that capture records an actual clip (it opens; it has not recorded sound);
- original/processed playback and switching;
- whether any rendering is actually easier to follow.

## Follow-ups

- Linear-interpolation resampling is used when a dropped-in WAV is at a
  different rate from the device. Fine for auditioning; live processing must
  not use it.
- The output callback takes a mutex with `try_lock`. Acceptable for this
  offline tool; the live path needs a lock-free handoff.
- Speech-aware lowering (only when a consonant is present) is on the roadmap
  if plain lowering makes vowels sound odd.
- The live-audio route (korg 4509) is the next decision and is not part of
  this sprint.
