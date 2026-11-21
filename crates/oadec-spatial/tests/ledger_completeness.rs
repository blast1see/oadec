//! Round 5, P3: every OAMD field an object output drops has to leave a line
//! in the loss ledger. One test per field that was parsed and dropped without
//! a word before round 5, built from payloads made by hand (no committed
//! stream carries any of them: see `docs/oamd.md`, *What a library carries*).
//! They were written red, before the kinds existed, and name each kind by its
//! report name, which is what a reader of the report meets. Every kind is of a
//! class that leaves the exit code at 0: none is a declared loss.
//!
//! | report name | class | counted |
//! |---|---|---|
//! | `additional-table-data-dropped` | unrepresentable | per update block that carries any |
//! | `alternate-object-data-dropped` | unrepresentable | once per change of the set of alternate elements not read |
//! | `unknown-element-dropped` | unrepresentable | once per change of the set of unknown element ids |
//! | `bed-distribute-dropped` | unrepresentable | once per change, when set |
//! | `reserved-data-dropped` | unrepresentable | once per change, when present |
//! | `reserved-zone-read-as-all` | approximation | per event whose zone index is 6 or 7 |
//!
//! Beside them, the ADM profile guard (task 2), two counts the ADM writer
//! used to miss on the first state of a bed channel, and what the timeline
//! keeps of what it does not write.

use std::path::{Path, PathBuf};

use std::cell::RefCell;
use std::io;

use oadec_emdf::oamd::{
    BasicInfo, Bed, BedChannel, BlockTiming, Distance, Element, ElementMd, ExtendedObjectElement,
    Gain, Oamd, ObjectElement, ObjectInfoBlock, ProgramAssignment, RenderInfo, ScreenRef, Status,
    TrimConfig, TrimElement, UpdateTiming,
};
use oadec_spatial::adm::{self, Events, MAX_ELEMENTS, MAX_OBJECTS, MAX_TRACKS, MemoryEvents};
use oadec_spatial::program::{BedState, Extras, ObjectState};
use oadec_spatial::{
    AdmOptions, AdmWriter, DamfOptions, DamfWriter, ElementState, Event, LossClass, LossLedger,
    Program, Timeline,
};

// ---------------------------------------------------------------------------
// Payloads by hand.

fn block(in_bed: bool) -> ObjectInfoBlock {
    ObjectInfoBlock {
        not_active: false,
        in_bed_or_isf: in_bed,
        basic_status: Status::Full,
        basic: BasicInfo {
            gain: Gain::Db(0),
            priority: 1.0,
        },
        render_status: if in_bed {
            Status::Default
        } else {
            Status::Full
        },
        render: RenderInfo::DEFAULT,
        additional_table_data: Vec::new(),
    }
}

fn element_md(id: u8, element: Element) -> ElementMd {
    ElementMd {
        id,
        size_bytes: 1,
        alternate_id: None,
        discard_unknown: false,
        element,
        size_ok: true,
        padding_bits: 0,
        padding_zero: true,
    }
}

/// An LFE bed channel (element id 3) and one dynamic object (element id 10),
/// one update block each, with a 1536-sample ramp.
fn payload() -> Oamd {
    let objects = ObjectElement {
        timing: UpdateTiming {
            sample_offset: 0,
            blocks: vec![BlockTiming {
                block_offset_factor: 0,
                ramp_duration: 1536,
            }],
        },
        reserved: None,
        objects: vec![vec![block(true)], vec![block(false)]],
    };
    Oamd {
        version: 0,
        object_count: 2,
        program: ProgramAssignment {
            dyn_object_only: false,
            content_description: 0b0101,
            beds: vec![Bed {
                lfe_only: true,
                standard: true,
                channels: vec![BedChannel::LFE],
            }],
            dynamic_objects: 1,
            ..ProgramAssignment::default()
        },
        alternate_object_data_present: false,
        elements: vec![element_md(1, Element::Object(objects))],
        padding_bits: 0,
        padding_zero: true,
    }
}

