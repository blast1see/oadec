//! Seeds: the inputs a target starts from, cut out of whole streams.
//!
//! A target reads a run of access units, a frame, a container or a payload,
//! and the fixtures are whole streams. This cuts a stream into the pieces
//! each target reads, a few of each per stream. A mutation of a real unit
//! gets past the first check and into the syntax behind it; a mutation of
//! noise is refused at the sync word.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use oadec_eac3::{FrameHeader, Options};
use oadec_emdf::container::{self, Container};
use oadec_truehd::{AccessUnit, Decoder, Extractor, StreamConfig, Unit};

/// Pieces of one kind taken from one stream.
const PIECES: usize = 6;
/// Access units in the run a TrueHD target starts from.
const RUN_UNITS: usize = 8;
/// The most bytes such a run, or a run of frames, takes.
const RUN_BYTES: usize = 8 * 1024;
/// The reads the extractor's seed is handed over in, less one.
const READ: u8 = 16;

/// The inputs of every target, by its name.
#[derive(Debug, Default)]
pub struct Seeds {
    sets: BTreeMap<&'static str, Vec<Vec<u8>>>,
}

/// At most `n` of `all`, spread from the first to the last: the metadata at
/// the head of a programme is not the metadata in the middle of it.
fn spread<T>(mut all: Vec<T>, n: usize) -> Vec<T> {
    if all.len() <= n || n < 2 {
        all.truncate(n);
        return all;
    }
    let last = all.len() - 1;
    let mut keep: Vec<usize> = (0..n).map(|i| i * last / (n - 1)).collect();
    keep.dedup();
    let mut index = 0;
    all.retain(|_| {
        index += 1;
        keep.contains(&(index - 1))
    });
    all
}

impl Seeds {
    /// No seeds yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The seeds of `target`; none for a name that is not a target's.
    #[must_use]
    pub fn of(&self, target: &str) -> &[Vec<u8>] {
        self.sets.get(target).map_or(&[], Vec::as_slice)
    }

