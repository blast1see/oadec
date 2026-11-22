//! A stream told to the stages of a pass that reads it once, in reads of any length and whole: `oadec_fuzz::targets::pipeline_push` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::pipeline_push(data));
