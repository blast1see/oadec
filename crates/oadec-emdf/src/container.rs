//! EMDF containers (ETSI TS 102 366 V1.4.1, Annex H) and the Evolution frames that
//! Dolby TrueHD carries in its access-unit extra data.
//!
//! Both flavours share one syntax: a version, a key id, a list of payloads (each with
//! a five-bit id, a configuration block and a byte payload) terminated by payload id 0,
//! and a protection block. They differ only in how the payload sample offset is coded
//! and in what surrounds the container: an E-AC-3 skip field wraps it in a sync word
//! and a byte length, a TrueHD extra-data block wraps it in a reserved nibble and a
//! byte length. Neither wrapper is parsed here.

use core::fmt;

use oadec_bits::{BitError, BitReader};

/// Which carrier the container came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// E-AC-3 EMDF: `smploffst` is eleven bits followed by one reserved bit.
    Emdf,
    /// TrueHD Evolution frame: `smploffst` is a `variable_bits_max(11, 2)` field.
    Evolution,
}

/// Sync word that precedes an EMDF container inside an E-AC-3 skip field.
pub const EMDF_SYNCWORD: u16 = 0x5838;

/// Payload id of object audio metadata (ETSI TS 103 420 table 55).
pub const PAYLOAD_ID_OAMD: u32 = 11;

/// Payload id of joint object coding side information (ETSI TS 103 420 table 55).
pub const PAYLOAD_ID_JOC: u32 = 14;

/// Errors while reading a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerError {
    /// The container ran past the end of its data.
    Bits(BitError),
    /// The syntax was violated.
    Malformed(&'static str),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bits(e) => write!(f, "container truncated: {e}"),
            Self::Malformed(what) => write!(f, "malformed container: {what}"),
        }
    }
}

impl std::error::Error for ContainerError {}

impl From<BitError> for ContainerError {
    fn from(e: BitError) -> Self {
        Self::Bits(e)
    }
}

/// `emdf_payload_config()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PayloadConfig {
    /// Sample offset of the payload relative to the start of the frame / access unit.
    pub sample_offset: Option<u32>,
    /// The reserved bit after `smploffst` was set (EMDF flavour only).
    pub sample_offset_reserved_set: bool,
    /// Payload duration in samples.
    pub duration: Option<u32>,
    /// Group id.
    pub group_id: Option<u32>,
    /// Codec-specific data (reserved byte).
    pub codec_data: Option<u8>,
    /// `discard_unknown_payload`.
    pub discard_unknown_payload: bool,
    /// `payload_frame_aligned` (only present when no sample offset was given).
    pub payload_frame_aligned: Option<bool>,
    /// `create_duplicate`.
    pub create_duplicate: Option<bool>,
    /// `remove_duplicate`.
    pub remove_duplicate: Option<bool>,
    /// `priority`.
    pub priority: Option<u8>,
    /// `proc_allowed`.
    pub proc_allowed: Option<u8>,
}

/// One payload of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// `emdf_payload_id` (11 = OAMD, 14 = JOC).
    pub id: u32,
    /// Configuration block.
    pub config: PayloadConfig,
    /// Payload bytes.
    pub data: Vec<u8>,
}

/// `emdf_protection()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Protection {
    /// Primary protection word (0, 1, 4 or 16 bytes).
    pub primary: Vec<u8>,
    /// Secondary protection word (0, 1, 4 or 16 bytes).
    pub secondary: Vec<u8>,
    /// Bit offset of the primary word relative to the start of the container.
    pub bits_offset: usize,
}

/// A parsed container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// `emdf_version` / `evo_version`.
    pub version: u32,
    /// `key_id`.
    pub key_id: u32,
    /// Payloads in stream order (the terminating id 0 is not included).
    pub payloads: Vec<Payload>,
    /// Protection block.
    pub protection: Protection,
    /// Number of bits consumed from the reader.
    pub len_bits: usize,
}

impl Container {
    /// Payloads with the given id.
    pub fn payloads_with_id(&self, id: u32) -> impl Iterator<Item = &Payload> {
        self.payloads.iter().filter(move |p| p.id == id)
    }
}

const PROTECTION_SIZES: [usize; 4] = [0, 1, 4, 16];

fn read_variable(reader: &mut BitReader<'_>, n: u32) -> Result<u32, BitError> {
    // Bounded so that a corrupt stream cannot spin: sixteen groups is far beyond any
    // legal value for the two- and three-bit escapes and covers 32 bits for n = 8.
    reader.read_variable_bits_max(n, 16)
}