    /// Seeds in all.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.values().map(Vec::len).sum()
    }

    /// No stream has given a seed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn push(&mut self, target: &'static str, seed: Vec<u8>) {
        let set = self.sets.entry(target).or_default();
        if !seed.is_empty() && !set.contains(&seed) {
            set.push(seed);
        }
    }

    /// The payloads of a container the pipeline reads, each a seed of its
    /// parser's.
    fn push_payloads(&mut self, container: &Container) {
        for payload in &container.payloads {
            match payload.id {
                container::PAYLOAD_ID_OAMD => self.push("oamd_parse", payload.data.clone()),
                container::PAYLOAD_ID_JOC => self.push("joc_params", payload.data.clone()),
                _ => {}
            }
        }
    }

    /// Cuts a TrueHD or MLP stream: a run of access units from its first
    /// major sync for the unit parser, for the extractor and for the decoder
    /// at each presentation it carries, and the Evolution frames of a few of
    /// its units with the payloads in them.
    pub fn add_truehd(&mut self, stream: &[u8]) {
        let units = units_of(stream);
        let Some(start) = units.iter().position(|u| u.has_major_sync) else {
            return;
        };
        let mut run = Vec::new();
        for unit in units[start..].iter().take(RUN_UNITS) {
            if !run.is_empty() && run.len() + unit.bytes.len() > RUN_BYTES {
                break;
            }
            run.extend_from_slice(&unit.bytes);
        }
        self.push("truehd_unit", run.clone());
        self.push("truehd_extract", [&[READ][..], &run].concat());
        if let Ok((first, _)) = AccessUnit::parse(&units[start].bytes, None)
            && let Some(sync) = &first.major_sync
        {
            for presentation in 0..4u8 {
                if Decoder::new(sync, usize::from(presentation)).is_ok() {
                    self.push("truehd_decode", [&[presentation][..], &run].concat());
                }
            }
        }

        let mut config: Option<StreamConfig> = None;
        let mut frames = Vec::new();
        for unit in &units {
            let Ok((parsed, next)) = AccessUnit::parse(&unit.bytes, config.as_ref()) else {
                continue;
            };
            config = Some(next);
            if let Some(frame) = parsed.extra.as_ref().and_then(|e| e.evolution_frame()) {
                frames.push(frame.to_vec());
            }
        }
        for frame in spread(frames, PIECES) {
            if let Ok(container) = container::parse_evolution(&frame) {
                self.push_payloads(&container);
            }
            self.push("emdf_container", frame);
        }
    }

    /// Cuts an AC-3 or E-AC-3 stream: its first frames one by one and as a
    /// run, and the EMDF containers in the skip fields of a few of its frames
    /// with the payloads in them.
    pub fn add_ac3(&mut self, stream: &[u8]) {
        let frames = frames_of(stream);
        let mut run = Vec::new();
        for frame in frames.iter().take(PIECES) {
            self.push("eac3_frame", frame.to_vec());
        }
        for frame in &frames {
            if !run.is_empty() && run.len() + frame.len() > RUN_BYTES {
                break;
            }
            run.extend_from_slice(frame);
        }
        self.push("eac3_program", run);

        let mut containers = Vec::new();
        for frame in &frames {
            let mut decoder = oadec_eac3::Decoder::new(Options {
                dither: false,
                tpnp: true,
                ecpl_full: false,
            });
            let mut skip = Vec::new();
            if let Ok(Some(decoded)) = decoder.decode(frame) {
                skip.extend(decoded.skip_fields.concat());
            }
            while let Ok(Some(decoded)) = decoder.flush() {
                skip.extend(decoded.skip_fields.concat());
            }
            // the payload bytes may hold the sync word again; what parses is
            // a container
            for at in 0..skip.len().saturating_sub(3) {
                if skip[at..at + 2] == container::EMDF_SYNCWORD.to_be_bytes()
                    && let Ok((_, declared)) = container::parse_emdf_with_sync(&skip[at..])
                {
                    // the sync word and the length, then the bytes the length
                    // declares
                    containers.push(skip[at..at + 4 + declared].to_vec());
                    break;
                }
            }
        }
        for bytes in spread(containers, PIECES) {
            if let Ok((container, _)) = container::parse_emdf_with_sync(&bytes) {
                self.push_payloads(&container);
            }
            self.push("emdf_container", bytes);
        }
    }

    /// Cuts every stream of a directory, told by its extension, in the order
    /// of the names.
    pub fn from_dir(dir: &Path) -> io::Result<Self> {
        let mut paths: Vec<_> = std::fs::read_dir(dir)?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<io::Result<_>>()?;
        paths.sort();
        let mut seeds = Self::new();
        for path in paths {
            match path.extension().and_then(|e| e.to_str()) {
                Some("thd" | "mlp") => seeds.add_truehd(&std::fs::read(&path)?),
                Some("ac3" | "ec3" | "eac3") => seeds.add_ac3(&std::fs::read(&path)?),
                _ => {}
            }
        }
        Ok(seeds)
    }

    /// Writes the seeds as a libFuzzer corpus: a directory per target under
    /// `dir`, a file per seed named by a digest of its bytes. Returns the
    /// number of files written.
    pub fn write_corpus(&self, dir: &Path) -> io::Result<usize> {
        let mut written = 0;
        for (target, seeds) in &self.sets {
            let into = dir.join(target);
            std::fs::create_dir_all(&into)?;
            for seed in seeds {
                std::fs::write(into.join(format!("seed-{:016x}", fnv1a(seed))), seed)?;
                written += 1;
            }
        }
        Ok(written)
    }
}

/// FNV-1a over `bytes`, 64 bits: a name for a seed, not a check on it.
#[must_use]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn units_of(stream: &[u8]) -> Vec<Unit> {
    let mut extractor = Extractor::new();
    extractor.push(stream);
    let mut units = Vec::new();
    while let Ok(Some(unit)) = extractor.next_unit() {
        units.push(unit);
    }
    if let Ok((rest, _)) = extractor.finish() {
        units.extend(rest);
    }
    units
}

