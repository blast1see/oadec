//! What the tests over the fixtures share: the streams, cut into access
//! units, laid end to end, damaged, and decoded one presentation at a time
//! by the `Decoder` that the one-pass decoder is held to.

#![allow(dead_code, reason = "each test file uses a part of this")]

use std::path::PathBuf;

use oadec_truehd::{
    AccessUnit, Decoder, Error, Extractor, MajorSync, OnConfigChange, StreamConfig,
};

/// Every TrueHD and MLP fixture of the command line's tests.
pub const FIXTURES: &[&str] = &[
    "authored-scene.mlp",
    "mlp-41-48k.mlp",
    "mlp-51-96k.mlp",
    "mlp-quad-48k.mlp",
    "mlp-stereo-44k.mlp",
    "truehd-20-176k.thd",
    "truehd-20-192k.thd",
    "truehd-20-44k.thd",
    "truehd-51-88k.thd",
];

/// The object stream: four substreams and the four presentations.
pub const OBJECTS: &str = "authored-scene.mlp";

/// Units of the object stream a spliced stream takes, to keep the tests
/// short: a little over a second.
pub const OBJECT_UNITS: usize = 1500;

/// Streams laid end to end, each change of configuration a decoder can
/// follow, and the name of each. The last takes the object presentation
/// away and brings it back.
pub const SPLICES: &[(&str, &[&str])] = &[
    (
        "rate and channels",
        &[
            "truehd-20-44k.thd",
            "truehd-51-88k.thd",
            "truehd-20-192k.thd",
        ],
    ),
    (
        "rate within a family",
        &["truehd-20-176k.thd", "truehd-20-192k.thd"],
    ),
    (
        "dvd-audio layouts",
        &["mlp-stereo-44k.mlp", "mlp-51-96k.mlp", "mlp-41-48k.mlp"],
    ),
    (
        "objects away and back",
        &[OBJECTS, "truehd-51-88k.thd", OBJECTS],
    ),
];

/// The bytes of a fixture.
pub fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../oadec-cli/tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The access units of a stream, as the extractor frames them.
pub fn units_of(stream: &[u8]) -> Vec<Vec<u8>> {
    let mut extractor = Extractor::new();
    extractor.push(stream);
    let mut units = Vec::new();
    while let Some(unit) = extractor.next_unit().expect("the stream frames") {
        units.push(unit.bytes.to_vec());
    }
    let (rest, _) = extractor.finish().expect("the stream ends");
    units.extend(rest.into_iter().map(|unit| unit.bytes.to_vec()));
    assert_eq!(extractor.stats.skipped_bytes, 0, "bytes outside any unit");
    units
}

/// The access units of a fixture; the object stream cut to
/// [`OBJECT_UNITS`] when `cut`.
pub fn fixture_units(name: &str, cut: bool) -> Vec<Vec<u8>> {
    let mut units = units_of(&fixture(name));
    if cut && name == OBJECTS {
        units.truncate(OBJECT_UNITS);
    }
    units
}

/// A stream laid end to end from `parts`: its units, and the index of the
/// first unit of each part.
pub fn splice(parts: &[&str]) -> (Vec<Vec<u8>>, Vec<usize>) {
    let mut units = Vec::new();
    let mut starts = Vec::new();
    for part in parts {
        starts.push(units.len());
        units.extend(fixture_units(part, true));
    }
    (units, starts)
}

/// The major sync of the first unit.
pub fn first_sync(units: &[Vec<u8>]) -> MajorSync {
    let (au, _) = AccessUnit::parse(&units[0], None).expect("the first unit parses");
    au.major_sync.expect("the first unit carries a major sync")
}

/// The configuration of the first unit.
pub fn first_config(units: &[Vec<u8>]) -> StreamConfig {
    AccessUnit::parse(&units[0], None)
        .expect("the first unit parses")
        .1
}

/// The configuration in force at each unit: the one of the last major sync
/// up to it.
pub fn configs(units: &[Vec<u8>]) -> Vec<StreamConfig> {
    let mut config: Option<StreamConfig> = None;
    units
        .iter()
        .map(|unit| {
            let (_, next) = AccessUnit::parse(unit, config.as_ref()).expect("the unit parses");
            config = Some(next.clone());
            next
        })
        .collect()
}

