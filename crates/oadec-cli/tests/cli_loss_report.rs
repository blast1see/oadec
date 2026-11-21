//! The loss report of the object outputs, held to its schema and to what it
//! says today.
//!
//! `--loss-report` writes the ledger of what a DAMF set or an ADM BWF file
//! does not carry of its programme, and stderr prints the same ledger a class
//! to a line. Both may change on purpose (a new kind, a new count); nothing
//! else of an object output may. So the report has a schema,
//! `docs/schemas/loss-report.schema.json`, and every report these tests make
//! is checked against it by the small validator below (no dependency is added
//! for it: it reads the keywords the schema uses and refuses any other). What
//! each fixture's report and loss lines say is recorded under
//! `tests/loss-golden/`, one file per stream; after a change that is meant to
//! move them:
//!
//! ```text
//! OADEC_BLESS=1 cargo test -p oadec-cli --test cli_loss_report
//! ```
//!
//! and read the difference before committing it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use oadec_spatial::{LossClass, LossKind};
use serde_json::{Value, json};

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn schema() -> Value {
    let path = manifest_dir().join("../../docs/schemas/loss-report.schema.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} cannot be read: {e}", path.display()));
    serde_json::from_str(&text).expect("the schema is JSON")
}

// ---------------------------------------------------------------------------
// The validator: the part of JSON Schema 2020-12 the loss-report schema uses.

/// Keywords that say something about a value and are checked.
const CHECKED: [&str; 11] = [
    "type",
    "enum",
    "required",
    "properties",
    "additionalProperties",
    "patternProperties",
    "items",
    "minimum",
    "maxItems",
    "$ref",
    "$defs",
];

/// Keywords that only annotate.
const ANNOTATIONS: [&str; 4] = ["$schema", "title", "description", "x-oadec-kind-class"];

fn type_matches(name: &str, v: &Value) -> bool {
    match name {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "null" => v.is_null(),
        other => panic!("the validator knows no type {other}"),
    }
}

/// The one pattern the schema uses for a property name.
fn pattern_matches(pattern: &str, name: &str) -> bool {
    match pattern {
        "^[0-9]+$" => !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()),
        other => panic!("the validator knows no pattern {other}"),
    }
}

fn resolve<'a>(root: &'a Value, reference: &str) -> &'a Value {
    let name = reference.strip_prefix("#/$defs/").unwrap_or_else(|| {
        panic!("the validator follows only #/$defs/ references, not {reference}")
    });
    root["$defs"]
        .get(name)
        .unwrap_or_else(|| panic!("{reference} names no definition"))
}

/// Every way `value` breaks `schema`, as `path: what`.
fn validate(root: &Value, schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(rules) = schema.as_object() else {
        panic!("{path}: a schema is an object");
    };
    for key in rules.keys() {
        assert!(
            CHECKED.contains(&key.as_str()) || ANNOTATIONS.contains(&key.as_str()),
            "{path}: the validator does not check the keyword {key}; teach it before the schema uses it"
        );
    }
    if let Some(reference) = rules.get("$ref").and_then(Value::as_str) {
        validate(root, resolve(root, reference), value, path, errors);
    }
    if let Some(types) = rules.get("type") {
        let ok = match types {
            Value::String(t) => type_matches(t, value),
            Value::Array(ts) => ts.iter().any(|t| type_matches(t.as_str().unwrap(), value)),
            _ => panic!("{path}: type is a name or a list of names"),
        };
        if !ok {
            errors.push(format!("{path}: {value} is not of type {types}"));
            return;
        }
    }
    if let Some(Value::Array(allowed)) = rules.get("enum")
        && !allowed.contains(value)
    {
        errors.push(format!(
            "{path}: {value} is not one of {}",
            Value::Array(allowed.clone())
        ));
    }
    if let Some(min) = rules.get("minimum").and_then(Value::as_i64)
        && let Some(n) = value.as_i64().or_else(|| value.as_u64().map(|_| i64::MAX))
        && n < min
    {
        errors.push(format!("{path}: {n} is below {min}"));
    }
    if let Value::Array(items) = value {
        if let Some(max) = rules.get("maxItems").and_then(Value::as_u64)
            && items.len() as u64 > max
        {
            errors.push(format!("{path}: {} items, more than {max}", items.len()));
        }
        if let Some(item) = rules.get("items") {
            for (i, v) in items.iter().enumerate() {
                validate(root, item, v, &format!("{path}[{i}]"), errors);
            }
        }
    }
    if let Value::Object(map) = value {
        if let Some(Value::Array(required)) = rules.get("required") {
            for name in required {
                let name = name.as_str().unwrap();
                if !map.contains_key(name) {
                    errors.push(format!("{path}: {name} is missing"));
                }
            }
        }
        let properties = rules.get("properties").and_then(Value::as_object);
        let patterns = rules.get("patternProperties").and_then(Value::as_object);
        for (name, v) in map {
            let at = format!("{path}.{name}");
            let mut known = false;
            if let Some(s) = properties.and_then(|p| p.get(name)) {
                known = true;
                validate(root, s, v, &at, errors);
            }
            for (pattern, s) in patterns.into_iter().flatten() {
                if pattern_matches(pattern, name) {
                    known = true;
                    validate(root, s, v, &at, errors);
                }
            }
            if !known && rules.get("additionalProperties") == Some(&Value::Bool(false)) {
                errors.push(format!("{at}: no such property"));
            }
        }
    }
}

