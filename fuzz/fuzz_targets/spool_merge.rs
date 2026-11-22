//! Records given to a spool, kept in memory and spilled to a file, and read back: `oadec_fuzz::targets::spool_merge` says what has to hold.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| oadec_fuzz::targets::spool_merge(data));