fn object_element(oamd: &mut Oamd) -> &mut ObjectElement {
    oamd.elements
        .iter_mut()
        .find_map(|e| match &mut e.element {
            Element::Object(o) => Some(o),
            _ => None,
        })
        .expect("an object element")
}

/// Pushes the payloads at 0, 1536, 3072 … and gives the timeline.
fn run(payloads: &[Oamd]) -> Timeline {
    let mut t = Timeline::new(false);
    for (k, p) in payloads.iter().enumerate() {
        t.push(p, k as u64 * 1536, 0, |_| {})
            .unwrap_or_else(|e| panic!("payload {k}: {e}"));
    }
    t
}

/// The count of the kind reported as `name`, with its class and examples.
/// A kind's count, class and examples (element id, sample position).
type Entry = (u64, LossClass, Vec<(u32, u64)>);

fn entry(ledger: &LossLedger, name: &str) -> Option<Entry> {
    ledger
        .iter()
        .find(|(kind, _)| kind.name() == name)
        .map(|(kind, n)| (n, kind.class(), ledger.examples(kind).to_vec()))
}

/// What a red test asks of the ledger: the kind, its class, its count and its
/// examples; no declared loss (the exit code stays 0); and the stderr line.
fn assert_kind(
    ledger: &LossLedger,
    name: &str,
    class: LossClass,
    count: u64,
    examples: &[(u32, u64)],
    line: &str,
) {
    let got = entry(ledger, name);
    assert_eq!(
        got,
        Some((count, class, examples.to_vec())),
        "{name}: the ledger has {:?}",
        ledger
            .iter()
            .map(|(k, n)| (k.name(), n))
            .collect::<Vec<_>>()
    );
    assert!(!ledger.declared_loss(), "{name} must not set exit code 4");
    let lines = ledger.lines("damf");
    assert!(lines.iter().any(|l| l == line), "{name}: {lines:#?}");
}

// ---------------------------------------------------------------------------
// The fields that leave no line today.

/// `additional_table_data` of an object info block: bytes the standard leaves
/// to later versions, which neither output has a place for. Counted per update
/// block that carries any, on beds and objects alike.
#[test]
fn additional_table_data_is_counted_per_update() {
    let mut p = payload();
    object_element(&mut p).objects[1][0].additional_table_data = vec![0xA5];
    let mut bed = payload();
    object_element(&mut bed).objects[0][0].additional_table_data = vec![1, 2];
    let t = run(&[p.clone(), p, bed]);
    assert_kind(
        &t.losses,
        "additional-table-data-dropped",
        LossClass::Unrepresentable,
        3,
        &[(10, 0), (10, 1536), (3, 3072)],
        "damf: not representable in DAMF or the ADM profile: \
         3 updates with additional table data not carried \
         (element 10 at 0, element 10 at 1536, element 3 at 3072)",
    );
}

/// With `b_alternate_object_data_present` a payload may carry more than one
/// object element, told apart by `alternate_object_data_id_idx`; the timeline
/// reads the first and drops the rest. Counted once per change of the set it
/// drops, not once per payload.
#[test]
fn alternate_object_data_is_counted_once_per_change() {
    let mut p = payload();
    p.alternate_object_data_present = true;
    p.elements[0].alternate_id = Some(0);
    let mut alternate = p.elements[0].clone();
    alternate.alternate_id = Some(1);
    p.elements.push(alternate);
    let mut other = p.clone();
    other.elements[1].alternate_id = Some(2);
    let t = run(&[p.clone(), p, other]);
    assert_kind(
        &t.losses,
        "alternate-object-data-dropped",
        LossClass::Unrepresentable,
        2,
        &[(0, 0), (0, 3072)],
        "damf: not representable in DAMF or the ADM profile: \
         2 alternate object data sets not carried",
    );
}

