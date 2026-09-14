//! Object-audio program model and DAMF / ADM BWF / WAV / CAF writers.
//!
//! Part of the `oadec` object-audio decoder engine.

pub mod adm;
pub mod caf;
pub mod damf;
pub mod dbmd;
pub mod loss;
pub mod program;

pub use adm::{AdmError, AdmOptions, AdmSummary, AdmWriter, Interpolation};
pub use damf::{DamfError, DamfOptions, DamfWriter};
pub use loss::{LossClass, LossKind, LossLedger};
pub use program::{ElementState, Event, IsfPolicy, Program, ProgramError, Timeline};
