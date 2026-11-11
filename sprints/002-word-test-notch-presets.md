# 002 — Word-recognition test and notch-aware presets

korg proposal 4513 · work items 4511 (word test), 4512 (retest, outliers,
presets, tinnitus) · branch `002-word-test-notch-presets`

## Why

Sprint 001's tool got its first real use and taught three things.

1. **The blind test measured familiarity.** The listener rated the original
   highest, and said why: a voice he knows well, on a clip he had heard
   several times. Preference on familiar, repeated material favours whatever
   is familiar.
2. **The loss is a dip, not a missing top end.** The hearing check shows a
   notch around 3–4 kHz with clearer hearing on both sides. Sprint 001
   assumed a loss that falls away with frequency; for a notch its suggestion
   logic clamped to the mildest lowering it had, so the blind test barely
   exercised lowering.
3. **It may be masking as much as loss.** With tinnitus, a constant ringing
   can cover tones near its own pitch, and sounds near that pitch can be
   actively uncomfortable.

The third changes what "help" means. Plain gain in the notch is the textbook
answer to the audiogram — and may be exactly the wrong thing if the notch is
where the ringing is.

## What shipped

**A word test (new screen 5).** One word, said once, in a synthetic voice
the listener has never heard; pick it from six that differ in one consonant
(sip / ship / tip / chip / zip / hip). Twelve sets, two voices, sixty trials
a session. Each trial is processed one of five ways at random and the
listener is not told which. Optional background noise (pink, +6 dB or 0 dB
word-to-noise) for when quiet is too easy. Scores are percent correct per
rendering — this session, all quiet sessions, all noisy sessions — plus the
most common mix-ups ("ship" heard as "sip").

**Hearing check additions.**
- Retest any single tone from a row of buttons; the new reading replaces the
  old one and nothing else changes.
- A tone heard much more easily than both neighbours is flagged. That is
  what a key press on ringing instead of the tone looks like.
- An optional **tinnitus pitch match** per ear: adjust a pulsed tone until
  it sits on the ringing. Stored in the profile, drawn on the chart. An ear
  can hold several tones — the first real use found two in one ear, and the
  first build could only save one. Each tone gets its own EQ guard. Profiles
  saved by the single-tone build still load.

**Presets rebuilt from the measured shape.** Original · EQ at half the
estimated loss · EQ at 80 % · EQ *sparing the tinnitus pitch* (or frequency
compression, if no pitch was matched) · the notch band copied down to just
below the notch (or the top octave copied down, if there is no notch).

**Listen screen.** A "leave the band around my tinnitus pitch un-boosted"
option, and the transposition controls now cover a mid-band copy (source
range, how far down, level).

## Decisions

- **Words come from Windows' own speech synthesis**, written to files once
  and kept in the user data folder. No licence question, nothing personal,
  nothing in this repo, and — the point — voices he does not know. It runs
  Windows PowerShell 5.1 (where `System.Speech` lives) with the script
  passed as one encoded argument: no temp file, no quoting layers.
- **No feedback during the test and no replay.** Being told right/wrong
  teaches the word list; a replay is the repetition the test exists to
  remove.
- **Every rendering gets the same sets and the same voices** within a
  session, and no two trials in a row come from the same set.
- **Comfort is not scored by the word test.** A rendering can win on words
  and still be unpleasant. The blind test on a real clip stays for that.
- **The zero point of "loss" moved** from the best tone at or below 1 kHz to
  the median of the three best tones between 500 Hz and 2 kHz. The lowest
  tone depends too much on the headset's bass response and seal.
- **Only too-good readings are flagged as outliers.** A tone *worse* than
  both neighbours is what a real notch looks like and must not be flagged.
- **The tinnitus guard is "no boost", not a cut**, a third of an octave
  either side of the matched pitch. Whether a cut would be more comfortable
  is a question for after the first results.
- **The default is now EQ alone**, with lowering off. The measured loss does
  not call for squeezing the whole top end down.

## Safety

- The tinnitus-match tone is the sound most likely to be uncomfortable, so
  it is pulsed, starts at −95 dBFS, plays only when asked, and its level
  slider stops 15 dB above the listener's own measured threshold at that
  pitch (and never above the global tone cap). Tested.
- Word-test playback goes through the same limiter and the same output
  clamp as everything else.

## Verified

- `just check`: fmt, clippy with warnings denied, 44 tests (40 library, 4
  hearing-check).
- `kvimh words` on the dev machine made all 71 words in both installed
  voices in four seconds, without playing anything.
- Screens 2, 3 and 5 render in demo mode: the notch sentence, the flagged
  reading, the retest rows, the ringing marker on the chart.

## First use

The listener ran the word test, the pitch match and a retest; all worked.
One session in quiet scored at or near the ceiling for every rendering, as
predicted — quiet is too easy to separate them, so the sessions that matter
are the ones with background noise. Every miss in that session was on the
female voice.

## Not verified — the listener's part

- Several tones per ear in the pitch match (built after the first use).
- How natural the synthetic voices' consonants are. If "s" and "sh" from a
  synthesiser are easier or harder than a person's, the scores still compare
  renderings fairly, but may not transfer to real speech.
- The tinnitus pitch match and a single-tone retest, by ear.
- Whether sixty trials is enough. Twelve words per rendering is a small
  sample; the screen says so, and pools across sessions.

## Follow-ups

- If the scores in quiet sit near 100 % for every rendering, the noise
  levels need tuning; if synthetic speech proves a poor stand-in, the test
  needs recorded human words.
- The live-audio route (korg 4509) should wait for these results: if EQ
  alone wins, live processing needs no custom engine.
