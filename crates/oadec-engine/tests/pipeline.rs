//! A stream read once is the stream read from its file.
//!
//! [`Pass`] reads a stream once and lends every unit to its stages; the calls
//! of `api` that end in `_from` stand on it. For every stream the tests keep,
//! a pass frames what the walk over a file of it frames, whatever the size of
//! the reads that brought it, and a call that reads it once reports what the
//! call of a [`Source`] of the same bytes reports. A [`Spool`] gives back what
//! it was given, in order, wherever its limit fell.
//!
//! Round 5 builds these; until the second stop every test here is red and
//! ignored, and `cargo test -- --ignored` shows it.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use oadec_engine::api::{self, Source, VerifyOptions};
use oadec_engine::pipeline::{Ac3Gap, Codec, Framed, Head, Once, Pass, PassEnd, Stage, UnitRef};
use oadec_engine::spool::Spool;
use oadec_engine::{eac3, input};
use serde_json::Value;

const NAME: &str = "<stdin>";

/// The streams the command line's tests keep, with what the campaign of
/// damaged streams turned up: gaps, resyncs and units cut short among them.
fn streams() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for (folder, extensions) in [
        ("fixtures", &["thd", "mlp", "ec3"][..]),
        ("found", &["bin"][..]),
    ] {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oadec-cli/tests")
            .join(folder);
        found.extend(
            std::fs::read_dir(&dir)
                .expect("the streams of the command line's tests")
                .map(|entry| entry.expect("an entry").path())
                .filter(|path| {
                    path.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| extensions.contains(&e))
                }),
        );
    }
    found.sort();
    assert!(found.len() >= 17, "{found:?}");
    found
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The stream over and over, past a megabyte. Most of the streams kept are
/// shorter than the head a pass holds back ([`HEAD`]), so as they are they
/// never reach the framer in more than one piece.
fn long(bytes: &[u8]) -> Vec<u8> {
    bytes.repeat((1 << 20) / bytes.len().max(1) + 1)
}

/// A reader that gives its bytes in pieces of the sizes a seed draws, from
/// one byte to `most`, as a pipe does.
struct Cut<'a> {
    bytes: &'a [u8],
    at: usize,
    most: usize,
    state: u64,
}

impl<'a> Cut<'a> {
    fn new(bytes: &'a [u8], most: usize, seed: u64) -> Self {
        Self {
            bytes,
            at: 0,
            most,
            state: seed | 1,
        }
    }
}

