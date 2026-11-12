//! What the commands say of the same bytes, and what has to hold between
//! them.
//!
//! A stream goes to every command that reads one, through the calls of
//! `oadec_cli::api`, and what each made of it is put side by side. The
//! policy of the exit codes says how the answers hang together
//! (`docs/exit-codes.md`): every command that delivers audio or metadata
//! decides between clean and faulty with the list of faults `verify` uses.
//! [`check`] is that sentence, clause by clause.
//!
//! The commands had it wrong before, each time in one stream nobody had put
//! to two of them: `verify` exited 7 where `decode` exited 0 on the same CRC
//! failure, and the object outputs read no integrity flag at all. Changing
//! bytes at random and asking every command is how the next one is found
//! without waiting for the stream that has it.

use std::path::Path;

use oadec_cli::api::{self, ObjectOptions, PcmBytes, PcmOptions, Source, Verdict, VerifyOptions};

use crate::mutate::{Rng, mutate};
use crate::seeds::fnv1a;

/// What one command made of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Said {
    /// It refused the input: exit code 2.
    Refused,
    /// Exit code 0.
    Clean,
    /// Exit code 4.
    Lossy,
    /// Exit code 7.
    Faulty,
}

impl Said {
    fn of_clean(clean: bool) -> Self {
        if clean { Self::Clean } else { Self::Faulty }
    }

    fn of_verdict(verdict: Verdict) -> Self {
        match verdict {
            Verdict::Clean => Self::Clean,
            Verdict::Lossy => Self::Lossy,
            Verdict::Faulty => Self::Faulty,
        }
    }

    /// One letter: `-` refused, `c` clean, `l` lossy, `F` faulty.
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Self::Refused => '-',
            Self::Clean => 'c',
            Self::Lossy => 'l',
            Self::Faulty => 'F',
        }
    }
}

/// What every command made of one stream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Survey {
    /// Whether the stream is of the AC-3 family.
    pub ac3: bool,
    /// `info`.
    pub info: Said,
    /// `verify`.
    pub verify: Said,
    /// Whether what `verify` found is a fault of a delivery too. Three things
    /// it counts in an AC-3 family stream are not (`docs/exit-codes.md`): a
    /// frame that ends inside its own tail, an EMDF payload configuration
    /// outside the table, and a complexity index that disagrees with the
    /// object total. A decode of such a stream keeps its verdict.
    pub delivery_fault: bool,
    /// `verify --decode`.
    pub verify_decode: Said,
    /// `oamd`.
    pub oamd: Said,
    /// `emdf`.
    pub emdf: Said,
    /// `decode --format pcm`: presentations 0 to 3 of a TrueHD stream, or
    /// the programme and the core of an AC-3 family one.
    pub pcm: Vec<Said>,
    /// A digest of the samples each of those decodes wrote; none where it
    /// refused.
    pub samples: Vec<Option<u64>>,
    /// `decode --format damf` and `adm`, where a directory to write to was
    /// given.
    pub objects: Vec<Said>,
}

impl Survey {
    /// The survey in a line, for a tally.
    #[must_use]
    pub fn line(&self) -> String {
        let letters = |said: &[Said]| said.iter().map(|s| s.letter()).collect::<String>();
        format!(
            "{} info {} verify {} decode {} oamd {} emdf {} pcm {} objects {}",
            if self.ac3 { "ac3" } else { "thd" },
            self.info.letter(),
            self.verify.letter(),
            self.verify_decode.letter(),
            self.oamd.letter(),
            self.emdf.letter(),
            letters(&self.pcm),
            letters(&self.objects),
        )
    }

    /// What the commands that write an output said, samples first.
    pub fn deliveries(&self) -> impl Iterator<Item = Said> + '_ {
        self.pcm.iter().chain(&self.objects).copied()
    }

    /// Whether every report called the stream clean and every delivery that
    /// was made is clean.
    #[must_use]
    pub fn all_clean(&self) -> bool {
        [
            self.info,
            self.verify,
            self.verify_decode,
            self.oamd,
            self.emdf,
        ]
        .into_iter()
        .all(|said| said == Said::Clean)
            && self
                .deliveries()
                .all(|said| matches!(said, Said::Clean | Said::Refused))
    }
}