/// Elements whose id has no syntax in the parser (`Element::Unknown`): 3 and 4
/// (`truehdd` names them headphone and object description) and the reserved
/// ids. Counted once per change of the set of ids.
#[test]
fn unknown_elements_are_counted_once_per_change() {
    let mut three = payload();
    three
        .elements
        .push(element_md(3, Element::Unknown(vec![0x80])));
    let mut both = three.clone();
    both.elements
        .push(element_md(4, Element::Unknown(vec![0x40])));
    let t = run(&[three.clone(), three, both]);
    assert_kind(
        &t.losses,
        "unknown-element-dropped",
        LossClass::Unrepresentable,
        2,
        &[(0, 0), (0, 3072)],
        "damf: not representable in DAMF or the ADM profile: \
         2 elements without a known syntax not carried",
    );
}

/// `b_bed_chan_distribute`. DAMF has a `bedDistribution` key, but adding it to
/// a set left the encoders' output unchanged (`02-damf-key-probe.json`), and
/// the ADM profile has no field. Counted once per change, when set.
#[test]
fn bed_channel_distribution_is_counted_once() {
    let mut p = payload();
    p.program.bed_chan_distribute = true;
    let t = run(&[p.clone(), p.clone(), p]);
    assert_kind(
        &t.losses,
        "bed-distribute-dropped",
        LossClass::Unrepresentable,
        1,
        &[(0, 0)],
        "damf: not representable in DAMF or the ADM profile: \
         1 bed channel distribution flag not carried",
    );
}

/// `reserved_data` of the programme assignment (`content_description` bit 3)
/// and the five `reserved` bits of the object element: what a later version
/// of the syntax may say, carried by neither output. Counted once per change,
/// when present.
#[test]
fn reserved_data_is_counted_once_per_change() {
    let mut p = payload();
    p.program.content_description |= 0b1000;
    p.program.reserved_data = vec![0x12];
    let mut bits = payload();
    object_element(&mut bits).reserved = Some(0b10101);
    let t = run(&[p.clone(), p, bits]);
    assert_kind(
        &t.losses,
        "reserved-data-dropped",
        LossClass::Unrepresentable,
        2,
        &[(0, 0), (0, 3072)],
        "damf: not representable in DAMF or the ADM profile: \
         2 reserved data blocks not carried",
    );
}

/// `zone_constraints_idx` 6 and 7 are reserved (TS 103 420 table 20). DAMF
/// writes them as `zones: all` and the ADM writer as no zone, which is what
/// index 0 says. Counted per event that carries one.
#[test]
fn a_reserved_zone_index_is_counted_per_event() {
    let mut six = payload();
    object_element(&mut six).objects[1][0]
        .render
        .zone_constraints = 6;
    let mut seven = payload();
    object_element(&mut seven).objects[1][0]
        .render
        .zone_constraints = 7;
    let t = run(&[six, seven]);
    assert_kind(
        &t.losses,
        "reserved-zone-read-as-all",
        LossClass::Approximation,
        2,
        &[(10, 0), (10, 1536)],
        "damf: approximations: 2 updates with a reserved zone index written as all zones \
         (element 10 at 0, element 10 at 1536)",
    );
}

/// The ledger is complete when a payload that carries every field above at
/// once leaves a line for each, and no kind is a declared loss.
#[test]
fn a_payload_with_every_dropped_field_leaves_a_line_for_each() {
    let mut p = payload();
    p.program.bed_chan_distribute = true;
    p.program.content_description |= 0b1000;
    p.program.reserved_data = vec![0x12];
    {
        let o = object_element(&mut p);
        o.objects[1][0].additional_table_data = vec![1];
        o.objects[1][0].render.zone_constraints = 7;
    }
    p.alternate_object_data_present = true;
    p.elements[0].alternate_id = Some(0);
    let mut alternate = p.elements[0].clone();
    alternate.alternate_id = Some(1);
    p.elements.push(alternate);
    let mut unknown = element_md(4, Element::Unknown(vec![0]));
    unknown.alternate_id = Some(0);
    p.elements.push(unknown);
    let t = run(&[p]);
    for name in [
        "additional-table-data-dropped",
        "alternate-object-data-dropped",
        "unknown-element-dropped",
        "bed-distribute-dropped",
        "reserved-data-dropped",
        "reserved-zone-read-as-all",
    ] {
        assert!(entry(&t.losses, name).is_some(), "{name} is missing");
    }
    assert!(!t.losses.declared_loss());
}

