//! Audit harness for oadec's ADM BWF and DAMF writers.
//!
//! Reads a case description (JSON), feeds the *same* events to `AdmWriter`
//! and `DamfWriter` exactly as `oadec decode` does through its `Sink`, writes
//! `<case>.wav`, `<case>.atmos{,.metadata,.audio}` and `<case>.summary.json`.
//! Two input modes:
//!   * `events`  -- explicit `Event`s (object or bed states at sample positions);
//!   * `oamd`    -- OAMD payloads built as `oadec_emdf::oamd::Oamd` structs and
//!                  pushed through `Timeline::push`, which is the only way to
//!                  exercise the programme model (3-axis size, distance, screen
//!                  reference, ISF, restatement) without touching the parser.
//! No production source is modified; the writers are used through their public API.

use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

use oadec_emdf::oamd::{
    BasicInfo, Bed, BedChannel, BlockTiming, Distance, Element, ElementMd, Gain, Oamd,
    ObjectElement, ObjectInfoBlock, ProgramAssignment, RenderInfo, ScreenRef, Status,
    UpdateTiming,
};
use oadec_spatial::program::{damf_channel_name, BedState, ElementState, Event, ObjectState, Program, Timeline};
use oadec_spatial::{AdmOptions, AdmWriter, DamfOptions, DamfWriter, IsfPolicy};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    case: String,
    #[serde(default = "d48k")]
    sample_rate: u32,
    frames: u64,
    program: ProgramSpec,
    #[serde(default = "d_true")]
    bed_conform: bool,
    #[serde(default)]
    creator: Option<String>,
    #[serde(default)]
    audio: AudioSpec,
    #[serde(default)]
    events: Vec<EventSpec>,
    #[serde(default)]
    oamd: Vec<OamdSpec>,
    #[serde(default)]
    keep_all: bool,
    /// Write the output without the ISF elements instead of refusing (the writers' default).
    #[serde(default)]
    isf_drop: bool,
}

fn d48k() -> u32 {
    48_000
}
fn d_true() -> bool {
    true
}

#[derive(Deserialize)]
struct ProgramSpec {
    beds: Vec<Vec<String>>,
    #[serde(default)]
    isf_objects: usize,
    dynamic_objects: usize,
}

#[derive(Deserialize, Default)]
struct AudioSpec {
    /// Hz per element in stream order; 0 = silence. Missing entries = silence.
    #[serde(default)]
    hz: Vec<f64>,
    #[serde(default = "d_dbfs")]
    dbfs: f64,
}
fn d_dbfs() -> f64 {
    -20.0
}

#[derive(Deserialize)]
struct EventSpec {
    id: u32,
    sample_pos: u64,
    #[serde(default)]
    object: Option<ObjectSpec>,
    #[serde(default)]
    bed: Option<BedSpec>,
}

#[derive(Deserialize, Clone)]
struct ObjectSpec {
    #[serde(default = "d_true")]
    active: bool,
    pos: [f32; 3],
    #[serde(default)]
    snap: bool,
    #[serde(default = "d_true")]
    elevation: bool,
    #[serde(default)]
    zones: u8,
    #[serde(default)]
    size: f32,
    #[serde(default = "d_one")]
    importance: f32,
    #[serde(default)]
    gain_db: Option<i8>,
    #[serde(default)]
    gain_minus_inf: bool,
    #[serde(default)]
    ramp: u32,
    #[serde(default)]
    trim_bypass: bool,
    #[serde(default)]
    screen_factor: f32,
    #[serde(default = "d_quarter")]
    depth_factor: f32,
}
fn d_one() -> f32 {
    1.0
}
fn d_quarter() -> f32 {
    0.25
}

#[derive(Deserialize, Clone)]
struct BedSpec {
    #[serde(default = "d_true")]
    active: bool,
    #[serde(default = "d_one")]
    importance: f32,
    #[serde(default)]
    gain_db: Option<i8>,
    #[serde(default)]
    gain_minus_inf: bool,
    #[serde(default)]
    ramp: u32,
    #[serde(default)]
    trim_bypass: bool,
}