/// The syncframes of a stream, one after another from its first byte.
fn frames_of(stream: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut at = 0;
    while let Some(rest) = stream.get(at..)
        && let Ok(header) = FrameHeader::parse(rest)
        && let Some(frame) = rest.get(..header.frame_bytes)
        && !frame.is_empty()
    {
        frames.push(frame);
        at += frame.len();
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TARGETS;
    use oadec_emdf::joc::{Joc, SparseReading};
    use oadec_emdf::oamd::Oamd;

    fn fixtures() -> Seeds {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../oadec-cli/tests/fixtures");
        Seeds::from_dir(Path::new(dir)).expect("the fixtures are in the repository")
    }

    #[test]
    fn the_fixtures_give_every_target_seeds() {
        let seeds = fixtures();
        for (name, _) in TARGETS {
            assert!(!seeds.of(name).is_empty(), "{name} has no seed");
        }
        assert!(seeds.of("no such target").is_empty());
        // nothing is cut for a target that does not exist
        let known: usize = TARGETS.iter().map(|(name, _)| seeds.of(name).len()).sum();
        assert_eq!(known, seeds.len());
    }

    /// A seed its own parser refuses starts every run at the door.
    #[test]
    fn each_seed_is_read_by_the_parser_it_is_for() {
        let seeds = fixtures();
        for seed in seeds.of("truehd_unit") {
            let (unit, _) = AccessUnit::parse(seed, None).expect("a run starts at a major sync");
            assert!(unit.major_sync.is_some());
        }
        for seed in seeds.of("truehd_extract") {
            assert_eq!(seed[0], READ);
            assert!(units_of(&seed[1..]).len() >= 2, "a run is more than a unit");
        }
        for seed in seeds.of("truehd_decode") {
            let units = units_of(&seed[1..]);
            let (first, _) = AccessUnit::parse(&units[0].bytes, None).unwrap();
            let mut decoder =
                Decoder::new(first.major_sync.as_ref().unwrap(), usize::from(seed[0])).unwrap();
            let mut samples = 0;
            for unit in &units {
                samples += decoder
                    .decode(&unit.bytes)
                    .expect("a fixture decodes")
                    .pcm
                    .len();
            }
            assert!(samples > 0);
        }
        for seed in seeds.of("emdf_container") {
            assert!(
                container::parse_evolution(seed).is_ok()
                    || container::parse_emdf_with_sync(seed).is_ok()
            );
        }
        for seed in seeds.of("oamd_parse") {
            Oamd::parse(seed).expect("a payload of a fixture parses");
        }
        for seed in seeds.of("joc_params") {
            Joc::parse(seed, SparseReading::Measured).expect("a payload of a fixture parses");
        }
        for seed in seeds.of("eac3_frame") {
            let header = FrameHeader::parse(seed).expect("a frame of a fixture");
            assert_eq!(header.frame_bytes, seed.len());
        }
        for seed in seeds.of("eac3_program") {
            assert!(frames_of(seed).len() >= 2, "a run is more than a frame");
        }
    }

    #[test]
    fn both_carriages_of_a_container_and_both_payloads_are_among_the_seeds() {
        let seeds = fixtures();
        let containers = seeds.of("emdf_container");
        assert!(
            containers
                .iter()
                .any(|c| container::parse_evolution(c).is_ok())
        );
        assert!(
            containers
                .iter()
                .any(|c| container::parse_emdf_with_sync(c).is_ok())
        );
        // object metadata rides in both carriages, JOC in the second only
        assert!(seeds.of("oamd_parse").len() >= 2);
        assert!(!seeds.of("joc_params").is_empty());
    }

    #[test]
    fn a_seed_is_cut_once_however_often_its_stream_is_given() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../oadec-cli/tests/fixtures");
        let stream = std::fs::read(Path::new(dir).join("truehd-20-44k.thd")).unwrap();
        let mut seeds = Seeds::new();
        seeds.add_truehd(&stream);
        let once = seeds.len();
        seeds.add_truehd(&stream);
        assert!(once > 0);
        assert_eq!(seeds.len(), once);
    }

    #[test]
    fn spreading_keeps_the_ends_and_the_order() {
        assert_eq!(spread((0..10).collect(), 4), [0, 3, 6, 9]);
        assert_eq!(spread((0..3).collect(), 6), [0, 1, 2]);
        assert_eq!(spread((0..100).collect(), 2), [0, 99]);
        assert_eq!(spread((0..5).collect(), 1), [0]);
        assert_eq!(spread(Vec::<u8>::new(), 3), []);
    }

    #[test]
    fn a_stream_that_is_no_stream_gives_nothing() {
        let mut seeds = Seeds::new();
        seeds.add_truehd(&[0x55; 4096]);
        seeds.add_ac3(&[0x55; 4096]);
        seeds.add_truehd(&[]);
        seeds.add_ac3(&[]);
        assert!(seeds.is_empty());
    }

    #[test]
    fn the_corpus_is_a_directory_per_target() {
        let seeds = fixtures();
        let dir = std::env::temp_dir().join(format!("oadec-fuzz-corpus-{}", std::process::id()));
        let written = seeds.write_corpus(&dir).unwrap();
        assert_eq!(written, seeds.len());
        for (name, _) in TARGETS {
            let files = std::fs::read_dir(dir.join(name)).unwrap().count();
            assert_eq!(files, seeds.of(name).len(), "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
