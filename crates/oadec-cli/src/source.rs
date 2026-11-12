//! Where a command reads its stream from.

use std::fs::File;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A stream a command reads: a file, or bytes that are already in memory.
///
/// A command reads its input more than once. It looks at the first bytes to
/// tell the codecs apart; most commands run the checks of `verify` over the
/// whole stream on a thread of their own, beside the pass that does their
/// work; and `verify --decode` decodes after it has scanned. So a source is
/// something that opens again at its beginning as often as it is asked, and
/// that a second thread can hold: a clone of one is the same stream.
#[derive(Debug, Clone)]
pub struct Source {
    name: String,
    from: From,
}

#[derive(Debug, Clone)]
enum From {
    File(PathBuf),
    Memory(Arc<[u8]>),
}

impl Source {
    /// The file at `path`. Nothing is opened until the source is read, so a
    /// file that is not there is the complaint of whoever reads it first.
    #[must_use]
    pub fn file(path: &Path) -> Self {
        Self {
            name: path.display().to_string(),
            from: From::File(path.to_path_buf()),
        }
    }

    /// Bytes in memory, under the name a report is to give them.
    #[must_use]
    pub fn memory(name: impl Into<String>, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            name: name.into(),
            from: From::Memory(bytes.into()),
        }
    }

    /// What reports and messages call the source: for a file, its path as it
    /// was given.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Opens the source at its beginning.
    ///
    /// # Errors
    /// What opening the file gives; bytes in memory always open.
    pub fn open(&self) -> io::Result<Reader> {
        Ok(match &self.from {
            From::File(path) => Reader::File(File::open(path)?),
            From::Memory(bytes) => Reader::Memory(Cursor::new(Arc::clone(bytes))),
        })
    }

    /// The size of the source in bytes.
    ///
    /// # Errors
    /// What asking the file system for the size of the file gives.
    pub fn size(&self) -> io::Result<u64> {
        Ok(match &self.from {
            From::File(path) => std::fs::metadata(path)?.len(),
            From::Memory(bytes) => bytes.len() as u64,
        })
    }
}

/// A [`Source`] opened for reading.
#[derive(Debug)]
pub enum Reader {
    /// A file.
    File(File),
    /// Bytes in memory.
    Memory(Cursor<Arc<[u8]>>),
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::File(file) => file.read(buf),
            Self::Memory(bytes) => bytes.read(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(source: &Source) -> Vec<u8> {
        let mut out = Vec::new();
        source
            .open()
            .expect("opened")
            .read_to_end(&mut out)
            .expect("read");
        out
    }

    #[test]
    fn bytes_in_memory_open_at_their_beginning_every_time() {
        let source = Source::memory("a stream", vec![1u8, 2, 3, 4]);
        assert_eq!(source.name(), "a stream");
        assert_eq!(source.size().expect("a size"), 4);
        assert_eq!(all(&source), [1, 2, 3, 4]);
        // a second reader, and a clone on another thread, start over
        assert_eq!(all(&source), [1, 2, 3, 4]);
        let clone = source.clone();
        let read = std::thread::spawn(move || all(&clone))
            .join()
            .expect("joined");
        assert_eq!(read, [1, 2, 3, 4]);
    }

    #[test]
    fn a_file_is_named_as_it_was_given_and_opened_when_it_is_read() {
        let dir = std::env::temp_dir().join(format!("oadec-source-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a directory");
        let path = dir.join("stream.bin");
        let source = Source::file(&path);
        assert_eq!(source.name(), path.display().to_string());
        // not there yet: making the source did not look
        assert!(source.open().is_err());
        assert!(source.size().is_err());
        std::fs::write(&path, [9u8, 8, 7]).expect("written");
        assert_eq!(source.size().expect("a size"), 3);
        assert_eq!(all(&source), [9, 8, 7]);
        assert_eq!(all(&source), [9, 8, 7]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