fn violations(report: &Value) -> Vec<String> {
    let root = schema();
    let mut errors = Vec::new();
    validate(&root, &root, report, "$", &mut errors);
    // the one rule the keywords above do not say: a kind has one class
    for loss in report["losses"].as_array().into_iter().flatten() {
        let (Some(kind), Some(class)) = (loss["kind"].as_str(), loss["class"].as_str()) else {
            continue;
        };
        if let Some(want) = root["x-oadec-kind-class"][kind].as_str()
            && want != class
        {
            errors.push(format!(
                "$.losses: {kind} is reported as {class}, not {want}"
            ));
        }
    }
    errors
}

fn assert_valid(report: &Value, what: &str) {
    let errors = violations(report);
    assert!(
        errors.is_empty(),
        "{what} breaks the schema:\n{}",
        errors.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The schema against the code and against reports it must refuse.

/// The kinds and classes of the schema are the ones the ledger has, in
/// reporting order, each kind with its class.
#[test]
fn the_schema_names_the_kinds_and_classes_of_the_ledger() {
    let root = schema();
    let names = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    let kinds: Vec<String> = LossKind::ALL.iter().map(|k| k.name().to_string()).collect();
    assert_eq!(names(&root["$defs"]["kind"]["enum"]), kinds);
    let classes: Vec<String> = LossClass::ALL
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    assert_eq!(names(&root["$defs"]["class"]["enum"]), classes);
    let map = root["x-oadec-kind-class"].as_object().unwrap();
    assert_eq!(map.len(), LossKind::ALL.len());
    for kind in LossKind::ALL {
        assert_eq!(
            map[kind.name()].as_str(),
            Some(kind.class().name()),
            "{}",
            kind.name()
        );
    }
    assert_eq!(
        root["properties"]["schema"]["enum"],
        json!([oadec_spatial::loss::REPORT_SCHEMA]),
        "the report names the version of the schema it follows"
    );
    // the kinds whose class sets exit code 4, as docs/exit-codes.md has them
    let declared: Vec<&str> = LossKind::ALL
        .iter()
        .filter(|k| k.class() == LossClass::DeclaredLoss)
        .map(|k| k.name())
        .collect();
    assert_eq!(
        declared,
        [
            "isf-dropped",
            "out-of-order-written-as-is",
            "non-profile-sample-rate"
        ]
    );
}

/// A report that is wrong in each way the schema can say is refused, and the
/// right one is not: a validator that accepts everything would pass every
/// golden below.
#[test]
fn the_validator_refuses_what_the_schema_refuses() {
    let good = json!({
        "schema": 2,
        "target": "adm",
        "declared_loss": true,
        "losses": [
            {"kind": "ramp-replaced", "class": "profile-reduction", "count": 4,
             "examples": [{"element": 11, "sample": 0}, {"element": 11, "sample": 1536},
                          {"element": 12, "sample": 0}]},
            {"kind": "isf-dropped", "class": "declared-loss", "count": 10,
             "examples": [{"element": 0, "sample": 0}]}
        ],
        "ramp_sources": {"1536": 3, "32": 1},
        "overrides": [
            {"flag": "joc-lag", "value": 3, "default": 2},
            {"flag": "joc-low-band", "value": "plain", "default": "filtered"}
        ]
    });
    assert_valid(&good, "the good report");

    type Change<'a> = &'a dyn Fn(&mut Value);
    let broken = |change: Change| -> Vec<String> {
        let mut v = good.clone();
        change(&mut v);
        violations(&v)
    };
    let cases: [(&str, Change); 15] = [
        ("no schema", &|v| {
            v.as_object_mut().unwrap().remove("schema");
        }),
        ("schema 1", &|v| v["schema"] = json!(1)),
        ("no target", &|v| {
            v.as_object_mut().unwrap().remove("target");
        }),
        ("another target", &|v| v["target"] = json!("wav")),
        ("a field more", &|v| v["comment"] = json!("x")),
        ("declared_loss as text", &|v| {
            v["declared_loss"] = json!("no")
        }),
        ("an unknown kind", &|v| {
            v["losses"][0]["kind"] = json!("gain-dropped")
        }),
        ("an unknown class", &|v| {
            v["losses"][0]["class"] = json!("loss")
        }),
        ("a kind in another class", &|v| {
            v["losses"][1]["class"] = json!("approximation");
        }),
        ("a count of zero", &|v| v["losses"][0]["count"] = json!(0)),
        ("four examples", &|v| {
            v["losses"][0]["examples"]
                .as_array_mut()
                .unwrap()
                .push(json!({"element": 1, "sample": 1}));
        }),
        ("a negative sample", &|v| {
            v["losses"][0]["examples"][0]["sample"] = json!(-1);
        }),
        ("a ramp that is not a number", &|v| {
            v["ramp_sources"]["long"] = json!(1)
        }),
        ("an unknown override", &|v| {
            v["overrides"][0]["flag"] = json!("joc-gain")
        }),
        ("an override without a default", &|v| {
            v["overrides"][1].as_object_mut().unwrap().remove("default");
        }),
    ];
    for (what, change) in cases {
        assert!(!broken(change).is_empty(), "the validator accepted {what}");
    }
}

// ---------------------------------------------------------------------------
// What the fixtures' reports say today.

fn golden_dir() -> PathBuf {
    manifest_dir().join("tests").join("loss-golden")
}

fn blessing() -> bool {
    std::env::var_os("OADEC_BLESS").is_some_and(|v| v == "1")
}

/// The streams whose object outputs are recorded: every stream of the two
/// fixture directories, the ones that are no object programme too (what they
/// say is a refusal, and that is part of the contract), and, as relative paths
/// from this crate, so that a recorded line is the same on every machine.
const STREAMS: [&str; 15] = [
    "tests/fixtures/authored-scene.ec3",
    "tests/fixtures/authored-scene.mlp",
    "tests/fixtures/mlp-41-48k.mlp",
    "tests/fixtures/mlp-51-96k.mlp",
    "tests/fixtures/mlp-quad-48k.mlp",
    "tests/fixtures/mlp-stereo-44k.mlp",
    "tests/fixtures/truehd-20-176k.thd",
    "tests/fixtures/truehd-20-192k.thd",
    "tests/fixtures/truehd-20-44k.thd",
    "tests/fixtures/truehd-51-88k.thd",
    "../oadec-eac3/tests/fixtures/ac3-20-coupled.ac3",
    "../oadec-eac3/tests/fixtures/eac3-20-coupled.ec3",
    "../oadec-eac3/tests/fixtures/eac3-71-tones.ec3",
    "../oadec-eac3/tests/fixtures/joc-512-tones.ec3",
    // written by the test below, not committed: an ISF programme
    "isf-sr7.3.0.0.thd",
];

/// Options a stream is run with besides the defaults, one run each; every
/// stream also runs with none.
fn variants(stream: &str) -> Vec<Vec<&'static str>> {
    let name = stream.rsplit('/').next().unwrap();
    match name {
        // the source ramps: no ramp is replaced, and the file says it is outside the profile
        "authored-scene.mlp" => vec![vec!["--adm-interpolation", "real"]],
        // a measurement override is recorded; the warp mode the stream declares is kept
        "authored-scene.ec3" => vec![vec!["--joc-lag", "3"], vec!["--warp-mode", "stream"]],
        // the ISF objects: refused by default, dropped and declared on request
        "isf-sr7.3.0.0.thd" => vec![vec!["--isf", "drop"]],
        _ => Vec::new(),
    }
}

/// Options that only one of the outputs takes.
fn applies(format: &str, options: &[&str]) -> bool {
    !(format == "damf" && options.first() == Some(&"--adm-interpolation"))
}

fn golden_name(stream: &str) -> String {
    format!(
        "{}.txt",
        stream.rsplit('/').next().unwrap().replace('.', "-")
    )
}

/// The lines of stderr that are the loss summary (`damf: …`, `adm: …`) or a
/// refusal (`error: …`).
fn ledger_lines(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|l| l.starts_with("damf: ") || l.starts_with("adm: ") || l.starts_with("error: "))
        .collect()
}

