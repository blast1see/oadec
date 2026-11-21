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

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
    handle: Option<Handle>,
    path: PathBuf,
}

/// The file of a spool, being written or being read back.
#[derive(Debug)]
enum Handle {
    Writing(BufWriter<File>),
    Reading(BufReader<File>),
}

impl Spilled {
    /// Makes a file of its own in `dir`, under a name no other file has.
    fn make(dir: &Path) -> io::Result<Self> {
        /// Names tried before a spool gives up on finding one.
        const TRIES: u32 = 16;
        static MADE: AtomicU64 = AtomicU64::new(0);
        let mut last = None;
        for _ in 0..TRIES {
            let n = MADE.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!("oadec-spool-{}-{n}", std::process::id()));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        handle: Some(Handle::Writing(BufWriter::new(file))),
                        path,
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(in_dir(dir, &e)),
            }
        }
        Err(in_dir(
            dir,
            &last.unwrap_or_else(|| io::Error::other("no name was free")),
        ))
    }

    /// Writes `bytes` behind what was written.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match &mut self.handle {
            Some(Handle::Writing(w)) => w.write_all(bytes),
            _ => Err(io::Error::other("the spool file is not being written")),
        }
    }

    /// Ends the writing: what was written is read back from its beginning.
    fn rewind(&mut self) -> io::Result<()> {
        let Some(Handle::Writing(w)) = self.handle.take() else {
            return Err(io::Error::other("the spool file is not being written"));
        };
        let mut file = w.into_inner().map_err(io::IntoInnerError::into_error)?;
        file.seek(SeekFrom::Start(0))?;
        self.handle = Some(Handle::Reading(BufReader::new(file)));
        Ok(())
    }
}

/// What a file that could not be made in `dir` says.
fn in_dir(dir: &Path, e: &io::Error) -> io::Error {
    io::Error::new(
        e.kind(),
        format!("making a spool file in {}: {e}", dir.display()),
    )
}

impl Drop for Spilled {
    fn drop(&mut self) {
        // the handle is closed first: Windows removes no file that is open
        drop(self.handle.take());
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
        if record.len() > MAX_RECORD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "a record of {} bytes is longer than a spool takes",
                    record.len()
                ),
            ));
        }
        if self.failed {
            return Err(failed());
        }
        let len = (record.len() as u32).to_le_bytes();
        if self.file.is_none() && self.mem.len() + len.len() + record.len() <= self.limit {
            self.mem.extend_from_slice(&len);
            self.mem.extend_from_slice(record);
            self.records += 1;
            return Ok(());
        }
        let written = self.spill(&len, record);
        self.failed = written.is_err();
        if written.is_ok() {
            self.records += 1;
        }
        written
    }

    /// Writes a record into the file, which is made for the first.
    fn spill(&mut self, len: &[u8], record: &[u8]) -> io::Result<()> {
        if self.file.is_none() {
            self.file = Some(Spilled::make(&self.dir)?);
        }
        let file = self.file.as_mut().expect("made above");
        file.write(len)?;
        file.write(record)
    }

    /// Ends the pushing; what is given back reads the records in the order
    /// they were pushed.
    ///
    /// # Errors
    /// What flushing the file gives, and an error where a push failed.
    pub fn finish(mut self) -> io::Result<Records> {
        if self.failed {
            return Err(failed());
        }
        let mut file = self.file.take();
        if let Some(file) = &mut file {
            file.rewind()?;
        }
        Ok(Records {
            mem: std::mem::take(&mut self.mem),
            at: 0,
            file,
        })
    }
}

/// What a spool that failed says to every call after.
fn failed() -> io::Error {
    io::Error::other("the spool failed earlier and takes nothing more")
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
        into.clear();
        if let Some(len) = self.mem.get(self.at..self.at + 4) {
            let len = u32::from_le_bytes(len.try_into().expect("four bytes")) as usize;
            let from = self.at + 4;
            into.extend_from_slice(&self.mem[from..from + len]);
            self.at = from + len;
            return Ok(true);
        }
        let Some(Handle::Reading(file)) = self.file.as_mut().and_then(|f| f.handle.as_mut()) else {
            return Ok(false);
        };
        let mut len = [0u8; 4];
        let mut held = 0;
        while held < len.len() {
            match file.read(&mut len[held..]) {
                Ok(0) if held == 0 => return Ok(false),
                Ok(0) => return Err(cut_short()),
                Ok(n) => held += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let len = u32::from_le_bytes(len) as usize;
        into.resize(len, 0);
        file.read_exact(into).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                cut_short()
            } else {
                e
            }
        })?;
        Ok(true)
    }
}

/// What a record cut short in the file of a spool says.
fn cut_short() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "a record in the file of a spool is cut short",
    )
}