/// Puts the checksums of `stream` right, so that a parser takes a change
/// under one at its word: the access units of a TrueHD or MLP stream, or the
/// syncframes of an AC-3 family one.
pub fn repair(stream: &mut [u8]) {
    if stream.starts_with(&[0x0b, 0x77]) {
        let mut at = 0;
        while at + 8 <= stream.len() {
            let Ok(header) = oadec_eac3::FrameHeader::parse(&stream[at..]) else {
                at += 1;
                continue;
            };
            let Some(frame) = stream.get_mut(at..at + header.frame_bytes) else {
                break;
            };
            oadec_eac3::repair::fix_crcs(frame);
            at += header.frame_bytes.max(1);
        }
    } else {
        oadec_truehd::repair_stream(stream);
    }
}

/// The stream of one round of a campaign over `streams`: one of them,
/// changed, and in three rounds of four with its checksums put right, as
/// the fourth is left the way a medium would leave it. Which stream it was
/// comes back with it. The same numbers give the same bytes on every
/// machine.
#[must_use]
pub fn round(streams: &[Vec<u8>], seed: u64, round: u64) -> (usize, Vec<u8>) {
    let mut rng = Rng::new(0x7061_7269_7479 ^ seed.rotate_left(32) ^ round);
    let from = rng.below(streams.len());
    let donor = &streams[rng.below(streams.len())];
    let mut bytes = streams[from].clone();
    mutate(&mut bytes, donor, &mut rng);
    if rng.below(4) != 0 {
        repair(&mut bytes);
    }
    (from, bytes)
}

/// Asks every command about `stream`. With `scratch`, the object outputs are
/// written under it and asked too.
#[must_use]
pub fn survey(stream: &[u8], scratch: Option<&Path>) -> Survey {
    let source = Source::memory("a stream", stream.to_vec());
    let ac3 = api::is_ac3_family(&source).unwrap_or(false);
    let info = api::info(&source).map_or(Said::Refused, |_| Said::Clean);
    let (verify, delivery_fault) = match api::verify(&source, &VerifyOptions::default()) {
        Err(_) => (Said::Refused, false),
        Ok(report) if report.clean() => (Said::Clean, false),
        Ok(report) => (Said::Faulty, !ac3 || faults_a_delivery(&report)),
    };
    let options = VerifyOptions {
        decode: true,
        ..VerifyOptions::default()
    };
    let verify_decode = api::verify(&source, &options)
        .map_or(Said::Refused, |report| Said::of_clean(report.clean()));
    let oamd = api::oamd(&source).map_or(Said::Refused, |report| Said::of_clean(report.clean()));
    let emdf = api::emdf(&source).map_or(Said::Refused, |report| Said::of_clean(report.clean()));
    let decodes: Vec<PcmOptions> = if ac3 {
        // dither off, so that the samples are a function of the stream
        [false, true]
            .into_iter()
            .map(|core_only| PcmOptions {
                core_only,
                dither: false,
                ..PcmOptions::default()
            })
            .collect()
    } else {
        (0..4)
            .map(|presentation| PcmOptions {
                presentation: Some(presentation),
                ..PcmOptions::default()
            })
            .collect()
    };
    let mut pcm = Vec::new();
    let mut samples = Vec::new();
    for options in &decodes {
        let mut sink = PcmBytes::default();
        let said = api::decode_pcm(&source, options, &mut sink, &mut std::io::sink())
            .and_then(|pcm| pcm.findings())
            .map_or(Said::Refused, |findings| {
                Said::of_verdict(findings.verdict())
            });
        samples.push((said != Said::Refused).then(|| fnv1a(&sink.bytes)));
        pcm.push(said);
    }
    let objects = scratch.map_or_else(Vec::new, |dir| {
        [false, true]
            .into_iter()
            .map(|adm| {
                let options = ObjectOptions {
                    adm,
                    ..ObjectOptions::default()
                };
                api::decode_objects(&source, &dir.join("scene"), &options, &mut std::io::sink())
                    .and_then(|written| written.findings())
                    .map_or(Said::Refused, |findings| {
                        Said::of_verdict(findings.verdict())
                    })
            })
            .collect()
    });
    Survey {
        ac3,
        info,
        verify,
        delivery_fault,
        verify_decode,
        oamd,
        emdf,
        pcm,
        samples,
        objects,
    }
}