/// Runs each output of `stream` with each set of options and records the exit
/// code, the loss lines and the report, checking each report against the
/// schema on the way.
fn record(stream: &str, path: &Path, out: &Path) -> String {
    let mut t = String::new();
    let mut runs = vec![Vec::new()];
    runs.extend(variants(stream));
    for options in &runs {
        for format in ["damf", "adm"] {
            if !applies(format, options) {
                continue;
            }
            let report = out.join("loss.json");
            let _ = std::fs::remove_file(&report);
            let base = out.join(format);
            let output = Command::new(env!("CARGO_BIN_EXE_oadec"))
                .arg("decode")
                .arg(path)
                .args(["--format", format])
                .args(options)
                .arg("--loss-report")
                .arg(&report)
                .arg("-o")
                .arg(&base)
                .current_dir(manifest_dir())
                .output()
                .expect("run oadec");
            let mut shown = vec!["decode", stream, "--format", format];
            shown.extend(options.iter());
            shown.extend(["--loss-report", "<out>/loss.json", "-o", "<out>/x"]);
            writeln!(t, "$ oadec {}", shown.join(" ")).unwrap();
            match output.status.code() {
                Some(code) => writeln!(t, "exit {code}").unwrap(),
                None => writeln!(t, "no exit code").unwrap(),
            }
            let stderr = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");
            let lines = ledger_lines(&stderr);
            if lines.is_empty() {
                t.push_str("[stderr ledger] none\n");
            } else {
                t.push_str("[stderr ledger]\n");
                for line in lines {
                    writeln!(t, "{line}").unwrap();
                }
            }
            match std::fs::read(&report) {
                Ok(bytes) => {
                    let value: Value = serde_json::from_slice(&bytes)
                        .unwrap_or_else(|e| panic!("{stream} {format} {options:?}: {e}"));
                    assert_valid(&value, &format!("{stream} {format} {options:?}"));
                    // as written, byte for byte: the layout is part of what a reader meets
                    t.push_str("[loss report]\n");
                    t.push_str(&String::from_utf8_lossy(&bytes).replace("\r\n", "\n"));
                }
                Err(_) => t.push_str("[loss report] not written\n"),
            }
            t.push('\n');
        }
    }
    t
}

