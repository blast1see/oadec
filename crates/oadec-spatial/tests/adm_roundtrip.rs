//! Writer-level end-to-end check of the object outputs: a programme with a
//! bed and three objects is written as ADM and as DAMF from one set of events,
//! the ADM file is read back with a walker that shares no code with the
//! writer, and the two outputs are compared track for track.
//!
//! The events cover the paths the 2026-09-11 audit found unguarded: a gain on
//! an active object, a gain-only change, a late first event, an out-of-order
//! event, an event beyond the programme end, a size, a channel lock and a zone.

use std::path::{Path, PathBuf};

use oadec_emdf::oamd::{BedChannel, Gain};
use oadec_spatial::program::{BedState, ObjectState};
use oadec_spatial::{
    AdmOptions, AdmWriter, DamfOptions, DamfWriter, ElementState, Event, LossKind, Program,
};

const RATE: u32 = 48_000;
const FRAMES: usize = 96_000;

fn state(x: f32) -> ObjectState {
    ObjectState {
        active: true,
        pos: [x, 1.0, 0.0],
        snap: false,
        elevation: true,
        zones: 0,
        size: [0.0; 3],
        importance: 1.0,
        gain: Gain::Db(0),
        ramp: 1536,
        trim_bypass: false,
        screen_factor: 0.0,
        depth_factor: 0.25,
    }
}

fn object(id: u32, sample_pos: u64, s: ObjectState) -> Event {
    Event {
        id,
        sample_pos,
        state: ElementState::Object(s),
        previous: None,
    }
}

/// The events, in the order a decoder might deliver them.
fn events() -> Vec<Event> {
    let mut quiet = state(-1.0);
    quiet.gain = Gain::Db(-6);
    let mut quieter = state(0.0);
    quieter.gain = Gain::Db(-12);
    let mut boxed = state(0.5);
    boxed.size = [0.5; 3];
    boxed.snap = true;
    boxed.zones = 1;
    let mut ceiling = state(0.0);
    ceiling.pos = [0.0, 0.0, 1.0];
    vec![
        Event {
            id: 3,
            sample_pos: 0,
            state: ElementState::Bed(BedState {
                active: true,
                importance: 1.0,
                gain: Gain::Db(0),
                ramp: 0,
                trim_bypass: false,
            }),
            previous: None,
        },
        object(10, 0, quiet),
        object(10, 48_000, quieter),
        object(10, 24_000, state(-0.5)), // out of order
        object(10, 96_000, state(1.0)),  // at the programme end
        object(11, 24_000, boxed),       // late first event
        object(12, 0, ceiling),
    ]
}

/// Deterministic, per-element audio: element `e`, sample `n`.
fn sample(e: usize, n: usize) -> i32 {
    ((n * 7 + e * 1_000) % 20_000) as i32 - 10_000
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-adm-roundtrip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Chunk {
    id: [u8; 4],
    body: usize,
    size: usize,
}

/// The chunks of a RIFF file, in order.
fn chunks(bytes: &[u8]) -> Vec<Chunk> {
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut out = Vec::new();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        out.push(Chunk {
            id,
            body: at + 8,
            size,
        });
        at += 8 + size + (size & 1);
    }
    out
}

fn chunk<'a>(bytes: &'a [u8], chunks: &[Chunk], id: &[u8; 4]) -> &'a [u8] {
    let c = chunks
        .iter()
        .find(|c| &c.id == id)
        .unwrap_or_else(|| panic!("no {} chunk", String::from_utf8_lossy(id)));
    &bytes[c.body..c.body + c.size]
}

/// `hh:mm:ss.fffff` to samples at 48 kHz.
fn samples(tc: &str) -> u64 {
    let mut parts = tc.split(':');
    let h: f64 = parts.next().unwrap().parse().unwrap();
    let m: f64 = parts.next().unwrap().parse().unwrap();
    let s: f64 = parts.next().unwrap().parse().unwrap();
    ((h * 3600.0 + m * 60.0 + s) * f64::from(RATE)).round() as u64
}