fn read_config(
    reader: &mut BitReader<'_>,
    flavor: Flavor,
) -> Result<PayloadConfig, ContainerError> {
    let mut config = PayloadConfig::default();

    if reader.read_bool()? {
        match flavor {
            Flavor::Emdf => {
                config.sample_offset = Some(reader.read(11)?);
                config.sample_offset_reserved_set = reader.read_bool()?;
            }
            Flavor::Evolution => {
                config.sample_offset = Some(reader.read_variable_bits_max(11, 2)?);
            }
        }
    }
    if reader.read_bool()? {
        config.duration = Some(read_variable(reader, 11)?);
    }
    if reader.read_bool()? {
        config.group_id = Some(read_variable(reader, 2)?);
    }
    if reader.read_bool()? {
        config.codec_data = Some(reader.read(8)? as u8);
    }
    config.discard_unknown_payload = reader.read_bool()?;
    if !config.discard_unknown_payload {
        let mut frame_aligned = false;
        if config.sample_offset.is_none() {
            frame_aligned = reader.read_bool()?;
            config.payload_frame_aligned = Some(frame_aligned);
            if frame_aligned {
                config.create_duplicate = Some(reader.read_bool()?);
                config.remove_duplicate = Some(reader.read_bool()?);
            }
        }
        if config.sample_offset.is_some() || frame_aligned {
            config.priority = Some(reader.read(5)? as u8);
            config.proc_allowed = Some(reader.read(2)? as u8);
        }
    }
    Ok(config)
}

/// Parses a container starting at the reader position (after any sync/length
/// wrapper). The reader is left after the protection block.
pub fn parse(reader: &mut BitReader<'_>, flavor: Flavor) -> Result<Container, ContainerError> {
    let start = reader.position();

    let mut version = reader.read(2)?;
    if version == 3 {
        version = version.wrapping_add(read_variable(reader, 2)?);
    }
    let mut key_id = reader.read(3)?;
    if key_id == 7 {
        key_id = key_id.wrapping_add(read_variable(reader, 3)?);
    }

    let mut payloads = Vec::new();
    loop {
        let mut id = reader.read(5)?;
        if id == 0 {
            break;
        }
        if id == 0x1F {
            id = id.wrapping_add(read_variable(reader, 5)?);
        }
        let config = read_config(reader, flavor)?;
        let size = read_variable(reader, 8)? as usize;
        if size * 8 > reader.remaining() {
            return Err(ContainerError::Malformed(
                "payload size exceeds the container",
            ));
        }
        let mut data = Vec::with_capacity(size);
        for _ in 0..size {
            data.push(reader.read(8)? as u8);
        }
        payloads.push(Payload { id, config, data });
    }

    let primary_len = PROTECTION_SIZES[reader.read(2)? as usize];
    let secondary_len = PROTECTION_SIZES[reader.read(2)? as usize];
    let bits_offset = reader.position() - start;
    let mut primary = Vec::with_capacity(primary_len);
    for _ in 0..primary_len {
        primary.push(reader.read(8)? as u8);
    }
    let mut secondary = Vec::with_capacity(secondary_len);
    for _ in 0..secondary_len {
        secondary.push(reader.read(8)? as u8);
    }

    Ok(Container {
        version,
        key_id,
        payloads,
        protection: Protection {
            primary,
            secondary,
            bits_offset,
        },
        len_bits: reader.position() - start,
    })
}

/// Parses an Evolution frame (the bytes named by a TrueHD extra-data block).
pub fn parse_evolution(bytes: &[u8]) -> Result<Container, ContainerError> {
    let mut reader = BitReader::new(bytes);
    parse(&mut reader, Flavor::Evolution)
}

