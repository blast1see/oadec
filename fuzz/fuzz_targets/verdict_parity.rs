//! A stream through every command that reads one, with its checksums put right first: `oadec_fuzz::targets::verdict_parity` says what has to hold between their answers.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::verdict_parity(data));
