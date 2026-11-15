//! The census of a TrueHD stream as `verify --census` prints it.
//!
//! The counting is `oadec_truehd::census`; this turns the counters into a
//! report. In JSON every counter is there, used or not, so that a sweep over
//! many streams can add them up and name the ones that stayed at zero. A
//! count per value lists every value of a short field and only the values
//! seen of a long one.

use oadec_truehd::Census;
use oadec_truehd::census::Field;
use serde_json::{Map, Value, json};

/// A field with this many values or fewer lists them all, zero or not.
const LISTED_WHOLE: usize = 32;

fn group_json(fields: &[(&'static str, Field<'_>)]) -> Value {
    let mut out = Map::new();
    for (name, field) in fields {
        let value = match *field {
            Field::Count(n) => json!(n),
            Field::Values {
                low,
                counts,
                outside,
            } => {
                let mut values = Map::new();
                for (i, &count) in counts.iter().enumerate() {
                    if count != 0 || counts.len() <= LISTED_WHOLE {
                        values.insert((i64::from(low) + i as i64).to_string(), json!(count));
                    }
                }
                if outside != 0 {
                    values.insert("outside".to_string(), json!(outside));
                }
                Value::Object(values)
            }
        };
        out.insert((*name).to_string(), value);
    }
    Value::Object(out)
}

/// The census as JSON: the major syncs and directories, then the substreams
/// the stream carries.
#[must_use]
pub fn json(census: &Census, substreams: usize) -> Value {
    let carried: Vec<Value> = census
        .substreams
        .iter()
        .take(substreams)
        .map(|s| {
            json!({
                "restart": group_json(&s.restart.fields()),
                "block": group_json(&s.block.fields()),
                "filter": group_json(&s.filter.fields()),
                "entropy": group_json(&s.entropy.fields()),
                "matrix": group_json(&s.matrix.fields()),
            })
        })
        .collect();
    json!({
        "schema": 1,
        "major_sync": group_json(&census.sync.fields()),
        "substreams": carried,
    })
}

/// The counters of one group that are not zero, on one line.
fn group_line(fields: &[(&'static str, Field<'_>)]) -> String {
    let mut parts = Vec::new();
    for (name, field) in fields {
        match *field {
            Field::Count(0) => {}
            Field::Count(n) => parts.push(format!("{name} {n}")),
            Field::Values {
                low,
                counts,
                outside,
            } => {
                let mut seen: Vec<String> = counts
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| **c != 0)
                    .map(|(i, c)| format!("{}: {c}", i64::from(low) + i as i64))
                    .collect();
                if outside != 0 {
                    seen.push(format!("outside: {outside}"));
                }
                if !seen.is_empty() {
                    parts.push(format!("{name} {{{}}}", seen.join(", ")));
                }
            }
        }
    }
    if parts.is_empty() {
        "nothing counted".to_string()
    } else {
        parts.join(", ")
    }
}

/// The census as text: one line a group, the counters that are not zero.
#[must_use]
pub fn lines(census: &Census, substreams: usize) -> Vec<String> {
    let mut out = vec![format!("major sync: {}", group_line(&census.sync.fields()))];
    for (i, s) in census.substreams.iter().take(substreams).enumerate() {
        out.push(format!(
            "substream {i} restart: {}",
            group_line(&s.restart.fields())
        ));
        out.push(format!(
            "substream {i} block: {}",
            group_line(&s.block.fields())
        ));
        out.push(format!(
            "substream {i} filter: {}",
            group_line(&s.filter.fields())
        ));
        out.push(format!(
            "substream {i} entropy: {}",
            group_line(&s.entropy.fields())
        ));
        out.push(format!(
            "substream {i} matrix: {}",
            group_line(&s.matrix.fields())
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_field_lists_every_value_and_a_long_one_the_values_seen() {
        let mut census = Census::default();
        census.substreams[0].filter.fir_order.add(3);
        census.substreams[0].block.block_size.add(40);
        let value = json(&census, 1);
        assert_eq!(value["schema"], 1);
        assert_eq!(value["substreams"].as_array().unwrap().len(), 1);
        let fir = value["substreams"][0]["filter"]["fir_order"]
            .as_object()
            .unwrap();
        assert_eq!(fir.len(), 9, "orders 0 to 8, each one listed");
        assert_eq!((&fir["3"], &fir["0"]), (&json!(1), &json!(0)));
        let sizes = value["substreams"][0]["block"]["block_size"]
            .as_object()
            .unwrap();
        assert_eq!(sizes.len(), 1, "one of 161 block sizes was seen");
        assert_eq!(sizes["40"], 1);
        assert_eq!(value["major_sync"]["major_syncs"], 0);
    }

    #[test]
    fn a_value_below_zero_keeps_its_sign_in_the_report() {
        let mut census = Census::default();
        census.substreams[0].block.output_shift_sent.add(-2);
        census.substreams[0].block.output_shift_sent.add(9);
        let value = json(&census, 1);
        let shifts = &value["substreams"][0]["block"]["output_shift_sent"];
        assert_eq!(shifts["-2"], 1);
        assert_eq!(shifts["outside"], 1);
        let text = lines(&census, 1);
        assert_eq!(text[0], "major sync: nothing counted");
        assert_eq!(
            text[2],
            "substream 0 block: output_shift_sent {-2: 1, outside: 1}"
        );
        assert_eq!(text.len(), 6);
    }
}
