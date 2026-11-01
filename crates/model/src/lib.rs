//! Shared data model: timestamped device events and the session clock.
//!
//! This crate does no I/O, so devices, recordings, analysis and tests can all
//! depend on it without pulling in hardware or storage.

mod clock;
mod event;

pub use clock::Clock;
pub use event::{Device, Event, EventKind, Heart, Link, Pm5Force, Pm5Status, StrokeState};
