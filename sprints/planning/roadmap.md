# Roadmap

> The general plan for this project. Keep it current; detail lives in the
> sprint records.

## Now

- **Listening.** Sprint 001 built the offline tool (hearing check, listen
  and tweak, blind test — see `sprints/001-hearing-check-ab-test.md`). What
  it is for has not happened yet: the listener runs the check, captures a
  few hard voices, and does several blind rounds. The outcome is a decision:
  does this help, and with which settings.
- Fix whatever that first real use turns up.

## Next

- **Live system audio on the headset.** Decide the route first — processing
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
