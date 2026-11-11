//! A run of access units through the unit and segment parsers: `oadec_fuzz::targets::truehd_unit` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::truehd_unit(data));