/// Parses an EMDF container that starts with its sync word and byte length (the form
/// found in an E-AC-3 skip field). Returns the container and the number of bytes the
/// wrapper declared.
pub fn parse_emdf_with_sync(bytes: &[u8]) -> Result<(Container, usize), ContainerError> {
    let mut reader = BitReader::new(bytes);
    if reader.read(16)? != u32::from(EMDF_SYNCWORD) {
        return Err(ContainerError::Malformed("missing EMDF sync word"));
    }
    let length = reader.read(16)? as usize;
    if length * 8 > reader.remaining() {
        return Err(ContainerError::Malformed(
            "EMDF container length exceeds the data",
        ));
    }
    let container = parse(&mut reader, Flavor::Emdf)?;
    Ok((container, length))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal MSB-first bit writer for building test vectors.
    #[derive(Default)]
    pub(crate) struct BitWriter {
        pub bytes: Vec<u8>,
        pub len: usize,
    }

    impl BitWriter {
        pub fn push(&mut self, n: usize, value: u64) {
            for i in (0..n).rev() {
                if self.len.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                if (value >> i) & 1 == 1 {
                    let last = self.bytes.len() - 1;
                    self.bytes[last] |= 0x80 >> (self.len % 8);
                }
                self.len += 1;
            }
        }

        pub fn push_bytes(&mut self, data: &[u8]) {
            for &b in data {
                self.push(8, u64::from(b));
            }
        }
    }

    fn oamd_like_frame(flavor: Flavor) -> Vec<u8> {
        let mut w = BitWriter::default();
        w.push(2, 0); // version
        w.push(3, 0); // key_id
        // payload 11 with a sample offset of 5 and priority/proc_allowed
        w.push(5, 11);
        w.push(1, 1); // smploffste
        w.push(11, 5);
        w.push(1, 0); // reserved / no more groups
        w.push(1, 0); // duratione
        w.push(1, 0); // groupide
        w.push(1, 0); // codecdatae
        w.push(1, 0); // discard_unknown_payload
        w.push(5, 3); // priority
        w.push(2, 1); // proc_allowed
        w.push(8, 3); // size = 3, no continuation
        w.push(1, 0);
        w.push_bytes(&[0xAA, 0xBB, 0xCC]);
        // payload 14, frame aligned, size 300 needs two variable-bits groups
        w.push(5, 14);
        w.push(1, 0); // no smploffst
        w.push(1, 0);
        w.push(1, 0);
        w.push(1, 0);
        w.push(1, 0); // discard_unknown_payload = 0
        w.push(1, 1); // payload_frame_aligned
        w.push(1, 0); // create_duplicate
        w.push(1, 1); // remove_duplicate
        w.push(5, 0); // priority
        w.push(2, 0); // proc_allowed
        w.push(8, 0); // ((0 + 1) << 8) + 44 = 300
        w.push(1, 1);
        w.push(8, 44);
        w.push(1, 0);
        for i in 0..300u32 {
            w.push(8, u64::from(i & 0xFF));
        }
        w.push(5, 0); // end of payloads
        w.push(2, 1); // primary protection: 1 byte
        w.push(2, 0);
        w.push(8, 0x5A);
        let _ = flavor;
        w.bytes
    }

    #[test]
    fn parses_two_payloads_and_protection() {
        let bytes = oamd_like_frame(Flavor::Evolution);
        let c = parse_evolution(&bytes).unwrap();
        assert_eq!(c.version, 0);
        assert_eq!(c.key_id, 0);
        assert_eq!(c.payloads.len(), 2);
        let p0 = &c.payloads[0];
        assert_eq!(p0.id, PAYLOAD_ID_OAMD);
        assert_eq!(p0.config.sample_offset, Some(5));
        assert_eq!(p0.config.priority, Some(3));
        assert_eq!(p0.config.proc_allowed, Some(1));
        assert_eq!(p0.data, vec![0xAA, 0xBB, 0xCC]);
        let p1 = &c.payloads[1];
        assert_eq!(p1.id, PAYLOAD_ID_JOC);
        assert_eq!(p1.config.payload_frame_aligned, Some(true));
        assert_eq!(p1.config.create_duplicate, Some(false));
        assert_eq!(p1.config.remove_duplicate, Some(true));
        assert_eq!(p1.data.len(), 300);
        assert_eq!(p1.data[299], (299 & 0xFF) as u8);
        assert_eq!(c.protection.primary, vec![0x5A]);
        assert!(c.protection.secondary.is_empty());
        assert_eq!(c.payloads_with_id(PAYLOAD_ID_JOC).count(), 1);
    }

    #[test]
    fn emdf_flavour_reads_the_reserved_bit_and_the_sync_wrapper() {
        let mut w = BitWriter::default();
        w.push(16, u64::from(EMDF_SYNCWORD));
        let body = {
            let mut b = BitWriter::default();
            b.push(2, 0);
            b.push(3, 0);
            b.push(5, 11);
            b.push(1, 1);
            b.push(11, 1023);
            b.push(1, 1); // reserved bit set (EMDF) — must be reported, not consumed as a group
            b.push(1, 0);
            b.push(1, 0);
            b.push(1, 0);
            b.push(1, 1); // discard_unknown_payload
            b.push(8, 1);
            b.push(1, 0);
            b.push(8, 0xEE);
            b.push(5, 0);
            b.push(4, 0);
            while b.len % 8 != 0 {
                b.push(1, 0);
            }
            b.bytes
        };
        w.push(16, body.len() as u64);
        w.push_bytes(&body);
        let (c, len) = parse_emdf_with_sync(&w.bytes).unwrap();
        assert_eq!(len, body.len());
        assert_eq!(c.payloads[0].config.sample_offset, Some(1023));
        assert!(c.payloads[0].config.sample_offset_reserved_set);
        assert!(c.payloads[0].config.discard_unknown_payload);
        assert_eq!(c.payloads[0].data, vec![0xEE]);
    }

    #[test]
    fn oversized_payload_is_rejected_not_panicked() {
        let mut w = BitWriter::default();
        w.push(2, 0);
        w.push(3, 0);
        w.push(5, 11);
        w.push(4, 0); // no sample offset, duration, group id or codec data
        w.push(1, 1); // discard_unknown_payload: nothing else follows in the config
        w.push(8, 200); // size 200 but no bytes follow
        w.push(1, 0);
        assert_eq!(
            parse_evolution(&w.bytes),
            Err(ContainerError::Malformed(
                "payload size exceeds the container"
            ))
        );
        assert!(parse_evolution(&[]).is_err());
    }
}