// ---------------------------------------------------------------------------
// The ADM writer and the first state of a bed channel.

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-ledger-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lfe_and_one_object() -> Program {
    Program {
        beds: vec![vec![BedChannel::LFE]],
        isf_index: None,
        isf_objects: 0,
        dynamic_objects: 1,
    }
}

/// A DirectSpeakers block has no importance and no trim bypass. A later change
/// of either is counted (`bed-event-dropped`); the first state is checked for
/// its gain and activity alone (`bed-gain-dropped`), so an LFE whose first
/// importance is 0.5 or whose trim is bypassed loses that without a line.
/// Stop 2 counts them under the kinds the objects use.
#[test]
fn the_first_state_of_a_bed_channel_is_counted_whole() {
    let dir = temp("bed-first");
    let mut w = AdmWriter::create(
        &dir.join("b.wav"),
        &lfe_and_one_object(),
        48_000,
        &AdmOptions::default(),
    )
    .unwrap();
    let rows = vec![[0i32; 2]; 4_800];
    w.write_frames(rows.iter().map(|r| &r[..]), 2).unwrap();
    w.push_event(&Event {
        id: 3,
        sample_pos: 0,
        state: ElementState::Bed(BedState {
            active: true,
            importance: 0.5,
            gain: Gain::Db(0),
            ramp: 1536,
            trim_bypass: true,
        }),
        previous: None,
    });
    let summary = w.finish().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        entry(&summary.losses, "importance-omitted").map(|e| (e.0, e.2)),
        Some((1, vec![(3, 0)]))
    );
    assert_eq!(
        entry(&summary.losses, "trim-bypass-dropped").map(|e| (e.0, e.2)),
        Some((1, vec![(3, 0)]))
    );
}

// ---------------------------------------------------------------------------
// The ADM profile guard (task 2).

/// The `axml` chunk of an ADM BWF file, read without the writer's help.
fn axml_of(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        if id == b"axml" {
            return String::from_utf8_lossy(&bytes[body..body + size]).into_owned();
        }
        // the data chunk of an RF64 file has its size in ds64; these are small
        at = body + size + (size & 1);
    }
    panic!("{} has no axml chunk", path.display());
}

/// Instances of `<name ` (an opening tag, not a reference).
fn instances(axml: &str, name: &str) -> usize {
    axml.matches(&format!("<{name} ")).count()
}

/// The largest programme the writer takes, a 7.1.2 bed and 118 objects, stays
/// inside every limit of the profile: what the guard counts is counted here
/// by hand as well, and the guard finds nothing.
#[test]
fn the_largest_programme_the_writer_takes_is_inside_the_profile() {
    let dir = temp("largest");
    let program = Program {
        beds: vec![vec![BedChannel::LFE]],
        isf_index: None,
        isf_objects: 0,
        dynamic_objects: MAX_OBJECTS,
    };
    let path = dir.join("max.wav");
    let mut w = AdmWriter::create(&path, &program, 48_000, &AdmOptions::default()).unwrap();
    let rows = vec![[0i32; 1 + MAX_OBJECTS]; 480];
    w.write_frames(rows.iter().map(|r| &r[..]), 1 + MAX_OBJECTS)
        .unwrap();
    for k in 0..MAX_OBJECTS as u32 {
        w.push_event(&Event {
            id: 10 + k,
            sample_pos: 0,
            state: ElementState::Object(ObjectState {
                active: true,
                pos: [0.0, 1.0, 0.0],
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
            }),
            previous: None,
        });
    }
    let summary = w.finish().unwrap();
    assert!(summary.profile.is_empty(), "{:?}", summary.profile);
    let axml = axml_of(&path);
    std::fs::remove_dir_all(&dir).ok();
    let objects = instances(&axml, "audioObject");
    assert_eq!(objects, 1 + MAX_OBJECTS, "one bed object and the objects");
    assert!(objects <= MAX_ELEMENTS);
    assert!(instances(&axml, "audioPackFormat") <= MAX_ELEMENTS);
    for name in [
        "audioChannelFormat",
        "audioStreamFormat",
        "audioTrackFormat",
        "audioTrackUID",
    ] {
        let n = instances(&axml, name);
        assert!(
            n <= MAX_TRACKS,
            "{name}: {n} instances, more than {MAX_TRACKS}"
        );
    }
    assert!(adm::profile_overruns(&axml).is_empty());
}

