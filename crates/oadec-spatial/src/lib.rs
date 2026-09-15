//! Object-audio program model and DAMF / ADM BWF / WAV / CAF writers.
//!
//! Part of the `oadec` object-audio decoder engine.

pub mod adm;
pub mod caf;
pub mod damf;
pub mod dbmd;
pub mod loss;
pub mod program;

pub use adm::{AdmError, AdmOptions, AdmSummary, AdmWriter, Interpolation};
pub use damf::{DamfError, DamfOptions, DamfWriter};
pub use loss::{LossClass, LossKind, LossLedger};
pub use program::{ElementState, Event, IsfPolicy, Program, ProgramError, Timeline};

/// Saturates a sample to the signed 24-bit range; the flag says whether it
/// had to.
///
/// Every 24-bit writer goes through this one rule: the CAF audio of a DAMF
/// set, the samples of an ADM BWF file, and the PCM and WAVE output of
/// `decode`. The last used to keep the low three bytes instead, which turns a
/// sample one past the top of the range into the bottom of it.
#[must_use]
pub const fn clamp_i24(v: i32) -> (i32, bool) {
    const MIN: i32 = -(1 << 23);
    const MAX: i32 = (1 << 23) - 1;
    if v > MAX {
        (MAX, true)
    } else if v < MIN {
        (MIN, true)
    } else {
        (v, false)
    }
}

/// The FourCC of a WAVE file whose sizes outgrew 32 bits (EBU Tech 3306).
///
/// Named once, here, because it is the one thing a later change may want to
/// swap (for `BW64`, ITU-R BS.2088) without touching the writers.
pub const LONG_FORM_FOURCC: [u8; 4] = *b"RF64";

/// Body length of the `JUNK` chunk a WAVE writer reserves right after `WAVE`:
/// exactly a `ds64` body without a table (`riffSize`, `dataSize` and
/// `sampleCount` as 64-bit fields, then a 32-bit `tableLength`).
pub const DS64_LEN: u32 = 28;

