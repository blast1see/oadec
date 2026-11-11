//! A JOC payload, parsed and put through the reconstruction: `oadec_fuzz::targets::joc_params` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::joc_params(data));
