//! What the metadata containers of a stream declare about their protection.
//!
//! An EMDF container -- the Evolution frame of a TrueHD unit, or a container
//! in an E-AC-3 skip field -- ends with a key identifier and up to two
//! protection words. How the words are computed is not public (ETSI TS 102 366
//! clause H.2.2.4.3 calls it implementation dependent, and so does H.2.2.2.3 of
//! the key), and the key is not public either, so oadec does not check the
//! words; it reports what the containers declare, which is enough to compare
//! streams by it. What is declared is the version, the key identifier and
//! the lengths of the two words; the words themselves are never read here.

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
    /// Containers that declare another key identifier, or another length of
    /// a protection word, than the container before them.
    pub changes: u64,
    /// What the container before declared: its key identifier and the codes
    /// of its two lengths.
    #[serde(skip)]
    last: Option<(u32, u8, u8)>,
}

impl ProtectionStats {
    /// Counts one container, and says whether it declares another key
    /// identifier or another length of a protection word than the container
    /// before it. The first container of a stream has none before it.
    pub fn take(&mut self, c: &Container) -> bool {
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
        let declared = (
            c.key_id,
            c.protection.primary_code,
            c.protection.secondary_code,
        );
        let changed = self.last.is_some_and(|before| before != declared);
        self.last = Some(declared);
        if changed {
            self.changes += 1;
        }
        changed
    }

    /// Containers of a syntax version other than 0, the version of the
    /// syntax they are read by (clause H.2.2.2.2).
    #[must_use]
    pub fn other_versions(&self) -> u64 {
        self.versions
            .iter()
            .filter(|(version, _)| **version != 0)
            .map(|(_, n)| n)
            .sum()
    }

    /// One line for a text report: the key identifiers and word lengths seen.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "key_id {}; primary word bytes {}; secondary {}{}{}",
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
            },
            // said only where there is one, so that the line of every other
            // stream is the line it was
            if self.changes > 0 {
                format!(
                    "; {} declaring another key or length than the one before",
                    self.changes
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

/// A container with no payload, as it stands in a skip field: the sync word
/// and its length, a version, a key identifier, the codes of the lengths of
/// its two protection words, and words of zeroes. `key` is below 7 and
/// `version` below 3, where each field escapes to more bits.
#[cfg(test)]
pub(crate) fn bare_container(version: u8, key: u8, primary: u8, secondary: u8) -> Vec<u8> {
    const WORD_BYTES: [usize; 4] = [0, 1, 4, 16];
    assert!(version < 3 && key < 7 && primary < 4 && secondary < 4);
    // version (2), key (3), the identifier that ends the payloads (5), the
    // two codes (2 + 2): fourteen bits, and the words after them
    let head = u16::from(version) << 14
        | u16::from(key) << 11
        | u16::from(primary) << 4
        | u16::from(secondary) << 2;
    let words = WORD_BYTES[usize::from(primary)] + WORD_BYTES[usize::from(secondary)];
    let len = (14 + 8 * words).div_ceil(8);
    let mut bytes = vec![0x58, 0x38, (len >> 8) as u8, len as u8];
    bytes.extend(head.to_be_bytes());
    bytes.resize(4 + len, 0);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use oadec_emdf::container::parse_emdf_with_sync;

    fn take(stats: &mut ProtectionStats, version: u8, key: u8, primary: u8, secondary: u8) -> bool {
        let bytes = bare_container(version, key, primary, secondary);
        let (container, _) = parse_emdf_with_sync(&bytes).expect("a container");
        assert_eq!(
            (container.version, container.key_id),
            (u32::from(version), u32::from(key))
        );
        stats.take(&container)
    }

    /// A change is a key identifier, or a length of one of the two words,
    /// that the container before did not declare. The first container has
    /// none before it, and a version is not part of what protects one.
    #[test]
    fn a_change_is_a_key_or_a_length_the_container_before_did_not_declare() {
        let mut stats = ProtectionStats::default();
        assert!(!take(&mut stats, 0, 0, 1, 0), "none before the first");
        assert!(!take(&mut stats, 0, 0, 1, 0));
        assert!(take(&mut stats, 0, 5, 1, 0), "another key");
        assert!(!take(&mut stats, 0, 5, 1, 0));
        assert!(take(&mut stats, 0, 5, 2, 0), "another primary length");
        assert!(take(&mut stats, 0, 5, 2, 1), "another secondary length");
        assert!(!take(&mut stats, 1, 5, 2, 1), "another version alone");
        assert!(take(&mut stats, 0, 5, 0, 1), "the reserved code");
        assert_eq!(stats.containers, 8);
        assert_eq!(stats.changes, 4);
        assert_eq!(stats.reserved_primary_code, 1);
        assert_eq!(stats.other_versions(), 1);
        assert_eq!(stats.versions, BTreeMap::from([(0, 7), (1, 1)]));
        assert_eq!(stats.key_ids, BTreeMap::from([(0, 2), (5, 6)]));
        assert_eq!(
            stats.primary_bytes,
            BTreeMap::from([(0, 1), (1, 4), (4, 3)])
        );
        assert_eq!(stats.secondary_bytes, BTreeMap::from([(0, 5), (1, 3)]));
        let said = stats.summary();
        assert!(
            said.ends_with(
                "; 1 with the reserved primary length code; 4 declaring another key or length than the one before"
            ),
            "{said}"
        );
    }

    /// A stream whose containers all declare the same says nothing of a
    /// change: its line is the line it was before changes were counted.
    #[test]
    fn a_stream_that_declares_one_thing_has_the_line_it_had() {
        let mut stats = ProtectionStats::default();
        for _ in 0..3 {
            assert!(!take(&mut stats, 0, 2, 2, 0));
        }
        assert_eq!(
            stats.summary(),
            "key_id 2 x3; primary word bytes 4 x3; secondary 0 x3"
        );
        assert_eq!((stats.changes, stats.other_versions()), (0, 0));
    }
}
