//! A stream decoded as it arrives, in pieces and whole: `oadec_fuzz::targets::stream_push` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::stream_push(data));
