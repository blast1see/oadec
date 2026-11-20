//! A TrueHD stream at every presentation in one pass, against a decoder of each alone: `oadec_fuzz::targets::truehd_multi` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::truehd_multi(data));
