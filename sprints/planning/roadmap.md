# Roadmap

> The general plan for this project. Keep it current; detail lives in the
> sprint records.

## Now

- **Word-test results.** Sprint 001 built the offline tool; its first real
  use showed the loss is a dip around 3–4 kHz, possibly tinnitus masking as
  much as loss, and that preference ratings on a familiar clip measure
  familiarity. Sprint 002 built the fairer measure (a word-recognition
  test), a tinnitus pitch match, and presets shaped to the dip. What is
  needed now is data: a few word-test sessions, in quiet and in noise.
- The outcome is a decision between EQ alone, EQ that spares the tinnitus
  pitch, and copying the dip's band lower down.

## Next

- **Live system audio on the headset.** Wait for the word-test results: if
  EQ alone wins, this needs no custom real-time engine. Decide the route first — processing
  hosted inside the Windows audio engine on the existing outputs, or a
  virtual audio device with our own capture/render app — then build it.
  Latency budget: video must stay in sync.
- **Separate treatment per output.** The headset exposes separate game and
  chat outputs; voices want the full treatment, game audio little or none.
- **Controls.** Quick on/off and strength, so the processing can be compared
  against bypass at any time.

## Later / Ideas

- Speakers: carry the headset-tuned settings over, then see what the room
  needs.
- Microphone chain for reading aloud: de-ess, warmth, leveling, gate, out to
  a virtual microphone. The reader must not monitor their own processed
  voice live.
- Adaptation: start mild and step the strength up over a couple of weeks.
- Speech-aware processing: apply lowering only when a consonant is present.
