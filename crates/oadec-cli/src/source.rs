//! Where a command reads its stream from.

use std::fs::File;
use std::io::{self, Cursor, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What a command line writes for standard input where it would name a file.
pub const STDIN: &str = "-";

/// What reports and messages call a stream that came in on standard input.
pub const STDIN_NAME: &str = "<stdin>";

/// How much of a stream that is read to its end is asked for at a time.
const PIECE: usize = 16 << 20;

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
    /// A stream that was read to its end, in the pieces it was read in.
    Pieces(Arc<[Box<[u8]>]>),
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

    /// The stream a command line names: the file at `path`, or standard
    /// input where the path is `-`.
    ///
    /// # Errors
    /// What [`Source::stdin`] gives; a file is not looked at here.
    pub fn named(path: &Path) -> io::Result<Self> {
        if path.as_os_str() == STDIN {
            Self::stdin()
        } else {
            Ok(Self::file(path))
        }
    }

    /// Standard input, read to its end.
    ///
    /// A command reads its stream more than once and a pipe can be read
    /// once, so the stream is taken into memory whole before the command
    /// starts, and stays there for as long as the command runs.
    ///
    /// # Errors
    /// When standard input is a terminal, which has no stream to give, and
    /// when reading it fails.
    pub fn stdin() -> io::Result<Self> {
        let stdin = io::stdin();
        if stdin.is_terminal() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "`-` reads the stream from standard input, and standard input is a terminal here: \
                 pipe a stream into the command",
            ));
        }
        Self::read_whole(STDIN_NAME, stdin.lock())
            .map_err(|e| io::Error::new(e.kind(), format!("reading standard input: {e}")))
    }

    /// Whatever `from` gives until it ends, under the name a report is to
    /// give it. The bytes are kept in the pieces they are read in, so a
    /// stream of any length is in memory once and is never copied.
    ///
    /// # Errors
    /// What reading gives.
    pub fn read_whole(name: impl Into<String>, from: impl Read) -> io::Result<Self> {
        Ok(Self {
            name: name.into(),
            from: From::Pieces(pieces(from, PIECE)?.into()),
        })
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
            From::Pieces(pieces) => Reader::Pieces(Pieces {
                pieces: Arc::clone(pieces),
                piece: 0,
                at: 0,
            }),
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
            From::Pieces(pieces) => pieces.iter().map(|p| p.len() as u64).sum(),
        })
    }
}

/// Reads `from` to its end in pieces of `len` bytes; the last holds what was
/// left, and a stream that held nothing gives none.
fn pieces(mut from: impl Read, len: usize) -> io::Result<Vec<Box<[u8]>>> {
    let mut out = Vec::new();
    loop {
        let mut piece = Vec::with_capacity(len);
        from.by_ref().take(len as u64).read_to_end(&mut piece)?;
        let whole = piece.len() == len;
        if !piece.is_empty() {
            out.push(piece.into_boxed_slice());
        }
        if !whole {
            return Ok(out);
        }
    }
}

/// A stream held in the pieces it was read in, opened for reading.
#[derive(Debug)]
pub struct Pieces {
    pieces: Arc<[Box<[u8]>]>,
    /// The piece the next byte is in, and where in it.
    piece: usize,
    at: usize,
}

impl Read for Pieces {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            let Some(piece) = self.pieces.get(self.piece) else {
                break;
            };
            let left = &piece[self.at..];
            if left.is_empty() {
                self.piece += 1;
                self.at = 0;
                continue;
            }
            let take = left.len().min(buf.len() - n);
            buf[n..n + take].copy_from_slice(&left[..take]);
            n += take;
            self.at += take;
        }
        Ok(n)
    }
}

/// A [`Source`] opened for reading.
#[derive(Debug)]
pub enum Reader {
    /// A file.
    File(File),
    /// Bytes in memory.
    Memory(Cursor<Arc<[u8]>>),
    /// A stream that was read to its end.
    Pieces(Pieces),
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::File(file) => file.read(buf),
            Self::Memory(bytes) => bytes.read(buf),
            Self::Pieces(pieces) => pieces.read(buf),
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

