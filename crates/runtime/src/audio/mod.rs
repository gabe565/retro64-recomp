//! Audio: a software mixer reproducing FMOD 3.61's behaviour for the calls the
//! game makes (mixer.rs), XM music through libxmp, and SDL3 audio output
//! (device.rs). The FMOD import surface is in crate::fmod.

pub mod device;
pub mod mixer;
pub mod wav;
