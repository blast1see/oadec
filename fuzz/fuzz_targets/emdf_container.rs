//! An EMDF container in both its carriages: `oadec_fuzz::targets::emdf_container` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::emdf_container(data));
