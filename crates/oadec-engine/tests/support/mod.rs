//! Test support for JOC downmix configurations 1 and 2.
//!
//! Nothing here shares code with the decoder or its parsers. The JOC payload
//! is written by [`joc_writer`] with a bit writer of its own ([`bits`]) and a
//! copy of the Huffman trees of Annex A.1 of its own, from the syntax of
//! clause 6.2 of ETSI TS 103 420 V1.2.1; [`reference`] reads what
//! `tools/joc_reference.py` computed from the clauses 6.5 and 6.6 alone.
//!
//! [`carrier`] puts a payload into an E-AC-3 stream. That is surgery on the
//! frame, not on JOC, and it leans on the frame parser of `oadec-eac3` only
//! to learn where a skip field is and on its `repair::fix_crcs` to close the
//! frame. The EMDF container it writes carries no protection word: the
//! protection is keyed, nothing here computes or invents one, and oadec reads
//! a container without looking at it (`oadec_emdf::container::parse`). A
//! stream made here is a test vector for oadec, never one a Dolby decoder is
//! claimed to accept.

#![allow(dead_code, reason = "each test binary uses a part of the support")]

pub mod bits;
pub mod carrier;
pub mod joc_writer;
pub mod reference;
pub mod signal;