impl Read for Cut<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // xorshift64: the same pieces on every run
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        let want = 1 + (self.state % self.most as u64) as usize;
        let n = want.min(buf.len()).min(self.bytes.len() - self.at);
        buf[..n].copy_from_slice(&self.bytes[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Everything a stage is told, as a line each.
#[derive(Default)]
struct Told(Vec<String>);

impl Stage for Told {
    fn unit(&mut self, unit: &UnitRef<'_>) -> anyhow::Result<()> {
        let head = match unit.head {
            Head::TrueHd(u) => format!(
                "access unit, major sync {}, resynced {}, gap {:?}",
                u.has_major_sync, u.resynced, u.gap_before
            ),
            Head::Ac3(h) => format!("syncframe {h:?}"),
        };
        // the bytes by a sum of their own, which is enough to tell two apart
        let sum = unit.bytes.iter().fold(0u64, |s, &b| {
            s.wrapping_mul(1_000_003).wrapping_add(u64::from(b))
        });
        self.0.push(format!(
            "{} at {}: {} bytes ({sum:016x}), {head}",
            unit.index,
            unit.offset,
            unit.bytes.len()
        ));
        Ok(())
    }

    fn gap(&mut self, gap: &Ac3Gap) {
        self.0.push(format!("{gap:?}"));
    }

    fn finish(&mut self, end: &PassEnd) -> anyhow::Result<()> {
        self.0.push(format!("{end:?}"));
        Ok(())
    }
}

/// What a stage is told of `bytes` read from `reader`.
fn told(reader: impl Read) -> (Codec, Vec<String>, PassEnd) {
    let once = Once::new(reader).expect("the head");
    let codec = once.codec();
    let mut stage = Told::default();
    let end = Pass::run(once, NAME, None, &mut [&mut stage]).expect("the pass");
    (codec, stage.0, end)
}

#[test]
#[ignore = "r5 durak 2: Pass::run and Once::new are bodies of the second stop"]
fn a_stream_is_told_alike_however_it_was_cut() {
    for path in streams() {
        let once = read(&path);
        for bytes in [once.clone(), long(&once)] {
            let whole = told(&bytes[..]);
            for (most, seed) in [(1, 1), (7, 2), (4096, 3), (65_537, 4), (1 << 22, 5)] {
                // a byte at a time is slow: the short streams only
                if most < 8 && bytes.len() > 1 << 18 {
                    continue;
                }
                let cut = told(Cut::new(&bytes, most, seed));
                let at = format!("{} of {} bytes", path.display(), bytes.len());
                assert_eq!(cut.0, whole.0, "{at}");
                assert_eq!(cut.1, whole.1, "{at}, pieces of 1..={most}");
            }
        }
    }
}

#[test]
#[ignore = "r5 durak 2: Pass::run and Once::new are bodies of the second stop"]
fn a_pass_frames_what_the_walk_over_a_file_frames() {
    for path in streams() {
        let bytes = long(&read(&path));
        let source = Source::memory(path.display().to_string(), bytes.clone());
        let (codec, _, end) = told(Cut::new(&bytes, 9000, 7));
        let ac3 = api::is_ac3_family(&source).expect("the head");
        assert_eq!(codec == Codec::Ac3, ac3, "{}", path.display());
        assert_eq!(end.bytes, bytes.len() as u64, "{}", path.display());
        match end.framed {
            Framed::TrueHd(summary) => {
                let mut units = 0u64;
                let walked = input::for_each_unit(&source, |_| {
                    units += 1;
                    Ok(())
                })
                .expect("the walk");
                assert_eq!(end.units, units, "{}", path.display());
                assert_eq!(summary.stats, walked.stats, "{}", path.display());
                assert_eq!(summary.trailing_bytes, walked.trailing_bytes);
                assert_eq!(summary.final_gap, walked.final_gap);
                assert_eq!(summary.file_bytes, walked.file_bytes);
            }
            Framed::Ac3 {
                frames,
                sync_errors,
                skipped,
            } => {
                let walked = eac3::for_each_frame(&source, |_, _, _| Ok(())).expect("the walk");
                assert_eq!((frames, sync_errors, skipped), walked, "{}", path.display());
                assert_eq!(end.units, frames, "{}", path.display());
            }
        }
    }
}

#[test]
#[ignore = "r5 durak 2: Once::new is a body of the second stop"]
fn the_head_is_read_again_and_tells_the_codec_a_file_tells() {
    for path in streams() {
        let bytes = read(&path);
        // a pipe that gives a few bytes at a time still gives the whole head
        let mut once = Once::new(Cut::new(&bytes, 300, 11)).expect("the head");
        assert_eq!(
            once.head().len(),
            bytes.len().min(oadec_engine::pipeline::HEAD),
            "{}",
            path.display()
        );
        let ac3 = api::is_ac3_family(&Source::file(&path)).expect("the head");
        assert_eq!(once.codec() == Codec::Ac3, ac3, "{}", path.display());
        let mut again = Vec::new();
        once.read_to_end(&mut again).expect("read");
        assert!(again == bytes, "{}", path.display());
    }
}

/// A stage that ends the pass at a unit, with an error of its own type.
struct StopAt {
    at: u64,
    seen: Vec<u64>,
    finished: bool,
}

#[derive(Debug)]
struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stopped")
    }
}

impl std::error::Error for Stopped {}

impl Stage for StopAt {
    fn unit(&mut self, unit: &UnitRef<'_>) -> anyhow::Result<()> {
        self.seen.push(unit.index);
        if unit.index == self.at {
            return Err(Stopped.into());
        }
        Ok(())
    }

    fn finish(&mut self, _: &PassEnd) -> anyhow::Result<()> {
        self.finished = true;
        Ok(())
    }
}

#[test]
#[ignore = "r5 durak 2: Pass::run is a body of the second stop"]
fn an_error_of_a_stage_ends_the_pass_at_its_unit_and_nobody_finishes() {
    for name in ["truehd-20-44k.thd", "authored-scene.ec3"] {
        let bytes = read(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../oadec-cli/tests/fixtures")
                .join(name),
        );
        let mut first = StopAt {
            at: u64::MAX,
            seen: Vec::new(),
            finished: false,
        };
        let mut second = StopAt {
            at: 3,
            seen: Vec::new(),
            finished: false,
        };
        let mut third = StopAt {
            at: u64::MAX,
            seen: Vec::new(),
            finished: false,
        };
        let once = Once::new(&bytes[..]).expect("the head");
        let error = Pass::run(once, NAME, None, &mut [&mut first, &mut second, &mut third])
            .expect_err("the second stage stops it");
        // the error is the stage's own, as it is
        assert!(
            error.downcast_ref::<Stopped>().is_some(),
            "{name}: {error:#}"
        );
        assert_eq!(first.seen, [0, 1, 2, 3], "{name}");
        assert_eq!(second.seen, [0, 1, 2, 3], "{name}");
        // the stage after the one that failed is not lent its unit
        assert_eq!(third.seen, [0, 1, 2], "{name}");
        assert!(!first.finished && !second.finished && !third.finished);
    }
}