/// The `(rtime, duration)` of every block of object `k`, in samples.
fn blocks(axml: &str, k: usize) -> Vec<(u64, u64)> {
    let start = axml
        .find(&format!("audioChannelFormatName=\"Atmos_Obj_{k}\""))
        .expect("object channel format");
    let cf = &axml[start..];
    let cf = &cf[..cf.find("</audioChannelFormat>").unwrap()];
    cf.match_indices("rtime=\"")
        .map(|(i, _)| {
            let rest = &cf[i + 7..];
            let rtime = &rest[..rest.find('"').unwrap()];
            let d = rest.find("duration=\"").unwrap() + 10;
            let dur = &rest[d..d + rest[d..].find('"').unwrap()];
            (samples(rtime), samples(dur))
        })
        .collect()
}

/// Interleaved 24-bit big-endian samples of a CAF file and its channel count.
fn caf(path: &Path) -> (Vec<i32>, usize) {
    let b = std::fs::read(path).unwrap();
    assert_eq!(&b[..4], b"caff");
    let mut at = 8;
    let mut channels = 0usize;
    while at + 12 <= b.len() {
        let kind = &b[at..at + 4];
        let size = i64::from_be_bytes(b[at + 4..at + 12].try_into().unwrap());
        let body = at + 12;
        let len = if size < 0 {
            b.len() - body
        } else {
            size as usize
        };
        if kind == b"desc" {
            channels = u32::from_be_bytes(b[body + 24..body + 28].try_into().unwrap()) as usize;
            assert_eq!(
                u32::from_be_bytes(b[body + 28..body + 32].try_into().unwrap()),
                24
            );
        } else if kind == b"data" {
            let pcm = &b[body + 4..body + len];
            let out = pcm
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| {
                    (i32::from(c[0]) << 24 | i32::from(c[1]) << 16 | i32::from(c[2]) << 8) >> 8
                })
                .collect();
            return (out, channels);
        }
        at = body + len;
    }
    panic!("no data chunk in {}", path.display());
}

