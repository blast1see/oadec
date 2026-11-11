//! A run of syncframes through the programme decoder: `oadec_fuzz::targets::eac3_program` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::eac3_program(data));