/// The example of the TrueHD description (section 4.4) that `cli_isf.rs`
/// writes: C, LFE, the ten channels of SR7.3.0.0 and two objects, declared
/// by the major sync; no object metadata.
fn write_isf_stream(path: &Path) {
    use oadec_synth::signal::noise;
    use oadec_synth::{Above, Coding, Family, Layout, Sixteen, UnitPlan, write};
    let objects = 6;
    let mut layout = Layout::new(Family::TrueHd, 48_000, 2);
    layout.above = vec![Above::new(4), Above::new(2), Above::objects(objects)];
    layout.sixteen = Some(Sixteen {
        channel_count: None,
        content_description: 0b111,
        lfe_only: false,
        assignment: 0b00_0000_0110,
        isf: 2,
        dynamic_object_count: 1,
    });
    let units = 16;
    let per_unit = layout.samples_per_unit();
    let pcm: Vec<Vec<i32>> = (0..8 + objects)
        .map(|ch| noise(units * per_unit, 14, 100 + ch as u64))
        .collect();
    let plain = |n: usize| vec![Coding::default(); n];
    let plan = UnitPlan::uniform(&[per_unit], &plain(2))
        .over(UnitPlan::uniform(&[per_unit], &plain(4)))
        .over(UnitPlan::uniform(&[per_unit], &plain(2)))
        .over(UnitPlan::uniform(&[per_unit], &plain(objects)));
    let stream = write(&layout, &vec![plan; units], &pcm).expect("the ISF stream");
    std::fs::write(path, &stream.bytes).unwrap();
}

