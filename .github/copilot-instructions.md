<!-- kproject:begin — managed by kprojects; do not edit inside this block -->

## kproject conventions

This project uses the kproject minimal harness
(<https://github.com/kenhia/kprojects>). Keep context small; prefer doing
over ceremony.

### Layout

- `sprints/` — the project's evolution, one record per PR-sized unit of
  work (a "sprint")
  - `planning/` — planning docs; at minimum `roadmap.md` (the general plan)
  - `review/` — more formal reviews as the project matures
  - sprint records: `###-<short-name>.md` for small projects, or a
    `###-<short-name>/` directory of files for larger/more formal ones
  - a sprint record is one informal narrative: goal, decisions, what
    shipped, follow-ups — written during the sprint, not after
  - projects that deploy end the record with a `## Deployed` section:
    what shipped, where, when, and what was verified live — appended
    after the deploy, not predicted before it
- `docs/` — project documentation, architecture, usage
- `.scratch/` — git-ignored scratch space for user or agent ephemera;
  use it instead of /tmp
- `justfile` — dev recipes; default recipe is `@just --list`; `just check`
  runs the CI gates; `just deploy` (or variants) if the project deploys
- `.env` — git-ignored; tokens and environment vars

### Workflow

- One sprint ≈ one PR. Sprint proposals and work items are managed in
  `korg`; durable cross-project knowledge goes in `klams`.
- Mark each work item resolved as its work completes — don't batch the
  resolutions into sprint-ship. A proposal's progress should be readable
  while the sprint is running, which is the only time it is useful.
- If the korg or klams MCP tools are unavailable in your session, say so
  up front — don't silently work around missing infrastructure.
- A few projects share contract surfaces with siblings and have a
  **guiding plan** constraining how those change; most have none, and one
  grep is the whole cost of finding out. Grep the `index.md` routing
  table in `kai:~/src/tools/cross-project-planning` — a local path on
  kai, read through kaed from any other host (`root: "kai:src"`, path
  `tools/cross-project-planning/…`); don't clone a second copy. Not
  listed → nothing applies. Listed → read the mapped plan folder before
  planning sessions and before changing a contract surface it names, and
  amend the plan in the same ship when what you build diverges from it.
- TDD preferred: write the failing test first when practical.

### Tooling preferences

- Rust managed by `cargo`; format with `cargo fmt`, lint with
  `cargo clippy --all-targets` (test targets included deliberately — a gate
  that skips them is a gate that lies)
- Mirror `rust-toolchain.toml`, `rustfmt.toml` and `clippy.toml` from a
  sibling homelab repo rather than generating them
- License is MIT unless specifically directed otherwise

<!-- kproject:end -->

## Project

kvimh ("voices in my head") is hearing-assist audio for Windows. The listener
has high-frequency hearing loss and loses speech consonants (energy at about
3–8 kHz). The core technique is **frequency lowering**: leave everything below
a cutoff alone and compress or shift the band above it down into the audible
range, with an EQ shaped to the listener's measured thresholds and compression
around it. korg project `kvimh`; the design reasoning is in that project's
notes.

**Status:** the offline tool is built — hearing check with tinnitus pitch
match, listen-and-tweak screen, blind A/B test, word-recognition test. The
measured loss turned out to be a dip around 3–4 kHz (possibly tinnitus
masking), so the open question is EQ vs. EQ sparing the tinnitus pitch vs.
moving that band — the word test decides. Nothing processes live audio yet.

### Layout

- `src/dsp/` — the I/O-free core: `spectral` (frequency lowering + EQ in one
  STFT), `compressor`, `limiter`, `tone`.
- `src/profile.rs` — hearing profile, estimated loss, EQ points, suggested
  lowering. `src/render.rs` — offline chain and the blind-test presets.
- `src/audio.rs` — the only module that touches a sound device (cpal).
- `src/words.rs` — the word test: word sets, speech synthesis to files,
  noise, trial plan, scoring.
- `src/check.rs`, `src/app.rs`, `src/app/` — the hearing-check state machine
  and the egui window, one file per screen that outgrew `app.rs` (binary
  crate).
- The listener's profile, clips and results live in the user data folder
  (`%APPDATA%\kvimh`), never in the repo.

### Build and test

- `just check` — `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`.
- `just run` opens the window. `kvimh devices` and `kvimh render <in.wav> <dir>`
  make no sound; `--tab <name> --demo` opens a screen with made-up data.
- **Never play audio from an agent session** — it comes out of the listener's
  headset. Verify with tests, `devices`, `render`, and demo-mode screenshots.
- Windows only, developed on the machine it runs on. Audio cannot be judged
  by a test: DSP gets unit tests on synthetic signals (a tone at X comes out
  at Y, the limiter never exceeds its ceiling), and whether it *helps* is
  decided by the listener, by ear.

### Read first

- `sprints/planning/roadmap.md` — the phases and what is decided vs. open.
- `README.md` — the public statement of what this is.

### Rules that are not derivable from the code

- **The output limiter is a safety requirement, not polish.** Nothing reaches
  the listener's ears without passing a hard ceiling. This tool must never
  add to the hearing damage. Test tones start quiet and step up.
- **Keep the DSP core free of I/O.** The same processing has to run offline
  on a file (A/B test), live on system audio, and later on a microphone, so
  it takes and returns sample buffers and knows nothing about devices.
- **No allocation, locking or blocking in the audio path** once live
  processing exists.
- **The reader never monitors the processed microphone live** (roadmap,
  Later): delayed own-voice feedback disrupts speech.
- **This repo is public.** No homelab host names, tailnet endpoints, ports
  or tokens in anything committed, and keep personal detail about the
  listener to what the README already says.