/// The counters of `verify` on an AC-3 family stream that are no fault of a
/// delivery, by the names its report gives them.
const NOT_OF_A_DELIVERY: [&str; 4] = [
    "tail_overruns",
    "substream_tail_overruns",
    "payload_config_violations",
    "complexity_mismatches",
];

/// Whether a report of `verify` on an AC-3 family stream that is not clean
/// counts anything that is a fault of a delivery too. Every counter its
/// verdict rests on is among the failures of the report. A report without
/// its counters, of a stream no frame of which decoded, does.
fn faults_a_delivery(report: &api::Verified) -> bool {
    let Ok(Some(json)) = report.json() else {
        return true;
    };
    let Some(failures) = json["failures"].as_object() else {
        return true;
    };
    failures.iter().any(|(name, count)| {
        !NOT_OF_A_DELIVERY.contains(&name.as_str()) && count.as_u64() != Some(0)
    })
}

/// What has to hold between the answers of a [`Survey`], whatever the
/// stream. The first clause that does not hold is the error.
///
/// # Errors
/// When two commands disagree where the policy of the exit codes says they
/// agree.
pub fn check(s: &Survey) -> Result<(), String> {
    let reports = [
        ("info", s.info),
        ("verify", s.verify),
        ("verify --decode", s.verify_decode),
        ("oamd", s.oamd),
        ("emdf", s.emdf),
    ];
    let fail = |clause: &str| Err(format!("{clause}: {}", s.line()));

    // What holds no stream holds none for any command: the reports refuse it
    // together, and nothing is delivered from it.
    let refusing = reports
        .iter()
        .filter(|(_, said)| *said == Said::Refused)
        .count();
    if refusing != 0 {
        if refusing != reports.len() {
            return fail("one report finds a stream where another finds none");
        }
        if s.deliveries().any(|said| said != Said::Refused) {
            return fail("a delivery from what no report calls a stream");
        }
        return Ok(());
    }

    // A fault `verify` finds is a fault to every command that takes its
    // verdict: the decode of every presentation beside its own pass, the two
    // metadata walks beside theirs, and every delivery, which is faulty or
    // was not made. The three things the policy keeps out of the verdict of a
    // delivery are the exception, and for a delivery alone.
    if s.verify == Said::Faulty {
        if s.verify_decode != Said::Faulty {
            return fail("verify --decode clears what verify found");
        }
        if s.oamd != Said::Faulty || s.emdf != Said::Faulty {
            return fail("a metadata report clears what verify found");
        }
        if s.delivery_fault
            && s.deliveries()
                .any(|said| !matches!(said, Said::Faulty | Said::Refused))
        {
            return fail("a delivery clears what verify found");
        }
    }

    // A fault of a delivery is one `verify --decode` finds: a decode says
    // that `verify` reports the same faults, and it does.
    if s.deliveries().any(|said| said == Said::Faulty) && s.verify_decode != Said::Faulty {
        return fail("a delivery is faulty and verify --decode is clean");
    }

    // `--decode` adds to `verify` what a decode shows and nothing else: where
    // it turns a clean stream faulty, a decode of some presentation is faulty,
    // or stopped before its first sample, which `decode` refuses and
    // `verify --decode` counts.
    if s.verify == Said::Clean
        && s.verify_decode == Said::Faulty
        && s.pcm.iter().all(|said| *said == Said::Clean)
    {
        return fail("verify --decode finds a fault no decode has");
    }

    // A declared loss is of the object outputs alone.
    if [s.info, s.verify, s.verify_decode, s.oamd, s.emdf]
        .into_iter()
        .chain(s.pcm.iter().copied())
        .any(|said| said == Said::Lossy)
    {
        return fail("a declared loss where no object output is");
    }
    Ok(())
}