/// The presentations a decoder can be set up for at the start of the stream.
pub fn presentations(units: &[Vec<u8>]) -> Vec<usize> {
    let config = first_config(units);
    (0..4).filter(|&p| config.has_presentation(p)).collect()
}

/// What one access unit came to for one presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// Samples: the meaningful channels of each row, and what the decoder
    /// said of them.
    Samples {
        rows: Vec<Vec<i32>>,
        channels: usize,
        duplicate: bool,
        concealed: bool,
        output_delta: Option<u16>,
        /// The fault that was concealed, as it reads.
        fault: Option<String>,
    },
    /// [`Error::PresentationGone`].
    Gone,
    /// Any other error, as it reads; the decode ends there.
    Failed(String),
}

impl Out {
    pub fn samples(
        pcm: &[[i32; 16]],
        channels: usize,
        duplicate: bool,
        concealed: bool,
        output_delta: Option<u16>,
        fault: Option<String>,
    ) -> Self {
        Self::Samples {
            rows: pcm.iter().map(|row| row[..channels].to_vec()).collect(),
            channels,
            duplicate,
            concealed,
            output_delta,
            fault,
        }
    }
}

/// A decode of one presentation by a `Decoder` of its own, concealing,
/// following changes of configuration by `policy`: what each unit came to,
/// and the counters at the end as they print. The decode ends at the first
/// error that is neither concealed nor [`Error::PresentationGone`].
pub fn single(
    units: &[Vec<u8>],
    presentation: usize,
    policy: OnConfigChange,
) -> (Vec<Out>, String) {
    let mut decoder = Decoder::new(&first_sync(units), presentation).expect("the presentation");
    decoder.on_config_change(policy);
    let mut outs = Vec::with_capacity(units.len());
    for unit in units {
        match decoder.decode_concealing(unit) {
            Ok((d, fault)) => outs.push(Out::samples(
                d.pcm,
                d.channels,
                d.duplicate,
                d.concealed,
                d.output_delta,
                fault.map(|e| e.to_string()),
            )),
            Err(Error::PresentationGone { presentation: p }) => {
                assert_eq!(p, presentation, "the error names the presentation");
                outs.push(Out::Gone);
            }
            Err(e) => {
                outs.push(Out::Failed(e.to_string()));
                break;
            }
        }
    }
    (outs, format!("{:?}", decoder.stats()))
}

/// `units` with one byte of the segment of `substream` in unit `unit`
/// turned over, in the middle of the segment.
pub fn damage_substream(units: &mut [Vec<u8>], unit: usize, substream: usize) {
    let config = configs(units)[unit].clone();
    let (au, _) = AccessUnit::parse(&units[unit], Some(&config)).expect("the unit parses");
    let range = au.segment_range(substream);
    assert!(
        !range.is_empty(),
        "unit {unit} substream {substream} is empty"
    );
    let middle = range.start + range.len() / 2;
    units[unit][middle] ^= 0x5A;
}

/// `units` with the check nibble of the header of unit `unit` turned over,
/// so the unit's framing does not read.
pub fn damage_framing(units: &mut [Vec<u8>], unit: usize) {
    units[unit][0] ^= 0xF0;
}

/// FNV-1a, 64 bits.
#[derive(Debug, Clone, Copy)]
pub struct Fnv(pub u64);

impl Fnv {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn out(&mut self, out: &Out) {
        match out {
            Out::Samples {
                rows,
                channels,
                duplicate,
                concealed,
                output_delta,
                fault,
            } => {
                self.bytes(&[
                    0,
                    *channels as u8,
                    u8::from(*duplicate),
                    u8::from(*concealed),
                ]);
                self.bytes(format!("{output_delta:?}{fault:?}").as_bytes());
                for row in rows {
                    for v in row {
                        self.bytes(&v.to_le_bytes());
                    }
                }
            }
            Out::Gone => self.bytes(&[1]),
            Out::Failed(e) => {
                self.bytes(&[2]);
                self.bytes(e.as_bytes());
            }
        }
    }
}