    /// A reader that gives its bytes a few at a time, as a pipe does.
    struct Dribble<'a> {
        bytes: &'a [u8],
        at: usize,
        most: usize,
    }

    impl Read for Dribble<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.most.min(buf.len()).min(self.bytes.len() - self.at);
            buf[..n].copy_from_slice(&self.bytes[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    #[test]
    fn a_stream_read_to_its_end_is_cut_where_the_pieces_end_and_nowhere_else() {
        let bytes: Vec<u8> = (0..1000u32).map(|i| (i * 7 + i / 13) as u8).collect();
        for len in [1, 7, 64, 999, 1000, 1001, 4096] {
            for most in [1, 3, 64, 5000] {
                let from = Dribble {
                    bytes: &bytes,
                    at: 0,
                    most,
                };
                let held = pieces(from, len).expect("read");
                assert_eq!(held.len(), bytes.len().div_ceil(len), "pieces of {len}");
                assert!(held.iter().rev().skip(1).all(|p| p.len() == len));
                assert!(held.iter().all(|p| !p.is_empty()));
                assert_eq!(held.concat(), bytes, "pieces of {len}, {most} at a time");
            }
        }
        assert!(pieces(io::empty(), 16).expect("read").is_empty());
    }

    #[test]
    fn a_stream_in_pieces_reads_as_the_bytes_it_was() {
        let bytes: Vec<u8> = (0..1000u32).map(|i| (i * 11 + i / 7) as u8).collect();
        for len in [1, 7, 64, 999, 1000, 1001] {
            let source = Source {
                name: "piped".to_string(),
                from: From::Pieces(pieces(&bytes[..], len).expect("read").into()),
            };
            assert_eq!(source.size().expect("a size"), 1000);
            assert_eq!(all(&source), bytes, "pieces of {len}");
            // into buffers that end inside a piece, at its end and past it
            for buffer in [1, 3, 7, 64, 2000] {
                let mut reader = source.open().expect("opened");
                let (mut out, mut buf) = (Vec::new(), vec![0u8; buffer]);
                loop {
                    let n = reader.read(&mut buf).expect("read");
                    if n == 0 {
                        break;
                    }
                    // a short read is the end, and nothing else: readers that
                    // fill a buffer rely on it
                    assert!(n == buffer || out.len() + n == bytes.len());
                    out.extend_from_slice(&buf[..n]);
                }
                assert_eq!(out, bytes, "pieces of {len}, a buffer of {buffer}");
            }
        }
    }

    #[test]
    fn what_a_reader_gives_until_it_ends_is_a_source_like_any_other() {
        let bytes: Vec<u8> = (0..70_000u32).map(|i| (i ^ (i >> 5)) as u8).collect();
        let source = Source::read_whole(
            STDIN_NAME,
            Dribble {
                bytes: &bytes,
                at: 0,
                most: 4096,
            },
        )
        .expect("read");
        assert_eq!(source.name(), "<stdin>");
        assert_eq!(source.size().expect("a size"), 70_000);
        assert_eq!(all(&source), bytes);
        // a second reader, and a clone on another thread, start over
        assert_eq!(all(&source), bytes);
        let clone = source.clone();
        let read = std::thread::spawn(move || all(&clone))
            .join()
            .expect("joined");
        assert_eq!(read, bytes);
        // and nothing at all is a source of no bytes
        let empty = Source::read_whole("nothing", io::empty()).expect("read");
        assert_eq!(empty.size().expect("a size"), 0);
        assert!(all(&empty).is_empty());
    }

    #[test]
    fn a_path_names_a_file_and_a_dash_does_not() {
        let named = Source::named(Path::new("some/stream.thd")).expect("a file");
        assert_eq!(
            named.name(),
            Path::new("some/stream.thd").display().to_string()
        );
        // a file is not looked at until it is read
        assert!(named.open().is_err());
        assert!(matches!(named.from, From::File(_)));
        // the dash by itself, and nothing that merely has one
        for path in ["-x", "./-", "--", "-.thd"] {
            let named = Source::named(Path::new(path)).expect("a file");
            assert!(matches!(named.from, From::File(_)), "{path}");
        }
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