#[test]
fn the_adm_file_reads_back_as_the_programme_it_was_given() {
    let dir = temp_dir();
    let program = Program {
        beds: vec![vec![BedChannel::LFE]],
        isf_index: None,
        isf_objects: 0,
        dynamic_objects: 3,
    };
    let elements = program.elements();
    let rows: Vec<Vec<i32>> = (0..FRAMES)
        .map(|n| (0..elements).map(|e| sample(e, n)).collect())
        .collect();

    let adm_path = dir.join("t.wav");
    let mut adm = AdmWriter::create(&adm_path, &program, RATE, &AdmOptions::default()).unwrap();
    adm.write_frames(rows.iter().map(Vec::as_slice), elements)
        .unwrap();
    for e in events() {
        adm.push_event(&e);
    }
    let adm_summary = adm.finish().unwrap();

    let mut damf = DamfWriter::create(&dir, "t", &program, RATE, &DamfOptions::default()).unwrap();
    damf.write_frames(rows.iter().map(Vec::as_slice), elements)
        .unwrap();
    for e in events() {
        damf.push_event(&e).unwrap();
    }
    let damf_summary = damf.finish().unwrap();

    // ---- the ledgers say what the writers could not carry
    assert_eq!(adm_summary.losses.count(LossKind::EventBeyondEndDropped), 1);
    assert_eq!(adm_summary.losses.count(LossKind::LateFirstEventHeld), 1);
    assert!(adm_summary.losses.count(LossKind::RampReplaced) >= 3);
    assert!(!adm_summary.losses.declared_loss());
    assert_eq!(
        damf_summary.losses.count(LossKind::OutOfOrderWrittenAsIs),
        1
    );
    assert!(damf_summary.losses.declared_loss());

    // ---- container
    let bytes = std::fs::read(&adm_path).unwrap();
    let cs = chunks(&bytes);
    let ids: Vec<String> = cs
        .iter()
        .map(|c| String::from_utf8_lossy(&c.id).into_owned())
        .collect();
    assert_eq!(ids, ["JUNK", "fmt ", "data", "axml", "chna", "dbmd"]);
    let fmt = chunk(&bytes, &cs, b"fmt ");
    let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap()) as usize;
    assert_eq!(u16::from_le_bytes(fmt[0..2].try_into().unwrap()), 1);
    assert_eq!(channels, 13, "ten bed tracks and three objects");
    assert_eq!(u32::from_le_bytes(fmt[4..8].try_into().unwrap()), RATE);
    assert_eq!(u16::from_le_bytes(fmt[12..14].try_into().unwrap()), 39);
    assert_eq!(u16::from_le_bytes(fmt[14..16].try_into().unwrap()), 24);
    let data = chunk(&bytes, &cs, b"data");
    assert_eq!(data.len(), FRAMES * channels * 3);
    assert_eq!(adm_summary.channels, channels);
    assert_eq!(adm_summary.frames as usize, FRAMES);

    // ---- chna: one entry per track, in track order, with the profile's ids
    let chna = chunk(&bytes, &cs, b"chna");
    assert_eq!(
        u16::from_le_bytes(chna[0..2].try_into().unwrap()) as usize,
        channels
    );
    assert_eq!(
        u16::from_le_bytes(chna[2..4].try_into().unwrap()) as usize,
        channels
    );
    for i in 0..channels {
        let e = &chna[4 + 40 * i..4 + 40 * (i + 1)];
        let text = |from: usize, len: usize| {
            String::from_utf8_lossy(&e[from..from + len])
                .trim_end_matches('\0')
                .to_string()
        };
        assert_eq!(
            u16::from_le_bytes(e[0..2].try_into().unwrap()) as usize,
            i + 1
        );
        assert_eq!(text(2, 12), format!("ATU_{:08x}", i + 1));
        if i < 10 {
            assert_eq!(text(14, 14), format!("AT_{:08x}_01", 0x0001_1001 + i));
            assert_eq!(text(28, 11), "AP_00011001");
        } else {
            let k = i - 10 + 1;
            assert_eq!(text(14, 14), format!("AT_{:08x}_01", 0x0003_1000 + k));
            assert_eq!(text(28, 11), format!("AP_{:08x}", 0x0003_1000 + k));
        }
    }

    // ---- axml: ids, gain, blocks that tile, the fields the events carried
    let axml = String::from_utf8(chunk(&bytes, &cs, b"axml").to_vec()).unwrap();
    assert!(axml.contains("audioObjectID=\"AO_1001\""));
    assert!(axml.contains("audioObjectID=\"AO_100b\""));
    assert!(axml.contains("audioObjectID=\"AO_100d\""));
    assert!(axml.contains("<gain>0.5011872053</gain>"));
    assert!(axml.contains("<gain>0.2511886358</gain>"));
    assert!(axml.contains("<channelLock>1</channelLock>"));
    assert!(axml.contains("<width>0.5000000000</width>"));
    assert!(axml.contains(">ZM1</zone>"));
    assert!(axml.contains("<position coordinate=\"Z\">1.0000000000</position>"));
    assert!(axml.contains("interpolationLength=\"0.005208\""));
    for k in 1..=3 {
        let b = blocks(&axml, k);
        assert_eq!(b[0].0, 0, "object {k} starts at 0");
        for w in b.windows(2) {
            assert_eq!(w[0].0 + w[0].1, w[1].0, "object {k} blocks tile");
        }
        let (last_start, last_dur) = *b.last().unwrap();
        assert_eq!(
            last_start + last_dur,
            FRAMES as u64,
            "object {k} ends at the end"
        );
    }
    assert_eq!(
        blocks(&axml, 1).len(),
        3,
        "0, 24000 and 48000; the event at the end is dropped"
    );
    assert_eq!(blocks(&axml, 1)[1].0, 24_000, "sorted into time order");
    assert_eq!(
        blocks(&axml, 2).len(),
        2,
        "held from 0, then the real first event"
    );
    assert_eq!(blocks(&axml, 2)[1].0, 24_000);
    assert_eq!(adm_summary.blocks, 3 + 2 + 1);

    // ---- the same audio in both outputs, track for track
    let (damf_pcm, damf_channels) = caf(&dir.join("t.atmos.audio"));
    assert_eq!(damf_channels, channels);
    assert_eq!(damf_summary.channels, channels);
    let adm_pcm: Vec<i32> = data
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| (i32::from(c[0]) << 8 | i32::from(c[1]) << 16 | i32::from(c[2]) << 24) >> 8)
        .collect();
    assert_eq!(adm_pcm.len(), damf_pcm.len());
    assert!(
        adm_pcm == damf_pcm,
        "the ADM data and the DAMF audio differ"
    );
    // and it is the audio that was written: track 3 is the LFE, track 10 object 1
    assert_eq!(adm_pcm[3], sample(0, 0));
    assert_eq!(adm_pcm[channels * 5 + 10], sample(1, 5));
    assert_eq!(adm_pcm[channels * 7 + 12], sample(3, 7));

    std::fs::remove_dir_all(&dir).unwrap();
}
