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

/// Saturates a sample to the signed 24-bit range; the flag says whether it
/// had to.
///
/// Every 24-bit writer goes through this one rule: the CAF audio of a DAMF
/// set, the samples of an ADM BWF file, and the PCM and WAVE output of
/// `decode`. The last used to keep the low three bytes instead, which turns a
/// sample one past the top of the range into the bottom of it.
#[must_use]
pub const fn clamp_i24(v: i32) -> (i32, bool) {
    const MIN: i32 = -(1 << 23);
    const MAX: i32 = (1 << 23) - 1;
    if v > MAX {
        (MAX, true)
    } else if v < MIN {
        (MIN, true)
    } else {
        (v, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample inside the 24-bit range passes unchanged; one outside it
    /// saturates at the nearer end and says so.
    #[test]
    fn clamp_i24_saturates_outside_the_range() {
        const MAX: i32 = (1 << 23) - 1;
        const MIN: i32 = -(1 << 23);
        for v in [0, 1, -1, MAX, MIN] {
            assert_eq!(clamp_i24(v), (v, false), "{v}");
        }
        for (v, saturated) in [
            (MAX + 1, MAX),
            (1 << 24, MAX),
            (i32::MAX, MAX),
            (MIN - 1, MIN),
            (-(1 << 24), MIN),
            (i32::MIN, MIN),
        ] {
            assert_eq!(clamp_i24(v), (saturated, true), "{v}");
        }
    }
}
