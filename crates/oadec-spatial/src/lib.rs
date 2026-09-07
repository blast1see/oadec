//! Object-audio program model and DAMF / ADM BWF / WAV / CAF writers.
//!
//! Part of the `oadec` object-audio decoder engine.

pub mod caf;
pub mod damf;
pub mod program;

pub use damf::{DamfOptions, DamfWriter};
pub use program::{ElementState, Event, Program, ProgramError, Timeline};
