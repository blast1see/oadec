//! `oadec rules`: the checks `verify` makes on a TrueHD stream, with the
//! severity each carries by default and the source each rests on.

// A command that prints as it goes and is not offered as a call.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use anyhow::{Result, anyhow};
use oadec_truehd::{Provenance, RuleId};
use serde_json::{Value, json};

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
        print!("{}", oadec_truehd::rules::markdown());
        return Ok(());
    }
    let rules: Vec<RuleId> = match id {
        Some(id) => vec![
            RuleId::from_id(id)
                .ok_or_else(|| anyhow!("no rule is named {id}; `oadec rules` lists them"))?,
        ],
        None => RuleId::ALL.to_vec(),
    };
    if json {
        let list: Vec<Value> = rules
            .iter()
            .map(|r| {
                let i = r.info();
                json!({
                    "id": i.id,
                    "severity": i.severity.name(),
                    "summary": i.summary,
                    "legacy_counter": i.legacy_counter,
                    "provenance": i.provenance.iter().map(provenance_json).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else if id.is_some() {
        let i = rules[0].info();
        println!("Rule:      {}", i.id);
        println!("Severity:  {} by default", i.severity.name());
        println!("Meaning:   {}", i.summary);
        for (k, p) in i.provenance.iter().enumerate() {
            println!("{}{p}", if k == 0 { "Source:    " } else { "           " });
        }
        if let Some(c) = i.legacy_counter {
            println!("Counter:   failures.{c} in verify --json");
        }
        println!("Catalog:   docs/conformance-rules.md");
    } else {
        for r in &rules {
            let i = r.info();
            println!("{:<26} {:<8} {}", i.id, i.severity.name(), i.summary);
        }
    }
    Ok(())
}