/// What the guard says of a chunk that has more than the profile allows: one
/// line per kind of element, on stderr, beside the loss lines; the file is
/// written as it was (the guard reads the chunk after it is written).
#[test]
fn the_guard_names_what_exceeds_the_profile() {
    let mut axml = String::from("<audioFormatExtended>\n");
    for k in 0..124 {
        axml.push_str(&format!(
            "<audioObject audioObjectID=\"AO_{:04x}\" audioObjectName=\"o\">\n</audioObject>\n",
            0x1001 + k
        ));
    }
    for k in 0..129 {
        axml.push_str(&format!(
            "<audioTrackUID UID=\"ATU_{:08x}\" sampleRate=\"48000\" bitDepth=\"24\">\n</audioTrackUID>\n",
            k + 1
        ));
    }
    axml.push_str("</audioFormatExtended>\n");
    assert_eq!(
        adm::profile_overruns(&axml),
        [
            "adm: profile limit: 124 audioObject elements, more than the 123 the profile allows (MAX_ELEMENT_COUNT)",
            "adm: profile limit: 129 audioTrackUID elements, more than the 128 the profile allows (MAX_CHANNEL_COUNT)",
        ]
    );
}

// ---------------------------------------------------------------------------
// What the timeline keeps of what it does not write (task 3).

/// The distance, the divergence and the screen reference of an object's last
/// update, the trim configurations and the bed channel distribution flag are
/// kept beside the reduced state.
#[test]
fn what_no_output_writes_is_kept_beside_the_state() {
    let mut p = payload();
    p.program.bed_chan_distribute = true;
    {
        let render = &mut object_element(&mut p).objects[1][0].render;
        render.distance = Distance::Factor(2.0);
        render.screen_ref = Some(ScreenRef {
            screen_factor: 0.5,
            depth_factor: 1.0,
        });
    }
    p.elements.push(element_md(
        5,
        Element::ExtendedObject(ExtendedObjectElement {
            divergence: Some(vec![vec![0.0], vec![0.5]]),
            ext_precision: None,
        }),
    ));
    let configs = vec![TrimConfig::default(); 9];
    p.elements.push(element_md(
        2,
        Element::Trim(TrimElement {
            warp_mode: 1,
            global_trim_mode: 2,
            configs: configs.clone(),
            disable_per_object: None,
        }),
    ));
    let t = run(&[p]);
    assert_eq!(
        t.extras.get(&10),
        Some(&Extras {
            distance: Distance::Factor(2.0),
            divergence: 0.5,
            screen_ref: Some(ScreenRef {
                screen_factor: 0.5,
                depth_factor: 1.0
            }),
        })
    );
    assert_eq!(t.extras.get(&3), None, "a bed channel has no render info");
    assert_eq!(t.trim, Some(configs));
    assert_eq!(t.bed_chan_distribute, Some(true));
}

/// Pushes `payloads` through a timeline into a DAMF set and an ADM file and
/// gives the bytes of the four files and the events.
fn written(tag: &str, payloads: &[Oamd]) -> (Vec<Vec<u8>>, u64) {
    let dir = temp(tag);
    let program = Program::from_oamd(&payloads[0]);
    let mut damf = DamfWriter::create(&dir, "d", &program, 48_000, &DamfOptions::default())
        .expect("a DAMF set");
    let mut adm = AdmWriter::create(&dir.join("a.wav"), &program, 48_000, &AdmOptions::default())
        .expect("an ADM file");
    let mut t = Timeline::new(false);
    let rows = vec![[1i32, 2]; 1536];
    for (k, p) in payloads.iter().enumerate() {
        t.push(p, k as u64 * 1536, 0, |e| {
            damf.push_event(e).unwrap();
            adm.push_event(e);
        })
        .unwrap();
        damf.write_frames(rows.iter().map(|r| &r[..]), 2).unwrap();
        adm.write_frames(rows.iter().map(|r| &r[..]), 2).unwrap();
    }
    damf.finish().unwrap();
    adm.finish().unwrap();
    let files = ["d.atmos", "d.atmos.metadata", "d.atmos.audio", "a.wav"]
        .iter()
        .map(|f| std::fs::read(dir.join(f)).unwrap())
        .collect();
    std::fs::remove_dir_all(&dir).ok();
    (files, t.events)
}

