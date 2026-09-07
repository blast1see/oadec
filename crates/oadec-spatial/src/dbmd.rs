//! The `dbmd` chunk (Dolby audio metadata) of an ADM BWF file.
//!
//! There is no public description of this chunk. The byte layout below was
//! learned from two conversions of oadec DAMF sets by the Dolby encoder's own
//! DAMF-to-ADM converter (21 and 12 channels): a 4-byte version, then segments
//! of `id:8, size:16 (LE), payload, checksum:8` where the checksum makes
//! `sum(payload) + size + checksum == 0 (mod 256)`, closed by a zero id.
//! Segment 7 is constant; segment 9 carries the tool names and, at bytes
//! 135–136, the bed channel mask (LE, bit i = channel i in L R C LFE Lss Rss
//! Lrs Rrs Lfh Rfh Lts Rts Lrh Rrh Lw Rw order); segment 10 carries the
//! channel count, a constant block, one zero byte per channel and one flag
//! byte per channel (0x80 for the LFE, 0x84 for every other channel). Bytes outside those fields are copied
//! from the reference so that the Dolby tools read the chunk as Dolby Atmos.

/// Chunk version.
pub const VERSION: [u8; 4] = [0x06, 0x00, 0x00, 0x01];

const SEGMENT_7: &[&str] = &[
    "0047000000600000242400000000020200000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
];

const SEGMENT_9: &[&str] = &[
    "43726561746564207573696e6720446f6c62792065717569706d656e74000000",
    "446f6c62792041746d6f7320436f6e76657273696f6e20546f6f6c0000000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "02000000000000030005010000000022ff000000000003000000000000000000",
    "000000000000f0ff0c0000030000000000000000000000008400000000000000",
    "0000000000000000f0000800000000000000000000000000f000080000000000",
    "0000000000000000f00008000000000000000000000000000000000000000000",
    "000000000000000000000000000000000000000000000000",
];

const SEGMENT_10_MIDDLE: &[&str] = &[
    "0000010000000000000000000000000000010000000000000000000000000000",
    "0100000000000000000000000000000100000000000000000000000000000100",
    "0000000000000000000000000001000000000000000000000000000001000000",
    "0000000000000000000000010000000000000000000000000000010000000000",
    "000000000000000000",
];

fn decode(parts: &[&str]) -> Vec<u8> {
    let text: String = parts.concat();
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn push_segment(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    let size = payload.len();
    out.push(id);
    out.extend_from_slice(&(size as u16).to_le_bytes());
    out.extend_from_slice(payload);
    let sum = payload.iter().map(|&b| u32::from(b)).sum::<u32>() + size as u32;
    out.push((256 - (sum & 0xFF)) as u8);
}

fn put_string(dst: &mut [u8], text: &str) {
    dst.fill(0);
    let bytes = text.as_bytes();
    let n = bytes.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&bytes[..n]);
}

/// Builds the chunk payload for a program whose bed channels are `bed_mask`
/// (bit order as in the module documentation) and whose audio channels carry
/// `lfe_flags[i] == true` when channel `i` is an LFE.
#[must_use]
pub fn build(bed_mask: u16, lfe_flags: &[bool], creator: &str, tool: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(560);
    out.extend_from_slice(&VERSION);
    push_segment(&mut out, 7, &decode(SEGMENT_7));
    let mut s9 = decode(SEGMENT_9);
    put_string(&mut s9[0..32], creator);
    put_string(&mut s9[32..96], tool);
    s9[135..137].copy_from_slice(&bed_mask.to_le_bytes());
    push_segment(&mut out, 9, &s9);
    let mut s10 = Vec::with_capacity(5 + 137 + 2 * lfe_flags.len());
    s10.extend_from_slice(&[0xBD, 0x6F, 0x72, 0xF8]);
    s10.push(lfe_flags.len() as u8);
    s10.extend_from_slice(&decode(SEGMENT_10_MIDDLE));
    s10.extend(std::iter::repeat_n(0u8, lfe_flags.len()));
    for &lfe in lfe_flags {
        s10.push(if lfe { 0x80 } else { 0x84 });
    }
    push_segment(&mut out, 10, &s10);
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_have_the_reference_sizes_and_valid_checksums() {
        assert_eq!(decode(SEGMENT_7).len(), 96);
        assert_eq!(decode(SEGMENT_9).len(), 248);
        assert_eq!(decode(SEGMENT_10_MIDDLE).len(), 137);
        let flags: Vec<bool> = (0..21).map(|i| i == 3).collect();
        let d = build(0x0CFF, &flags, "Created using oadec", "oadec");
        assert_eq!(d.len(), 4 + (4 + 96) + (4 + 248) + (4 + 184) + 1);
        let mut pos = 4;
        while d[pos] != 0 {
            let size = usize::from(u16::from_le_bytes([d[pos + 1], d[pos + 2]]));
            let payload = &d[pos + 3..pos + 3 + size];
            let sum = payload.iter().map(|&b| u32::from(b)).sum::<u32>()
                + size as u32
                + u32::from(d[pos + 3 + size]);
            assert_eq!(sum & 0xFF, 0, "segment {}", d[pos]);
            pos += 4 + size;
        }
        assert_eq!(&d[4 + 100 + 3 + 135..4 + 100 + 3 + 137], &[0xFF, 0x0C]);
        assert!(d[4 + 100 + 3..].starts_with(b"Created using oadec"));
    }
}
