//! One AC-3 or E-AC-3 syncframe through the core decoder: `oadec_fuzz::targets::eac3_frame` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::eac3_frame(data));
