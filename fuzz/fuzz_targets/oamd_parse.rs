//! An object audio metadata payload: `oadec_fuzz::targets::oamd_parse` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::oamd_parse(data));