/// A stream through every command that reads one, with its checksums put
/// right first so that a change under one reaches the parsers. Whatever the
/// bytes, the commands answer without a panic and their answers hang
/// together as [`check`] has it. Nothing is written: the object outputs are
/// files, and are asked in the seeded campaign.
pub fn verdict_parity(data: &[u8]) {
    let mut stream = data.to_vec();
    repair(&mut stream);
    if let Err(why) = check(&survey(&stream, None)) {
        panic!("{why}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn survey_of(reports: [Said; 5], pcm: &[Said], objects: &[Said]) -> Survey {
        Survey {
            ac3: false,
            info: reports[0],
            verify: reports[1],
            delivery_fault: reports[1] == Said::Faulty,
            verify_decode: reports[2],
            oamd: reports[3],
            emdf: reports[4],
            pcm: pcm.to_vec(),
            samples: vec![None; pcm.len()],
            objects: objects.to_vec(),
        }
    }

    use Said::{Clean as C, Faulty as F, Lossy as L, Refused as R};

    /// What the commands have been seen to say of changed streams, and may.
    #[test]
    fn answers_that_hang_together_pass() {
        for (reports, pcm, objects) in [
            // clean, and a presentation the stream does not carry
            ([C, C, C, C, C], &[C, C, C, R][..], &[R, R][..]),
            ([C, C, C, C, C], &[C, C, C, C], &[C, C]),
            // a fault every command sees
            ([C, F, F, F, F], &[F, F, F, R], &[R, R]),
            ([C, F, F, F, F], &[F, F, F, F], &[F, F]),
            // a fault no decode gets past
            ([C, F, F, F, F], &[R, R, R, R], &[R, R]),
            // no stream
            ([R, R, R, R, R], &[R, R, R, R], &[R, R]),
            // a lossless check that fails: only a decode sees it
            ([C, C, F, C, C], &[C, F, F, R], &[R, R]),
            // a presentation the stream declares and cannot start
            ([C, C, F, C, C], &[C, R, R, R], &[R, R]),
            ([C, C, F, C, C], &[C, C, C, F], &[F, F]),
            // the metadata walks are stricter on what they read
            ([C, C, C, F, C], &[C, C, C, C], &[C, C]),
            // an object output that refuses, or declares a loss
            ([C, C, C, C, C], &[C, C, C, C], &[R, R]),
            ([C, C, C, C, C], &[C, C, C, C], &[L, R]),
            ([C, F, F, F, F], &[F, F, F, F], &[F, R]),
        ] {
            let s = survey_of(reports, pcm, objects);
            assert_eq!(check(&s), Ok(()), "{}", s.line());
        }
        // what verify counts and the policy keeps out of a delivery's verdict
        let mut s = survey_of([C, F, F, F, F], &[C, C], &[C, C]);
        s.delivery_fault = false;
        assert_eq!(check(&s), Ok(()), "{}", s.line());
    }

    /// Each clause, by an answer that breaks it and no other.
    #[test]
    fn each_clause_refuses_what_breaks_it() {
        for (reports, pcm, objects, clause) in [
            (
                [C, R, R, R, R],
                &[R, R, R, R][..],
                &[R, R][..],
                "one report finds a stream where another finds none",
            ),
            (
                [R, R, R, R, R],
                &[C, R, R, R],
                &[R, R],
                "a delivery from what no report calls a stream",
            ),
            (
                [C, F, C, F, F],
                &[F, F, F, F],
                &[F, F],
                "verify --decode clears what verify found",
            ),
            (
                [C, F, F, C, F],
                &[F, F, F, F],
                &[F, F],
                "a metadata report clears what verify found",
            ),
            (
                [C, F, F, F, C],
                &[F, F, F, F],
                &[F, F],
                "a metadata report clears what verify found",
            ),
            (
                [C, F, F, F, F],
                &[F, C, F, F],
                &[F, F],
                "a delivery clears what verify found",
            ),
            (
                [C, F, F, F, F],
                &[F, F, F, F],
                &[F, L],
                "a delivery clears what verify found",
            ),
            (
                [C, C, C, C, C],
                &[C, C, C, C],
                &[F, F],
                "a delivery is faulty and verify --decode is clean",
            ),
            (
                [C, C, C, C, C],
                &[C, F, C, C],
                &[C, C],
                "a delivery is faulty and verify --decode is clean",
            ),
            (
                [C, C, F, C, C],
                &[C, C, C, C],
                &[R, R],
                "verify --decode finds a fault no decode has",
            ),
            (
                [C, C, C, C, C],
                &[C, L, C, C],
                &[C, C],
                "a declared loss where no object output is",
            ),
        ] {
            let s = survey_of(reports, pcm, objects);
            let said = check(&s).expect_err(&s.line());
            assert!(said.starts_with(clause), "{said}");
        }
    }
}
