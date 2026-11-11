//! A TrueHD or MLP stream through the decoder with the checksums put right first: `oadec_fuzz::targets::truehd_decode_repaired` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::truehd_decode_repaired(data));
