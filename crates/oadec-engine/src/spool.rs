//! Records that are written in one order and read back in it, held in memory
//! up to a limit and in a temporary file past it.
//!
//! A writer that cannot write what it is told as it is told it, because what
//! it writes is ordered otherwise or needs its length in front of it, keeps
//! it until the end. Kept in memory, what it keeps grows with the stream; a
//! [`Spool`] keeps the first `limit` bytes in memory and the rest in a file
//! of its own, and gives the records back in the order they were pushed.
//! Nothing is connected to it yet: it is here for the object outputs of a
//! stream read once, which are not built.
//!
//! # What it promises
//!
//! - **Order.** [`Records`] gives every record pushed, whole, in the order it
//!   was pushed, wherever the limit fell: some in memory and the rest in the
//!   file reads as if all were in one place.
//! - **Memory.** What is in memory is never more than `limit` bytes of
//!   records, with four bytes of length in front of each; the first record
//!   that would take it past `limit` goes to the file, and every one after
//!   it. A record longer than [`MAX_RECORD`] bytes is refused.
//! - **The file.** It is made under [`std::env::temp_dir`] (or the
//!   directory [`Spool::in_dir`] is given), named
//!   `oadec-spool-<process>-<count>`, with `create_new`, so a file that is
//!   there already is never written into: another name is tried, a few
//!   times. Only `std` makes and reads it.
//! - **Cleaning up.** The file is removed when the [`Spool`] or the
//!   [`Records`] that holds it is dropped, however the run ended. On Windows
//!   a file is removed once its handle is closed, so the handle is closed
//!   first. A process that is killed leaves its file behind, under the name
//!   above.
//!
//! # When something goes wrong
//!
//! - The file cannot be made (the directory is not there, is not writable,
//!   a name could not be found): the push that needed it fails with that
//!   error, which names the directory.
//! - Writing it fails, a full disk among the reasons: the push fails with
//!   that error.
//! - Either way the spool has failed: every call after it gives an error, and
//!   what was pushed is not given back. A writer that spools fails with it.

use std::fs::File;
use std::io;
use std::path::PathBuf;

/// How much a spool holds in memory when it is asked for no other limit.
pub const DEFAULT_LIMIT: usize = 16 << 20;

/// The longest record a spool takes: its length is written in four bytes.
pub const MAX_RECORD: usize = u32::MAX as usize;

/// Records held in memory up to a limit and in a temporary file past it.
#[derive(Debug)]
pub struct Spool {
    /// The records in memory, each behind its length (four bytes,
    /// little-endian).
    mem: Vec<u8>,
    /// The records past the limit, in the same form, once there are any.
    file: Option<Spilled>,
    /// Where the file is made.
    dir: PathBuf,
    /// What `mem` may hold.
    limit: usize,
    /// Records pushed.
    records: u64,
    /// A push failed, and nothing more is taken.
    failed: bool,
}

/// The temporary file of a spool, removed when it is dropped.
#[derive(Debug)]
struct Spilled {
    /// Closed before the file is removed.
    file: Option<File>,
    path: PathBuf,
}

impl Drop for Spilled {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Spool {
    /// A spool that holds `limit` bytes in memory, and the rest in a file
    /// under [`std::env::temp_dir`].
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self::in_dir(limit, std::env::temp_dir())
    }

    /// [`Spool::new`], with its file in `dir`: a directory of the caller's,
    /// or one that is not there, for a test of what a spool does when its
    /// file cannot be made.
    #[must_use]
    pub fn in_dir(limit: usize, dir: PathBuf) -> Self {
        Self {
            mem: Vec::new(),
            file: None,
            dir,
            limit,
            records: 0,
            failed: false,
        }
    }

    /// Records pushed so far.
    #[must_use]
    pub fn records(&self) -> u64 {
        self.records
    }

    /// What it holds in memory may hold at most.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Whether records went to the file.
    #[must_use]
    pub fn spilled(&self) -> bool {
        self.file.is_some()
    }

    /// Bytes held in memory now, lengths among them.
    #[must_use]
    pub fn in_memory(&self) -> usize {
        self.mem.len()
    }

    /// Takes the next record.
    ///
    /// # Errors
    /// A record longer than [`MAX_RECORD`]; what making or writing the file
    /// gives; and an error at every call after one failed.
    pub fn push(&mut self, record: &[u8]) -> io::Result<()> {
        let _ = (record, self.failed, &self.dir);
        Err(crate::pipeline::not_yet("a spool"))
    }

    /// Ends the pushing; what is given back reads the records in the order
    /// they were pushed.
    ///
    /// # Errors
    /// What flushing the file gives, and an error where a push failed.
    pub fn finish(self) -> io::Result<Records> {
        Err(crate::pipeline::not_yet("a spool"))
    }
}

/// The records of a spool, read back in the order they were pushed.
#[derive(Debug)]
pub struct Records {
    mem: Vec<u8>,
    /// Where the next record in memory begins.
    at: usize,
    file: Option<Spilled>,
}

impl Records {
    /// Puts the next record into `into`, which it clears first; `false`
    /// where there is none left.
    ///
    /// # Errors
    /// What reading the file gives, and a record cut short in it.
    pub fn next_record(&mut self, into: &mut Vec<u8>) -> io::Result<bool> {
        let _ = (into, self.at, &self.mem, &self.file);
        Err(crate::pipeline::not_yet("reading a spool back"))
    }
}