/// Writes the final sizes into the header of a WAVE file laid out as `RIFF`,
/// `WAVE`, a [`DS64_LEN`]-byte `JUNK` chunk and then the other chunks, whose
/// `data` chunk has its 32-bit size at byte `data_size_pos`.
///
/// While the RIFF size (the file less eight bytes) and the `data` size both
/// fit in 32 bits, this patches those two fields and nothing else, so a file
/// under 4 GiB is what it was before this function existed. Past that it
/// promotes the file in place: [`LONG_FORM_FOURCC`] with a RIFF size of
/// `0xFFFFFFFF`, `ds64` over the placeholder carrying the real sizes and
/// `frames` as the sample count, and a `data` size of `0xFFFFFFFF`. Returns
/// whether it promoted. The writer is left inside the header.
pub fn patch_riff_sizes<W: std::io::Write + std::io::Seek>(
    out: &mut W,
    total_len: u64,
    data_size_pos: u64,
    data_len: u64,
    frames: u64,
) -> std::io::Result<bool> {
    use std::io::SeekFrom;
    let riff_size = total_len - 8;
    let long_form = riff_size > u64::from(u32::MAX) || data_len > u64::from(u32::MAX);
    if long_form {
        out.seek(SeekFrom::Start(0))?;
        out.write_all(&LONG_FORM_FOURCC)?;
        out.write_all(&u32::MAX.to_le_bytes())?;
        out.seek(SeekFrom::Start(12))?;
        out.write_all(b"ds64")?;
        out.write_all(&DS64_LEN.to_le_bytes())?;
        out.write_all(&riff_size.to_le_bytes())?;
        out.write_all(&data_len.to_le_bytes())?;
        out.write_all(&frames.to_le_bytes())?;
        out.write_all(&0u32.to_le_bytes())?;
        out.seek(SeekFrom::Start(data_size_pos))?;
        out.write_all(&u32::MAX.to_le_bytes())?;
    } else {
        out.seek(SeekFrom::Start(4))?;
        out.write_all(&(riff_size as u32).to_le_bytes())?;
        out.seek(SeekFrom::Start(data_size_pos))?;
        out.write_all(&(data_len as u32).to_le_bytes())?;
    }
    Ok(long_form)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample inside the 24-bit range passes unchanged; one outside it
    /// saturates at the nearer end and says so.
    #[test]
    fn clamp_i24_saturates_outside_the_range() {
        const MAX: i32 = (1 << 23) - 1;
        const MIN: i32 = -(1 << 23);
        for v in [0, 1, -1, MAX, MIN] {
            assert_eq!(clamp_i24(v), (v, false), "{v}");
        }
        for (v, saturated) in [
            (MAX + 1, MAX),
            (1 << 24, MAX),
            (i32::MAX, MAX),
            (MIN - 1, MIN),
            (-(1 << 24), MIN),
            (i32::MIN, MIN),
        ] {
            assert_eq!(clamp_i24(v), (saturated, true), "{v}");
        }
    }

    /// A WAVE file laid out with the 28-byte `JUNK` placeholder keeps the
    /// RIFF form while every size fits in 32 bits, and becomes RF64 when one
    /// does not: `ds64` takes the placeholder's place with the real sizes and
    /// the 32-bit fields say "look there".
    #[test]
    fn a_wave_file_past_4_gib_is_promoted_in_place() {
        // RIFF/WAVE, JUNK, a 16-byte fmt, data: the ADM writer's layout
        fn header() -> std::io::Cursor<Vec<u8>> {
            let mut h = b"RIFF\0\0\0\0WAVEJUNK".to_vec();
            h.extend_from_slice(&DS64_LEN.to_le_bytes());
            h.extend_from_slice(&[0; DS64_LEN as usize]);
            h.extend_from_slice(b"fmt ");
            h.extend_from_slice(&16u32.to_le_bytes());
            h.extend_from_slice(&[0; 16]);
            h.extend_from_slice(b"data\0\0\0\0");
            std::io::Cursor::new(h)
        }
        let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let u64_at = |b: &[u8], at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        let header_len = header().get_ref().len() as u64;
        let data_size_pos = header_len - 4;

        let mut short = header();
        let long_form = patch_riff_sizes(&mut short, header_len + 1000, data_size_pos, 1000, 250);
        assert!(!long_form.unwrap());
        let b = short.into_inner();
        assert_eq!(b[..4], *b"RIFF");
        assert_eq!(u64::from(u32_at(&b, 4)), header_len + 1000 - 8);
        assert_eq!(b[12..16], *b"JUNK");
        assert_eq!(b[20..48], [0; 28], "the placeholder is left as it was");
        assert_eq!(u32_at(&b, data_size_pos as usize), 1000);

        // five gibibytes of 8-channel 24-bit samples, which no test writes out
        let data: u64 = 5 << 30;
        let total = header_len + data;
        let mut long = header();
        let long_form = patch_riff_sizes(&mut long, total, data_size_pos, data, data / 24);
        assert!(long_form.unwrap());
        let b = long.into_inner();
        assert_eq!(b.len() as u64, header_len, "only the header is rewritten");
        assert_eq!(b[..4], LONG_FORM_FOURCC);
        assert_eq!(u32_at(&b, 4), u32::MAX);
        assert_eq!(b[8..12], *b"WAVE");
        assert_eq!(b[12..16], *b"ds64");
        assert_eq!(u32_at(&b, 16), DS64_LEN);
        assert_eq!(u64_at(&b, 20), total - 8, "riffSize");
        assert_eq!(u64_at(&b, 28), data, "dataSize");
        assert_eq!(u64_at(&b, 36), data / 24, "sampleCount");
        assert_eq!(u32_at(&b, 44), 0, "tableLength");
        assert_eq!(b[48..52], *b"fmt ");
        assert_eq!(u32_at(&b, data_size_pos as usize), u32::MAX);
    }
}
