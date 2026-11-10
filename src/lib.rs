//! kvimh — hearing-assist audio for Windows.
//!
//! `dsp` is the I/O-free processing core: it takes and returns sample
//! buffers and knows nothing about devices, so the same code runs offline on
//! a clip now and on live audio later. `audio` is the only module that
//! touches a sound device.

pub mod audio;
pub mod dsp;
pub mod profile;
pub mod render;
pub mod wav;
pub mod words;
