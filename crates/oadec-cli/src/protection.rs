//! What the metadata containers of a stream declare about their protection.
//!
//! An EMDF container -- the Evolution frame of a TrueHD unit, or a container
//! in an E-AC-3 skip field -- ends with a key identifier and up to two
//! protection words. How the words are computed is not public (ETSI TS 102 366
//! clause H.2.2.4.3 calls it implementation dependent, and so does H.2.2.2.3 of
//! the key), and the key is not public either, so oadec does not check the
//! words; it reports what the containers declare, which is enough to compare
//! streams by it.

use std::collections::BTreeMap;

use oadec_emdf::container::Container;
use serde::Serialize;

/// The protection fields of a stream's containers, counted.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProtectionStats {
    /// Containers counted.
    pub containers: u64,
    /// `key_id` -> containers.
    pub key_ids: BTreeMap<u32, u64>,
    /// Container version -> containers.
    pub versions: BTreeMap<u32, u64>,
    /// Bytes of the primary protection word -> containers.
    pub primary_bytes: BTreeMap<usize, u64>,
    /// Bytes of the secondary protection word -> containers.
    pub secondary_bytes: BTreeMap<usize, u64>,
    /// Containers whose primary length is coded 00, which TS 102 366 reserves.
    pub reserved_primary_code: u64,
}

impl ProtectionStats {
    /// Counts one container.
    pub fn take(&mut self, c: &Container) {
        self.containers += 1;
        *self.key_ids.entry(c.key_id).or_default() += 1;
        *self.versions.entry(c.version).or_default() += 1;
        *self
            .primary_bytes
            .entry(c.protection.primary.len())
            .or_default() += 1;
        *self
            .secondary_bytes
            .entry(c.protection.secondary.len())
            .or_default() += 1;
        if c.protection.primary_code == 0 {
            self.reserved_primary_code += 1;
        }
    }

    /// One line for a text report: the key identifiers and word lengths seen.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "key_id {}; primary word bytes {}; secondary {}{}",
            list(&self.key_ids),
            list(&self.primary_bytes),
            list(&self.secondary_bytes),
            if self.reserved_primary_code > 0 {
                format!(
                    "; {} with the reserved primary length code",
                    self.reserved_primary_code
                )
            } else {
                String::new()
            }
        )
    }
}

fn list<K: core::fmt::Display>(m: &BTreeMap<K, u64>) -> String {
    m.iter()
        .map(|(k, n)| format!("{k} x{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}
