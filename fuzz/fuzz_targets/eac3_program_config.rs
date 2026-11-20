//! The programme decoder under each configuration: `oadec_fuzz::targets::eac3_program_config` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::eac3_program_config(data));