/// The extras are no part of the event decision: an update that changes only
/// the distance or the divergence is no event, and the four files are the
/// same bytes as those of a stream that never said either.
#[test]
fn an_update_that_changes_only_what_is_kept_beside_writes_the_same_files() {
    let plain = payload();
    let mut far = payload();
    object_element(&mut far).objects[1][0].render.distance = Distance::Factor(2.0);
    far.elements.push(element_md(
        5,
        Element::ExtendedObject(ExtendedObjectElement {
            divergence: Some(vec![vec![0.0], vec![0.5]]),
            ext_precision: None,
        }),
    ));
    let (a, events_a) = written("extras-plain", &[plain.clone(), plain.clone(), plain]);
    let (b, events_b) = written("extras-far", &[payload(), far.clone(), far]);
    assert_eq!(events_a, events_b, "the same events");
    for (k, (x, y)) in a.iter().zip(&b).enumerate() {
        assert!(x == y, "file {k} differs");
    }
}

// ---------------------------------------------------------------------------
// The event store of the ADM writer (the seam of round 5's event spool).

/// A store that keeps every event in one list, as a spool on disk would, and
/// can be told to fail.
#[derive(Debug, Default)]
struct Log {
    all: RefCell<Vec<(u32, u64, ObjectState)>>,
    fail: bool,
}

impl Events for Log {
    fn push(&mut self, id: u32, sample_pos: u64, state: &ObjectState) -> io::Result<()> {
        if self.fail {
            return Err(io::Error::other("the store is full"));
        }
        self.all.borrow_mut().push((id, sample_pos, state.clone()));
        Ok(())
    }

    fn of(&self, id: u32) -> io::Result<Vec<(u64, ObjectState)>> {
        Ok(self
            .all
            .borrow()
            .iter()
            .filter(|(i, _, _)| *i == id)
            .map(|(_, p, s)| (*p, s.clone()))
            .collect())
    }
}

fn adm_bytes<E: Events>(tag: &str, events: E) -> Result<Vec<u8>, String> {
    let dir = temp(tag);
    let path = dir.join("e.wav");
    let program = Program::from_oamd(&payload());
    let mut w =
        AdmWriter::create_with(&path, &program, 48_000, &AdmOptions::default(), events).unwrap();
    let rows = vec![[0i32, 0]; 9_600];
    w.write_frames(rows.iter().map(|r| &r[..]), 2).unwrap();
    let mut moving = payload();
    let mut t = Timeline::new(false);
    for k in 0..6u64 {
        object_element(&mut moving).objects[1][0]
            .render
            .position_code = [k as i8 * 10, 31, 0];
        t.push(&moving, k * 1536, 0, |e| w.push_event(e)).unwrap();
    }
    let result = w.finish().map_err(|e| e.to_string());
    let bytes = std::fs::read(&path).unwrap_or_default();
    std::fs::remove_dir_all(&dir).ok();
    result.map(|_| bytes)
}

/// Whichever store holds the events, the file is the same bytes; a store
/// that fails makes `finish` fail with its error.
#[test]
fn the_file_does_not_depend_on_where_the_events_waited() {
    let memory = adm_bytes("store-memory", MemoryEvents::default()).unwrap();
    let log = adm_bytes("store-log", Log::default()).unwrap();
    assert!(memory == log, "the two stores wrote different files");
    let failed = adm_bytes(
        "store-fail",
        Log {
            fail: true,
            ..Log::default()
        },
    );
    assert_eq!(failed, Err("the store is full".to_string()));
}
