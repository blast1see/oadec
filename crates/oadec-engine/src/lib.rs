//! The engine of `oadec`: what reads a Dolby TrueHD, MLP, AC-3 or E-AC-3
//! stream, says what it declares, checks it, and decodes it to samples or to
//! its object programme.
//!
//! The codecs are crates of their own (`oadec-truehd`, `oadec-eac3`,
//! `oadec-joc`, `oadec-emdf`, `oadec-spatial`). This is what runs them over a
//! whole stream: the walks, the reports, the verdict a delivery ends with and
//! the files an object output is. [`api`] is the door: a call for each thing
//! the command line does, taking a [`source::Source`] and handing back a
//! value.
//!
//! Nothing here knows of a command line, and nothing here prints. A report
//! is a value; the text of one is written to the writer its caller hands
//! over, and how far a walk has got is told to the watcher of its source.
//! The command line (`oadec-cli`) is one caller: it parses arguments, hands
//! over stdout and stderr, and draws the line on the terminal.

// the `verify --json` report is one big `json!` literal
#![recursion_limit = "256"]
// What a call finds is a value, and where it is written is the caller's: no
// function here prints, and none is allowed to.
#![deny(clippy::print_stdout, clippy::print_stderr)]
// Whatever a call takes or hands back, its caller can write the type of.
#![warn(unnameable_types)]

pub mod api;
pub mod census;
pub mod damf;
pub mod decode;
pub mod eac3;
pub mod eac3_objects;
pub mod emdf;
pub mod info;
pub mod input;
pub mod integrity;
pub mod oamd;
pub mod protection;
pub mod scan;
pub mod source;
pub mod verify;