/// A simplified OAMD payload; positions are OAMD room coordinates (x, y in
/// 0..=1, z in -1..=1) and are quantised to the 6/6/5-bit codes the parser
/// would have produced.
#[derive(Deserialize)]
struct OamdSpec {
    base: u64,
    #[serde(default)]
    container_offset: u64,
    #[serde(default)]
    sample_offset: u16,
    blocks: Vec<(u8, u16)>,
    #[serde(default)]
    isf_index: Option<u8>,
    objects: Vec<Vec<OamdBlockSpec>>,
}

#[derive(Deserialize, Clone)]
struct OamdBlockSpec {
    #[serde(default)]
    not_active: bool,
    #[serde(default)]
    gain_db: Option<i8>,
    #[serde(default)]
    gain_minus_inf: bool,
    #[serde(default = "d_one")]
    priority: f32,
    #[serde(default)]
    pos: Option<[f32; 3]>,
    #[serde(default)]
    size: Option<[f32; 3]>,
    #[serde(default)]
    zone: u8,
    #[serde(default = "d_true")]
    elevation: bool,
    #[serde(default)]
    snap: bool,
    #[serde(default)]
    distance: Option<String>,
    #[serde(default)]
    screen: Option<(f32, f32)>,
}

fn gain(db: Option<i8>, minus_inf: bool) -> Gain {
    if minus_inf {
        Gain::MinusInfinity
    } else {
        Gain::Db(db.unwrap_or(0))
    }
}

fn bed_channel(name: &str) -> Result<BedChannel, String> {
    BedChannel::ALL
        .into_iter()
        .find(|c| damf_channel_name(*c).eq_ignore_ascii_case(name) || format!("{c:?}").eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("unknown bed channel {name}"))
}

fn object_state(s: &ObjectSpec) -> ObjectState {
    ObjectState {
        active: s.active,
        pos: s.pos,
        snap: s.snap,
        elevation: s.elevation,
        zones: s.zones,
        size: s.size,
        importance: s.importance,
        gain: gain(s.gain_db, s.gain_minus_inf),
        ramp: s.ramp,
        trim_bypass: s.trim_bypass,
        screen_factor: s.screen_factor,
        depth_factor: s.depth_factor,
    }
}

fn bed_state(s: &BedSpec) -> BedState {
    BedState {
        active: s.active,
        importance: s.importance,
        gain: gain(s.gain_db, s.gain_minus_inf),
        ramp: s.ramp,
        trim_bypass: s.trim_bypass,
    }
}

