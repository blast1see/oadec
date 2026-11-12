//! `oadec rules`: the checks `verify` makes, on a TrueHD stream and on a
//! stream of the AC-3 family, with the severity each carries by default and
//! the source each rests on.

// A command that prints as it goes and is not offered as a call.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use anyhow::{Result, anyhow};
use oadec_diag::{Provenance, Rule, RuleInfo};
use serde_json::{Value, json};

/// One rule of either catalog, as the command writes it.
struct Entry {
    info: &'static RuleInfo,
    /// The format whose streams the rule is checked on, as `--json` names it.
    format: &'static str,
    /// Whether a failure is a fault of what a decode delivers, where the
    /// catalog says: the AC-3 family's does.
    delivery: Option<bool>,
}

/// Every rule: the TrueHD catalog, then the catalog of the AC-3 family.
fn entries() -> Vec<Entry> {
    let truehd = oadec_truehd::RuleId::ALL.iter().map(|r| Entry {
        info: r.info(),
        format: "truehd",
        delivery: None,
    });
    let ac3 = oadec_eac3::RuleId::ALL.iter().map(|r| Entry {
        info: Rule::info(*r),
        format: "ac3",
        delivery: Some(r.faults_a_delivery()),
    });
    truehd.chain(ac3).collect()
}

fn provenance_json(p: &Provenance) -> Value {
    match p {
        Provenance::Spec { doc, clause } => json!({"kind": "spec", "doc": doc, "clause": clause}),
        Provenance::Measured { evidence } => json!({"kind": "measured", "evidence": evidence}),
        Provenance::Secondary { source } => json!({"kind": "secondary", "source": source}),
    }
}

/// Runs the command: the whole catalog, or one rule explained.
pub fn run(id: Option<&str>, json: bool, markdown: bool) -> Result<()> {
    if markdown {
        print!(
            "{}{}",
            oadec_truehd::rules::markdown(),
            oadec_eac3::rules::markdown()
        );
        return Ok(());
    }
    let mut rules = entries();
    if let Some(id) = id {
        rules.retain(|e| e.info.id == id);
        if rules.is_empty() {
            return Err(anyhow!("no rule is named {id}; `oadec rules` lists them"));
        }
    }
    if json {
        let list: Vec<Value> = rules
            .iter()
            .map(|e| {
                let i = e.info;
                let mut v = json!({
                    "id": i.id,
                    "format": e.format,
                    "severity": i.severity.name(),
                    "summary": i.summary,
                    "legacy_counter": i.legacy_counter,
                    "provenance": i.provenance.iter().map(provenance_json).collect::<Vec<_>>(),
                });
                if let Some(delivery) = e.delivery {
                    v["delivery"] = json!(delivery);
                }
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else if id.is_some() {
        let e = &rules[0];
        let i = e.info;
        println!("Rule:      {}", i.id);
        println!("Severity:  {} by default", i.severity.name());
        println!("Meaning:   {}", i.summary);
        for (k, p) in i.provenance.iter().enumerate() {
            println!("{}{p}", if k == 0 { "Source:    " } else { "           " });
        }
        if let Some(c) = i.legacy_counter {
            println!("Counter:   failures.{c} in verify --json");
        }
        match e.delivery {
            Some(true) => println!("Delivery:  a fault of what a decode delivers"),
            Some(false) => println!("Delivery:  counted by verify; a decode keeps its verdict"),
            None => {}
        }
        println!("Catalog:   docs/conformance-rules.md");
    } else {
        for e in &rules {
            let i = e.info;
            println!("{:<26} {:<8} {}", i.id, i.severity.name(), i.summary);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rule is named on the command line and in a report by its name
    /// alone, so the two catalogs share none.
    #[test]
    fn the_two_catalogs_share_no_name_and_no_counter() {
        let all = entries();
        assert_eq!(
            all.len(),
            oadec_truehd::RuleId::ALL.len() + oadec_eac3::RuleId::ALL.len()
        );
        let mut names = std::collections::BTreeSet::new();
        for e in &all {
            assert!(names.insert(e.info.id), "{} is in both catalogs", e.info.id);
        }
        // a rule of the AC-3 family says whether it is a fault of a
        // delivery, and a TrueHD rule does not
        assert!(
            all.iter()
                .all(|e| (e.format == "ac3") == e.delivery.is_some())
        );
    }
}