fn hold(name: &str, actual: &str) {
    let path = golden_dir().join(name);
    if blessing() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            panic!(
                "{} cannot be read ({e}); a report with nothing to compare with is not a pass. \
                 OADEC_BLESS=1 records one.",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    if golden == actual {
        return;
    }
    let (mut want, mut got) = (golden.lines(), actual.lines());
    let mut line = 1;
    loop {
        match (want.next(), got.next()) {
            (Some(w), Some(g)) if w == g => line += 1,
            (w, g) => panic!(
                "{name} differs from this run at line {line}:\n  recorded: {}\n  this run: {}\n\
                 A loss report changes only with a schema and a list of the differences \
                 approved (round 5, P3); if it is meant, OADEC_BLESS=1 records it.",
                w.unwrap_or("<the record ends>"),
                g.unwrap_or("<the record ends>")
            ),
        }
    }
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oadec-loss-report-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn check(stream: &str) {
    assert!(STREAMS.contains(&stream), "{stream} is not in STREAMS");
    let out = temp(&golden_name(stream).replace(".txt", ""));
    let path = if stream.contains('/') {
        PathBuf::from(stream)
    } else {
        let p = out.join(stream);
        write_isf_stream(&p);
        p
    };
    let text = record(stream, &path, &out);
    std::fs::remove_dir_all(&out).ok();
    hold(&golden_name(stream), &text);
}

macro_rules! goldens {
    ($($test:ident => $stream:literal),+ $(,)?) => {
        $(
            #[test]
            fn $test() {
                check($stream);
            }
        )+

        const COVERED: &[&str] = &[$($stream),+];
    };
}

goldens! {
    authored_scene_ec3 => "tests/fixtures/authored-scene.ec3",
    authored_scene_mlp => "tests/fixtures/authored-scene.mlp",
    mlp_41_48k => "tests/fixtures/mlp-41-48k.mlp",
    mlp_51_96k => "tests/fixtures/mlp-51-96k.mlp",
    mlp_quad_48k => "tests/fixtures/mlp-quad-48k.mlp",
    mlp_stereo_44k => "tests/fixtures/mlp-stereo-44k.mlp",
    truehd_20_176k => "tests/fixtures/truehd-20-176k.thd",
    truehd_20_192k => "tests/fixtures/truehd-20-192k.thd",
    truehd_20_44k => "tests/fixtures/truehd-20-44k.thd",
    truehd_51_88k => "tests/fixtures/truehd-51-88k.thd",
    ac3_20_coupled => "../oadec-eac3/tests/fixtures/ac3-20-coupled.ac3",
    eac3_20_coupled => "../oadec-eac3/tests/fixtures/eac3-20-coupled.ec3",
    eac3_71_tones => "../oadec-eac3/tests/fixtures/eac3-71-tones.ec3",
    joc_512_tones => "../oadec-eac3/tests/fixtures/joc-512-tones.ec3",
    isf_sr7_3_0_0 => "isf-sr7.3.0.0.thd",
}

/// A stream added to either fixture directory and to no golden here would be
/// one whose report nothing holds.
#[test]
fn every_fixture_stream_has_a_golden() {
    let mut found: Vec<String> = Vec::new();
    for dir in ["tests/fixtures", "../oadec-eac3/tests/fixtures"] {
        for e in std::fs::read_dir(manifest_dir().join(dir)).expect("a fixture directory") {
            let name = e.unwrap().file_name().to_string_lossy().into_owned();
            if [".ec3", ".ac3", ".mlp", ".thd"]
                .iter()
                .any(|x| name.ends_with(x))
            {
                found.push(format!("{dir}/{name}"));
            }
        }
    }
    found.sort();
    let mut listed: Vec<String> = STREAMS
        .iter()
        .filter(|s| s.contains('/'))
        .map(|s| (*s).to_string())
        .collect();
    listed.sort();
    assert_eq!(
        found, listed,
        "the fixture directories and STREAMS disagree"
    );
    let mut covered: Vec<&str> = COVERED.to_vec();
    covered.sort_unstable();
    let mut all: Vec<&str> = STREAMS.to_vec();
    all.sort_unstable();
    assert_eq!(covered, all, "a stream of STREAMS has no test");
}

/// A golden nothing writes any more would stay and be read as coverage.
#[test]
fn every_golden_belongs_to_a_stream() {
    if blessing() {
        return;
    }
    let mut wanted: Vec<String> = STREAMS.iter().map(|s| golden_name(s)).collect();
    wanted.sort();
    let mut found: Vec<String> = std::fs::read_dir(golden_dir())
        .expect("the golden directory")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    assert_eq!(found, wanted, "tests/loss-golden and the tests disagree");
}
