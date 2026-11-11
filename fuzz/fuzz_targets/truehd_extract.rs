//! Framing access units out of a byte stream: `oadec_fuzz::targets::truehd_extract` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::truehd_extract(data));
