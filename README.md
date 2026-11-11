# kvimh

*Voices in my head.* Hearing-assist audio for Windows.

With high-frequency hearing loss you can tell that someone is speaking but
lose which word it was: the consonants that separate words (s, f, th, sh, t,
k) carry most of their energy between about 3 and 8 kHz, above where the
hearing still works. Turning the treble up does not bring back a band that is
gone, and lowering the whole voice makes everyone sound slowed down.

kvimh takes the approach hearing aids call **frequency lowering**: everything
below a cutoff is left alone, and the band above it is compressed or shifted
down into the range the listener can still hear. Around that sit an equalizer
shaped to the listener's own measured thresholds, and compression with a hard
output limiter so quiet consonants come up without loud sounds getting louder.

## Status

Step 1 of the plan is built: an offline tool with a hearing check (including
an optional tinnitus pitch match), a listen-and-tweak screen, a blind A/B
test and a word-recognition test. Nothing processes live system audio yet.

The first real use changed the picture: not every high-frequency loss is a
missing top end. A dip in the middle of the consonant range, or a ringing
that masks it, wants different handling — gain in the dip, gain that spares
the ringing's pitch, or moving that band somewhere clearer. The word test
exists to find out which.

```sh
just run            # open the window
just run devices    # list output devices; plays nothing
```

The listener's hearing profile, captured clips and test results are kept in
the user data folder (`%APPDATA%\kvimh`), not in this repository.

## Plan

1. **Hearing check and A/B listening test.** Measure per-ear thresholds
   through the listener's own headset, then render speech clips several ways
   and let the listener pick what helps. Offline; no drivers, no changes to
   the audio setup.
2. **Live system audio.** Run everything the PC plays through the chosen
   processing, with low enough latency that video stays in sync.
3. **Microphone chain.** The same engine pointed the other way, to clean up
   and level a voice for reading aloud to remote listeners.

The roadmap is in [sprints/planning/roadmap.md](sprints/planning/roadmap.md).

## Not a medical device

kvimh is a personal tool, not a hearing aid and not a substitute for an
audiologist. Its hearing check is a relative measurement on one headset, not a
clinical audiogram.

## Development

Rust, on Windows. The repo uses the
[kprojects](https://github.com/kenhia/kprojects) harness:

```sh
just          # list recipes
just check    # fmt, clippy, tests — the gate
```

## License

MIT — see [LICENSE](LICENSE).