/// A reader that gives `good` bytes and then an error.
struct Breaks<'a> {
    good: &'a [u8],
    at: usize,
}

impl Read for Breaks<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.at == self.good.len() {
            return Err(io::Error::other("the cable came out"));
        }
        let n = buf.len().min(self.good.len() - self.at).min(1000);
        buf[..n].copy_from_slice(&self.good[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// A reader interrupted before every read it gives.
struct Interrupted<'a> {
    bytes: &'a [u8],
    at: usize,
    now: bool,
}

impl Read for Interrupted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.now = !self.now;
        if self.now {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let n = buf.len().min(self.bytes.len() - self.at).min(5000);
        buf[..n].copy_from_slice(&self.bytes[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[test]
#[ignore = "r5 durak 2: Pass::run and Once::new are bodies of the second stop"]
fn a_read_that_fails_ends_the_pass_by_name_and_one_interrupted_is_read_again() {
    // longer than the head, which is read before the pass begins
    let bytes = read(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oadec-cli/tests/fixtures/authored-scene.mlp"),
    );
    let mut stage = Told::default();
    let once = Once::new(Breaks {
        good: &bytes[..bytes.len() / 2],
        at: 0,
    })
    .expect("the head");
    let error = Pass::run(once, NAME, None, &mut [&mut stage]).expect_err("a read failed");
    let said = format!("{error:#}");
    assert!(
        said.contains(NAME) && said.contains("the cable came out"),
        "{said}"
    );
    // told units, and no end
    assert!(!stage.0.is_empty());
    assert!(stage.0.iter().all(|line| !line.starts_with("PassEnd")));

    let whole = told(&bytes[..]);
    let interrupted = told(Interrupted {
        bytes: &bytes,
        at: 0,
        now: false,
    });
    assert_eq!(interrupted.1, whole.1);
}

#[test]
#[ignore = "r5 durak 2: Pass::run and Once::new are bodies of the second stop"]
fn a_stream_cut_inside_a_unit_ends_as_the_file_cut_there_ends() {
    for name in ["truehd-51-88k.thd", "mlp-51-96k.mlp", "authored-scene.ec3"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../oadec-cli/tests/fixtures")
            .join(name);
        let bytes = long(&read(&path));
        // into the middle of a unit, wherever the middle of the stream is
        let cut = &bytes[..bytes.len() / 2 + 17];
        let (_, _, end) = told(Cut::new(cut, 50_000, 13));
        let source = Source::memory(NAME, cut.to_vec());
        match end.framed {
            Framed::TrueHd(summary) => {
                let walked = input::for_each_unit(&source, |_| Ok(())).expect("the walk");
                assert_eq!(summary.trailing_bytes, walked.trailing_bytes, "{name}");
                assert_eq!(summary.final_gap, walked.final_gap, "{name}");
                assert_eq!(summary.stats, walked.stats, "{name}");
            }
            Framed::Ac3 {
                frames,
                sync_errors,
                skipped,
            } => {
                let walked = eac3::for_each_frame(&source, |_, _, _| Ok(())).expect("the walk");
                assert_eq!((frames, sync_errors, skipped), walked, "{name}");
            }
        }
    }
}

/// A report without what the clock put in it.
fn timeless(mut report: Value) -> Value {
    if let Some(map) = report.as_object_mut() {
        map.remove("seconds");
        map.remove("realtime_factor");
    }
    report
}

#[test]
#[ignore = "r5 durak 2: the calls that read a stream once are bodies of the second stop"]
fn a_call_that_reads_a_stream_once_reports_what_the_call_of_its_source_reports() {
    for path in streams() {
        let bytes = read(&path);
        let at = path.display().to_string();
        let source = Source::memory(NAME, bytes.clone());
        let piped = || Cut::new(&bytes, 70_000, 17);

        let (of_source, of_reader) = (
            api::info(&source),
            api::info_from(NAME, piped(), None, None),
        );
        match (of_source, of_reader) {
            (Ok(a), Ok(b)) => assert_eq!(
                b.json().expect("json").map(timeless),
                a.json().expect("json").map(timeless),
                "info {at}"
            ),
            (Err(a), Err(b)) => assert_eq!(format!("{b:#}"), format!("{a:#}"), "info {at}"),
            (a, b) => panic!("info {at}: {:?} and {:?}", a.err(), b.err()),
        }

        for decode in [false, true] {
            let options = VerifyOptions {
                decode,
                ..VerifyOptions::default()
            };
            match (
                api::verify(&source, &options),
                api::verify_from(NAME, piped(), &options, None),
            ) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(b.clean(), a.clean(), "verify {decode} {at}");
                    assert_eq!(
                        b.json().expect("json").map(timeless),
                        a.json().expect("json").map(timeless),
                        "verify {decode} {at}"
                    );
                }
                (Err(a), Err(b)) => {
                    assert_eq!(format!("{b:#}"), format!("{a:#}"), "verify {decode} {at}");
                }
                (a, b) => panic!("verify {decode} {at}: {:?} and {:?}", a.err(), b.err()),
            }
        }

        match (api::oamd(&source), api::oamd_from(NAME, piped(), None)) {
            (Ok(a), Ok(b)) => {
                assert_eq!(b.clean(), a.clean(), "oamd {at}");
                assert_eq!(
                    timeless(b.json().expect("json")),
                    timeless(a.json().expect("json")),
                    "oamd {at}"
                );
            }
            (Err(a), Err(b)) => assert_eq!(format!("{b:#}"), format!("{a:#}"), "oamd {at}"),
            (a, b) => panic!("oamd {at}: {:?} and {:?}", a.err(), b.err()),
        }

        match (api::emdf(&source), api::emdf_from(NAME, piped(), None)) {
            (Ok(a), Ok(b)) => {
                assert_eq!(b.clean(), a.clean(), "emdf {at}");
                assert_eq!(
                    timeless(b.json().expect("json")),
                    timeless(a.json().expect("json")),
                    "emdf {at}"
                );
            }
            (Err(a), Err(b)) => assert_eq!(format!("{b:#}"), format!("{a:#}"), "emdf {at}"),
            (a, b) => panic!("emdf {at}: {:?} and {:?}", a.err(), b.err()),
        }
    }
}

// ---------------------------------------------------------------------------
// The spool.
// ---------------------------------------------------------------------------

/// Records of lengths from none to a few hundred bytes, each of its own
/// bytes.
fn records(count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|i| {
            let len = (i * 37) % 300;
            (0..len).map(|j| (i * 7 + j) as u8).collect()
        })
        .collect()
}

/// The files of this process's spools in `dir`.
fn spool_files(dir: &Path) -> Vec<PathBuf> {
    let prefix = format!("oadec-spool-{}-", std::process::id());
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(&prefix))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-pipeline-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "r5 durak 2: Spool::push and Spool::finish are bodies of the second stop"]
fn a_spool_gives_back_what_it_was_given_in_order_wherever_its_limit_fell() {
    let dir = temp("spool");
    let given = records(500);
    let total: usize = given.iter().map(|r| r.len() + 4).sum();
    for limit in [0, 1, 4, 100, 4096, total - 1, total, 1 << 30] {
        let mut spool = Spool::in_dir(limit, dir.clone());
        for record in &given {
            spool.push(record).expect("pushed");
            // what is in memory never passes the limit
            assert!(spool.in_memory() <= limit, "limit {limit}");
        }
        assert_eq!(spool.records(), given.len() as u64);
        assert_eq!(spool.spilled(), total > limit, "limit {limit}");
        assert_eq!(spool_files(&dir).len(), usize::from(spool.spilled()));
        let mut back = spool.finish().expect("finished");
        let mut record = Vec::new();
        let mut read = Vec::new();
        while back.next_record(&mut record).expect("read") {
            read.push(record.clone());
        }
        assert!(read == given, "limit {limit}");
        // the end stays the end
        assert!(!back.next_record(&mut record).expect("read"));
        drop(back);
        assert!(
            spool_files(&dir).is_empty(),
            "limit {limit}: the file stayed"
        );
    }
    // a spool dropped before it is finished takes its file with it
    let mut spool = Spool::in_dir(10, dir.clone());
    for record in &given {
        spool.push(record).expect("pushed");
    }
    assert!(spool.spilled());
    drop(spool);
    assert!(spool_files(&dir).is_empty());
    // and one given nothing gives nothing
    let mut none = Spool::in_dir(0, dir.clone()).finish().expect("finished");
    assert!(!none.next_record(&mut Vec::new()).expect("read"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "r5 durak 2: Spool::push is a body of the second stop"]
fn a_spool_whose_file_cannot_be_made_fails_and_takes_nothing_more() {
    let nowhere = temp("nowhere").join("not-there");
    let mut spool = Spool::in_dir(8, nowhere.clone());
    // what fits in memory needs no file
    spool.push(b"four").expect("in memory");
    let said = spool
        .push(b"past the limit")
        .expect_err("no file")
        .to_string();
    assert!(said.contains("not-there"), "{said}");
    assert!(spool.push(b"").is_err(), "a failed spool takes more");
    assert!(spool.finish().is_err());
    assert!(!nowhere.exists());
}