fn to_i24(v: f64) -> i32 {
    (v * 8_388_607.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32
}

fn build_oamd(spec: &OamdSpec, program: &Program) -> Oamd {
    let bed_or_isf = program.bed_channels().len() + program.isf_objects;
    let beds: Vec<Bed> = program
        .beds
        .iter()
        .map(|chs| Bed { lfe_only: chs == &[BedChannel::LFE], standard: true, channels: chs.clone() })
        .collect();
    let objects: Vec<Vec<ObjectInfoBlock>> = spec
        .objects
        .iter()
        .enumerate()
        .map(|(index, blocks)| {
            blocks
                .iter()
                .map(|b| {
                    let in_bed_or_isf = index < bed_or_isf;
                    let mut render = RenderInfo::DEFAULT;
                    if let Some(p) = b.pos {
                        render.position_code = [
                            (p[0].clamp(0.0, 1.0) * 62.0).round() as i8,
                            (p[1].clamp(0.0, 1.0) * 62.0).round() as i8,
                            (p[2].clamp(-1.0, 1.0) * 15.0).round() as i8,
                        ];
                    }
                    if let Some(s) = b.size {
                        render.size = s;
                    }
                    render.zone_constraints = b.zone;
                    render.enable_elevation = b.elevation;
                    render.snap = b.snap;
                    render.distance = match b.distance.as_deref() {
                        None => Distance::Unspecified,
                        Some("inf") | Some("infinity") => Distance::Infinity,
                        Some(f) => Distance::Factor(f.parse().expect("distance factor")),
                    };
                    render.screen_ref = b.screen.map(|(s, d)| ScreenRef { screen_factor: s, depth_factor: d });
                    ObjectInfoBlock {
                        not_active: b.not_active,
                        in_bed_or_isf,
                        basic_status: Status::Full,
                        basic: BasicInfo { gain: gain(b.gain_db, b.gain_minus_inf), priority: b.priority },
                        render_status: if in_bed_or_isf { Status::Default } else { Status::Full },
                        render,
                        additional_table_data: Vec::new(),
                    }
                })
                .collect()
        })
        .collect();
    let element = ObjectElement {
        timing: UpdateTiming {
            sample_offset: spec.sample_offset,
            blocks: spec.blocks.iter().map(|&(bof, ramp)| BlockTiming { block_offset_factor: bof, ramp_duration: ramp }).collect(),
        },
        reserved: None,
        objects,
    };
    Oamd {
        version: 0,
        object_count: spec.objects.len(),
        program: ProgramAssignment {
            dyn_object_only: program.beds.len() == 1 && program.beds[0] == [BedChannel::LFE] && program.isf_objects == 0,
            content_description: 0,
            bed_chan_distribute: false,
            beds,
            isf_index: spec.isf_index,
            dynamic_objects: program.dynamic_objects,
            reserved_data: Vec::new(),
        },
        alternate_object_data_present: false,
        elements: vec![ElementMd {
            id: 1,
            size_bytes: 0,
            alternate_id: None,
            discard_unknown: false,
            element: Element::Object(element),
            size_ok: true,
            padding_bits: 0,
            padding_zero: true,
        }],
        padding_bits: 0,
        padding_zero: true,
    }
}

fn run(case_path: &Path, out_dir: &Path) -> Result<serde_json::Value, String> {
    let case: Case = serde_json::from_reader(BufReader::new(File::open(case_path).map_err(|e| e.to_string())?)).map_err(|e| format!("case json: {e}"))?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let beds: Result<Vec<Vec<BedChannel>>, String> = case.program.beds.iter().map(|b| b.iter().map(|n| bed_channel(n)).collect()).collect();
    // the ISF type whose object count matches (table 11b), for the programme's isf_index
    let isf_index = if case.program.isf_objects > 0 {
        oadec_emdf::oamd::ISF_OBJECTS.iter().position(|n| *n == Some(case.program.isf_objects)).map(|i| i as u8)
    } else {
        None
    };
    let isf = if case.isf_drop { IsfPolicy::Drop } else { IsfPolicy::Error };
    let program = Program { beds: beds?, isf_index, isf_objects: case.program.isf_objects, dynamic_objects: case.program.dynamic_objects };
    let elements = program.elements();

    // events: explicit, plus whatever the timeline emits for OAMD payloads
    let mut events: Vec<Event> = Vec::new();
    let mut previous: std::collections::BTreeMap<u32, ElementState> = Default::default();
    for e in &case.events {
        let state = match (&e.object, &e.bed) {
            (Some(o), None) => ElementState::Object(object_state(o)),
            (None, Some(b)) => ElementState::Bed(bed_state(b)),
            _ => return Err(format!("event for id {} must have exactly one of object/bed", e.id)),
        };
        events.push(Event { id: e.id, sample_pos: e.sample_pos, state: state.clone(), previous: previous.get(&e.id).cloned() });
        previous.insert(e.id, state);
    }
    let mut timeline = Timeline::new(case.keep_all);
    let mut timeline_errors: Vec<String> = Vec::new();
    let mut emitted = 0usize;
    for spec in &case.oamd {
        let oamd = build_oamd(spec, &program);
        match timeline.push(&oamd, spec.base, spec.container_offset, |ev| events.push(ev.clone())) {
            Ok(n) => emitted += n,
            Err(e) => timeline_errors.push(format!("{e}")),
        }
    }
    // explicit events are already in order; OAMD payloads arrive in order too, but a
    // mixed case could interleave: keep the writer's own view honest by not sorting.

    let adm_path: PathBuf = out_dir.join(format!("{}.wav", case.case));
    let mut opts = AdmOptions { bed_conform: case.bed_conform, isf, ..AdmOptions::default() };
    if let Some(c) = &case.creator {
        opts.creator = c.clone();
    }
    let mut summary = serde_json::json!({
        "case": case.case, "sample_rate": case.sample_rate, "frames": case.frames,
        "elements": elements, "events_fed": events.len(), "timeline_emitted": emitted,
        "timeline_errors": timeline_errors, "timeline_out_of_order": timeline.out_of_order,
        "timeline_restatements": timeline.restatements,
    });

    // audio rows: one tone per element (0 Hz = silence)
    let make_row = |n: u64, row: &mut Vec<i32>| {
        for (i, v) in row.iter_mut().enumerate() {
            let hz = case.audio.hz.get(i).copied().unwrap_or(0.0);
            *v = if hz == 0.0 {
                0
            } else {
                let a = 10f64.powf(case.audio.dbfs / 20.0);
                to_i24(a * (std::f64::consts::TAU * hz * n as f64 / f64::from(case.sample_rate)).sin())
            };
        }
    };

    // ---- ADM
    let adm_result: Result<serde_json::Value, String> = (|| {
        let mut w = AdmWriter::create(&adm_path, &program, case.sample_rate, &opts).map_err(|e| format!("create: {e}"))?;
        for ev in &events {
            w.push_event(ev);
        }
        let mut row = vec![0i32; elements];
        let mut block: Vec<i32> = Vec::with_capacity(elements * 4096);
        let mut n = 0u64;
        while n < case.frames {
            block.clear();
            let end = (n + 4096).min(case.frames);
            for k in n..end {
                make_row(k, &mut row);
                block.extend_from_slice(&row);
            }
            w.write_frames(block.chunks(elements), elements).map_err(|e| format!("write: {e}"))?;
            n = end;
        }
        let s = w.finish().map_err(|e| format!("finish: {e}"))?;
        Ok(serde_json::json!({"frames": s.frames, "channels": s.channels, "blocks": s.blocks, "rf64": s.rf64, "bytes": s.bytes, "path": adm_path}))
    })();
    match adm_result {
        Ok(v) => summary["adm"] = v,
        Err(e) => {
            summary["adm_error"] = serde_json::Value::String(e);
            let _ = std::fs::remove_file(&adm_path);
        }
    }

    // ---- DAMF (same events, same audio)
    let damf_result: Result<serde_json::Value, String> = (|| {
        let mut w = DamfWriter::create(out_dir, &case.case, &program, case.sample_rate, &DamfOptions { bed_conform: case.bed_conform, isf, ..DamfOptions::default() }).map_err(|e| format!("create: {e}"))?;
        for ev in &events {
            w.push_event(ev).map_err(|e| format!("event: {e}"))?;
        }
        let mut row = vec![0i32; elements];
        for n in 0..case.frames {
            make_row(n, &mut row);
            w.write_frame(&row).map_err(|e| format!("write: {e}"))?;
        }
        let s = w.finish().map_err(|e| format!("finish: {e}"))?;
        Ok(serde_json::json!({"frames": s.frames, "channels": s.channels, "events": s.events, "base": out_dir.join(&case.case)}))
    })();
    match damf_result {
        Ok(v) => summary["damf"] = v,
        Err(e) => summary["damf_error"] = serde_json::Value::String(e),
    }
    Ok(summary)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: adm_harness <case.json> <out dir>");
        std::process::exit(2);
    }
    let case_path = Path::new(&args[1]);
    let out_dir = Path::new(&args[2]);
    let (summary, code) = match run(case_path, out_dir) {
        Ok(s) => (s, 0),
        Err(e) => (serde_json::json!({"error": e}), 1),
    };
    let name = case_path.file_stem().and_then(|s| s.to_str()).unwrap_or("case");
    let _ = std::fs::create_dir_all(out_dir);
    let mut f = File::create(out_dir.join(format!("{name}.summary.json"))).expect("summary file");
    writeln!(f, "{}", serde_json::to_string_pretty(&summary).unwrap()).unwrap();
    println!("{}", serde_json::to_string(&summary).unwrap());
    std::process::exit(code);
}
