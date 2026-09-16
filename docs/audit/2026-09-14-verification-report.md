# Verification round, 2026-09-14

The owner asked for four things: check that the problems the audits recorded are
really fixed, by testing rather than by reading; check that every function works;
find and fix what else is wrong; and measure whether the decoder really comes close
to Dolby. This report is what was run, what it showed, what changed, and what is
still open. Every number comes from a stored measurement under
`evidence/verification-2026-09-14/`.

The binary measured is the v0.3.0 tree (`c142b8f`, sha256 in
`00-provenance.json`). Fixes were written on branches from it, each with a test that
fails on the old code, and merged into `verification-2026-09-14`. The head of that
branch was measured again with every gate (§2) and the command matrix (§3), and put
through a review loop by a second model (§5).

## 1. Verdict

| Question | Answer |
|---|---|
| Are the recorded defects fixed? | Yes. Every gate the audits left behind passes with the media: the 22 media tests (27 at the head of the branch), the three TrueHD bit-exactness gates, the six-title JOC object gate, all four corruption campaigns (0 silent, 0 panics), and the ADM remediation stages (harness 27/27, Dolby and renderer stages, 50 of 51 decodes byte-identical and the one expected difference verified). |
| Does every function work? | Yes, after five fixes. 212 smoke cases over the 12 subcommands: five real defects (empty or random input exiting 0 from `info`, `oamd` and `decode`), everything else as designed once the smoke expectations were corrected. At the head of the branch the only unexpected cases are the designed ones (§3). |
| What else was wrong? | Twenty-six defects, all fixed: six from a second model's adversarial review of the released code, eight the audits had left open, two from the command matrix, four found while fixing the others, two in the project's own measuring tools, and four from the review loop over the finished branch. Every decoder fix comes with a test that fails on the old code (§5, §6, §7). |
| Is it close to Dolby? | TrueHD: exact. On sixteen titles nothing was fitted on, presentation 3 is byte-identical to `truehdd` on all sixteen and bit-exact with Dolby on every title Dolby opens at a dialogue norm of −31 dB; on the others Dolby applies its dialnorm gain and triangular dither, and nothing else differs. E-AC-3 JOC: worst object of a title 29.6–50.5 dB, median 39.6–65.4 dB over twelve titles, lag zero, correlation structure within 0.0021. The remainder is the E-AC-3 dither on some titles and the lowest 375 Hz subband on others, where reordering the correction and the matrix only makes it worse. |

## 2. What was re-run

`01-ci-and-media-suite.json`, `02-regression-gates.json`,
`03-corruption-replays.json`, `10-adm-remediation-stages.json`.

| Check | Result |
|---|---|
| `cargo fmt --check`, `clippy -D warnings`, `test` (debug and release), release build, `doc` | exit 0 |
| Media suite without `OADEC_MEDIA` | exit 101: it refuses to pass, as CI requires |
| Media suite with the work directory | 22 of 22 passed in 790 s |
| `regression_gates.sh` | presentation 2 vs FFmpeg, presentation 3 vs `truehdd`, presentation 3 vs Dolby: PASS, PASS, PASS |
| `joc_object_gate.py` | 6 titles measured, 0 regressed; worst object 35.25 dB (Disclosure), lag 0, correlation-structure delta ≤ 0.0014 |
| JOC corruption replay (seed 1, 120 flips) | verify, PCM decode and DAMF decode: 120 non-zero exits, 0 silent, 0 panics |
| TrueHD corruption replay (seed 7, 100 flips) | 100 non-zero, 0 silent, 0 panics |
| DD+ 7.1 corruption replay (150 flips) | programme and `--core-only`: 150 non-zero, 0 silent, 0 panics |
| Dependent-substream `bsi` flips with the CRC repaired (200) | 159 reported; 41 silent and every one of them left the output unchanged; 0 silently corrupted; 0 panics |
| ADM remediation, harness stage | 27 of 27 expectations |
| ADM remediation, Dolby and renderer stages | every expectation met |
| ADM remediation, regression stage | crashed on a fresh work directory and could not pass after 0.3.0; fixed (§7); then 51 records, 50 identical (48 outputs up to the version string), 1 expected difference verified, 0 unexplained, 0 exit codes changed |
| ADM audit toolkit self-tests | 136 passed |

### 2.1 The same checks at the head of the branch

Each file above has a `branch_head` section. Everything was run again with the binary
built at `3418cdf`, the branch with its fixes and before the changes of the review loop
(§5), from a detached worktree so that nothing edited during the review could reach it.

| Check | v0.3.0 | Branch head |
|---|---|---|
| `fmt`, `clippy -D warnings`, workspace tests (debug and release), release build, `doc` | exit 0 | exit 0; 261 tests passed, 0 failed |
| `cargo deny`, `cargo audit` | not run | exit 0 |
| Media suite without `OADEC_MEDIA` | exit 101 | exit 101 |
| Media suite with the work directory | 22 of 22 | 27 of 27 in 1 264 s; the five new tests cover the configuration change, `verify --decode`, AC-3 framing in `emdf` and the two programme decoder stalls |
| TrueHD gates | PASS, PASS, PASS | PASS, PASS, PASS |
| JOC object gate | 6 measured, 0 regressed, worst object 35.25 dB | the same six results, value for value |
| Corruption replays: JOC 120, TrueHD 100, DD+ 7.1 150 | 0 silent, 0 panics | 0 silent, 0 panics |
| Dependent-substream `bsi` flips, 200 | 41 silent with the output unchanged, 0 corrupted | the same |
| ADM stages | harness 27/27; Dolby and renderer met; regression 50 of 51 identical, 1 expected difference | the same |
| ADM toolkit self-tests | 136 passed | 136 passed |

CI on the pull request passed every job: the tests on Windows and Ubuntu, the media suite's
refusal without media, the ADM toolkit and `cargo deny`.

At `1a7e10c`, the head after the changes of the review loop (§5), the whole media suite
passed again, 27 of 27 in 1 235 s, and the command matrix gave the same exit code as at
`3418cdf` in all 212 cases. CI passed every job on each pushed commit.

At `b00f490`, the head after round 4 of the review loop, W4 and G4, the 273 workspace
tests passed in release, the whole media suite passed again, 27 of 27 in 1240 s, and the
command matrix ran its 212 cases with the same 51 designed unexpected ones. 13 cases
changed their exit code against `1a7e10c`, every one of them `emdf` or `oamd` on a clip
cut inside a frame or an access unit, from 0 to 7, which is G3.

## 3. Function by function

`04-function-smoke-matrix.json`. 212 cases: every subcommand's `--help`, `info`,
`verify` and `--json` on 22 clips, every `decode` format against every presentation
and the measurement flags, `compare`, `thd-demux`, the three rewriting tools,
`atmos-author`, the Dolby validators on every DAMF and ADM output, and empty,
random, missing and unwritable paths. 56 cases did not give the exit code the
matrix expected:

| Cases | Why | Verdict |
|---:|---|---|
| 46 | `verify` and `decode` of head cuts exit 7: a cut TrueHD head ends inside an access unit ("84 trailing bytes") and a cut E-AC-3 head starts inside a frame ("1728 bytes skipped") | as designed; the expectation was wrong |
| 4 | `atmos_info` 1.1 has no `--validate` option | tool syntax; re-run: all seven DAMF sets pass 1.1 and 5.7.2, the three tagged ADM files pass `bwf_info` and both `atmos_info`, the untagged ones are refused as documented |
| 1 | `eac3-ecpl-inject` on a 768 kbit/s stream: "no frame used coupling" | a correct refusal; on a 384 kbit/s stream it converts 3 305 of 3 305 frames, `verify` is clean and Dolby's decoder plays it |
| 4 | `info` and `oamd` exit 0 on an empty file and on 1 MB of random bytes | **defect**, fixed (§6) |
| 1 | `decode` exits 0 on an empty file ("nothing decoded") | **defect**, fixed (§6) |

Two readings of `compare` looked alarming and are not: against the whole-film
`truehdd` reference the head decode has 0 mismatching samples and reports DIFFERENT
only because the reference is longer; and against the stored FFmpeg clips the
surround channels of two AC-3/E-AC-3 heads show 6–8 dB because they sit at
−110 dBFS, digital near-silence where only the dither is left, while the one burst
on R near 10.15 s is the FFmpeg block-switching deviation recorded on 2026-09-08.

The same 212 cases with the binary built at `3418cdf` (`branch_head` in the same file):
51 did not give the expected exit code, and they are the 46 head cuts, the 4 `atmos_info`
1.1 syntax cases and the coupling refusal above, nothing else. Ten cases changed their exit
code, each by a fix of §6:

| Cases | v0.3.0 | Branch head | Fix |
|---|---:|---:|---|
| `info` and `oamd` on the empty and the random file | 0 | 2 | S1 |
| `decode` on the empty file | 0 | 2 | S2 |
| `decode` on the random file | 7 | 2 | S2: nothing framed, so nothing to judge |
| `emdf`, `emdf --json` and `emdf --dump` on the configuration 4 head | 7 | 0 | W1: its AC-3 core was read as E-AC-3 |
| `oamd` on an E-AC-3 stream | 2 | 0 | A5 |

## 4. Closeness to Dolby on material nothing was fitted on

Earlier object-domain numbers came from clips the matrix alignment and the
quadrature filter were fitted on. The titles below appear in no earlier evidence
file and no clip: they were picked from the library by name, from UHD remuxes and
streaming releases.

### 4.1 TrueHD, sixteen titles

`05-truehd-fresh-sample.json`. A 90-second head cut of each; presentation 3 against
Dolby (`dlbtruehddec presentation=16 out-ch-config=21` behind an MP4 wrapper) and
`truehdd`, presentation 2 against FFmpeg.

| | Titles |
|---|---:|
| byte-identical to `truehdd` (presentation 3) | 16 of 16 |
| bit-exact with FFmpeg (presentation 2) | 16 of 16 |
| opened by Dolby's object mode | 15 of 16 (Up, 2009, refused — the known 46 % class) |
| bit-exact with Dolby | 10 of 15: every one at a dialogue norm of −31 dB |
| not bit-exact with Dolby | 5 of 15: every one at −27 or −28 dB |

### 4.2 Dolby's object mode applies dialogue normalisation, with dither

`06-truehd-dolby-dialnorm.json`. On the five titles that differ, Dolby's 32-bit
words against oadec's samples:

| | Prey | Evil Dead Rise | Captain America: BNW |
|---|---:|---:|---:|
| dialogue norm | −27 | −28 | −27 |
| fitted gain minus nominal dialnorm gain | 3.1·10⁻⁵ dB | 1.5·10⁻⁵ dB | 3.1·10⁻⁵ dB |
| residual after the gain, std in 24-bit LSB | 0.408 | 0.408 | 0.408 |
| correlation of the residual with the signal | −8·10⁻¹¹ | −3·10⁻⁹ | 2·10⁻⁸ |
| Dolby non-zero where the coded audio is silent | 99.6 % | 99.6 % | 99.6 % |

0.408 is 1/√6, the standard deviation of triangular dither spanning ±1 LSB. So
Dolby's object output is the lossless decode, times the dialnorm gain, plus TPDF
dither, kept at 32-bit precision; `drc-mode` has no setting that turns it off.
oadec delivers the coded samples, which is what a master needs. The earlier
"bit-exact with Dolby" was true and incomplete: both titles it rested on carry
−31 dB. `docs/dolby-tools.md` now says so.

### 4.3 E-AC-3 JOC objects

`07-joc-fresh-sample.json`, `02-regression-gates.json`. Fresh titles: a 30-second cut
from ten minutes in; Dolby's objects from `dlbac3dec out-ch-config=21`
(`drc-suppress`, `custom-0`, `drop-delay`); `tools/objcmp.py`. All twelve are
downmix configuration 3 with fifteen objects.

| Title | Source | worst object | median | lag | correlation delta |
|---|---|---:|---:|---:|---:|
| Mercy (2026) | streaming, fresh | 29.55 | 40.57 | 0 | 0.0021 |
| A Man Called Otto (2022) | streaming, fresh | 30.33 | 44.25 | 0 | 0.0015 |
| Lion of the Desert (1980) | UHD remux, fresh | 35.31 | 40.09 | 0 | 0.0001 |
| Kar Kardeşliği (2024) | streaming, fresh | 39.29 | 53.64 | 0 | 0.0001 |
| Damsel (2024) | streaming, fresh | 44.38 | 51.05 | 0 | 0.0005 |
| Dexter: Resurrection S01E01 | streaming, fresh | 49.53 | 53.00 | 0 | 0.0002 |
| Disclosure | gate | 35.25 | 40.24 | 0 | 0.0010 |
| Kingsman | gate | 35.54 | 39.59 | 0 | 0.0001 |
| Knives Out | gate | 35.72 | 41.23 | 0 | 0.0007 |
| Blade Runner 2049 | gate | 37.63 | 42.08 | 0 | 0.0009 |
| Glass Onion | gate | 49.93 | 65.44 | 0 | 0.0001 |
| Extraction | gate | 50.51 | 53.51 | 0 | 0.0014 |

The fresh titles land in the range the gate titles span; nothing about the fitted
constants shows up as a worse fit on unseen material.

### 4.4 Where the remainder lives

`07-joc-fresh-sample.json` (`dither_check`, `low_band`, `residual_bands`).
Decoding with `--no-dither` removes our dither and leaves Dolby's; if the residual
is the two dithers, every object gains 3 dB.

| Title | worst → without our dither | median → without | residual in subband 0 (median share) |
|---|---|---|---:|
| Lion of the Desert | 35.31 → 38.34 | 40.09 → 43.07 | 0 % |
| Kar Kardeşliği | 39.29 → 42.20 | 53.64 → 56.63 | 1 % |
| Mercy | 29.55 → 29.56 | 40.57 → 41.12 | 81 % |
| A Man Called Otto | 30.33 → 30.34 | 44.25 → 45.61 | 72 % |
| Damsel | — | — | 96 % |
| Dexter | — | — | 0 % |

Two kinds of title. On Lion, Kar and Dexter the residual is dither, as the
documentation said. On Mercy, Otto and Damsel it is not: 72–96 % of it lies in
subband 0 (0–375 Hz), where the band SNR is 25–40 dB against 40–55 dB in the other
bands, and the dither switch moves it by 0.0 dB. Leaving subband 0 out, Damsel's
median object goes from 51.0 to 59.1 dB. In absolute terms the worst of these errors
is −94 to −97 dBFS.

Subband 0 is where the downmix's 90-degree phase shift is taken out by the fitted
37-tap filter. The two readings it was fitted between are far worse on Mercy
(band-0 SNR 5.9 dB flat, −2.5 dB untouched, against 27.7 dB), so the filter is doing
its job; the question is what it leaves.

### 4.5 The quadrature table is not the problem

`08-joc-quadrature-diagnosis.json`. Dolby's operator on subband 0, measured per
title with the fitting tool's own method:

- it is the same fixed network on all twelve titles (at 48 Hz, |R| 0.375–0.403 at
  −88 to −94 degrees; the table gives 0.398 at −88.7);
- the table is within 1–3 dB of each title's own best 37-tap fit on ten titles
  (Mercy −39.6 against −42.5; Otto −35.5 against −36.3; Damsel −35.3 against −36.7),
  and pooled refits make individual titles worse;
- but between −19.8 and −37.3 dB of Dolby-minus-flat (−14.6 and −17.4 on two titles)
  is explained by no per-frequency complex gain at all.

A fixed filter commutes with a constant matrix and not with the interpolated JOC
matrix, so the order of the two is the obvious suspect: oadec filters each downmix
channel and then applies the matrix. It is not the cause.

`08-joc-quadrature-diagnosis.json` (`order_experiment`). A side branch implements the
alternatives: the subband 0 correction applied after the matrix, paired with the matrix of
the same time slot or of the slot before or after it, and subband 0 mixed with the matrix of
the slot before or after the one the other subbands meet. Its default reading decodes all
six fresh titles to exactly the released numbers, so the experiment changes nothing by
being there. Every alternative moves subband 0 further from Dolby wherever it changes
anything. Median object SDR against Dolby, in dB:

| Title | current order | after the matrix | pairing −1 / +1 | subband 0 one slot early / late |
|---|---:|---:|---:|---:|
| Mercy | 40.30 | 34.26 | 32.67 / 35.64 | 37.70 / 37.00 |
| A Man Called Otto | 44.12 | 41.90 | 41.18 / 42.11 | 42.39 / 41.91 |
| Damsel | 50.98 | 42.75 | 41.72 / 43.52 | 39.27 / 37.88 |
| Kar Kardeşliği | 53.72 | 48.77 | 47.63 / 47.92 | 35.61 / 36.54 |

On Lion of the Desert the readings change nothing; on Dexter they leave the median where it
is and pull the worst object down, from 49.6 dB to between 36.2 and 49.0 dB. The order oadec
uses is the best of the six. What leaves the rest of the subband 0 residual is still open
(§9), with the obvious suspect ruled out.

### 4.6 The E-AC-3 core on fresh streams, and FFmpeg's default compression

`09-eac3-core-three-way.json`. Two DD+ 5.1 cores against FFmpeg and DEE
`ddp_decode`. Lion of the Desert is inside the envelope on every channel. Kar
Kardeşliği first came out 8–20 dB apart on all three decoders. The cause was FFmpeg:
its E-AC-3 decoder applies the stream's dynamic range words by default
(`drc_scale 1`), and on this stream the gain it applies moves between 0.62 and 1.06
over half-second windows. With `-drc_scale 0`:

| channel | oadec → Dolby | FFmpeg → Dolby | oadec without dither → Dolby |
|---|---:|---:|---:|
| L | 45.45 | 43.78 | 48.43 |
| R | 41.09 | 39.20 | 44.03 |
| C | 52.90 | 51.11 | 55.92 |
| Ls | 50.33 | 48.47 | 53.25 |
| Rs | 46.41 | 44.64 | 49.38 |

oadec is closer to Dolby than FFmpeg on every channel, and 3 dB closer again with its
own dither off. The stored FFmpeg references were made with `-drc_scale 0`; a fresh
one has to be too, and `tools/three_way.py` now says so.

## 5. A second model's adversarial review

`12-codex-adversarial-review.json`. `codex exec` (codex-cli 0.154.0, read-only) over
the code diff v0.2.0..v0.3.0 (46 files, 435 KB) with the codex-review-cc plugin's
security, reliability and simplification checklists and its JSON schema. Verdict
CHANGES_REQUESTED, six findings; every one was checked against the code and, where
it made a claim about behaviour, against a test before anything changed.

The six findings are called B1 to B6 here, to keep them apart from the review loop
over the finished branch below.

| | What it said | How it was confirmed | Disposition | Fixed in |
|---|---|---|---|---|
| B1 | The E-AC-3 programme decoder can stall delivery | A DD+ 7.1 stream with every dependent frame repeated decoded to one group of 375 and exited 0; a dependent substream that disappears held every later group until the end of the stream | agree | `82290a0`, `cc12bc7` |
| B2 | TrueHD delivery ignores the extra-data faults | A fixture with one Evolution byte changed: `verify` 7, every TrueHD `decode` 0 | agree | `6fcea74` |
| B3 | The object path skips the JOC size check | It called `Joc::parse` without `size_ok`, which `verify` and the PCM path check | agree | `a97a0e8` |
| B4 | A distance- or divergence-only update escapes the loss ledger | Unit tests on the old code: both counted 0 | agree | `2352bf1` |
| B5 | A trailing ADM event dropped as equal loses its loss counts | Unit test on the old code: `importance-omitted` 0 where 3 were dropped | agree | `7a6b93e` |
| B6 | E-AC-3 delivery fails on a tail overrun, against the exit-code policy | *The 400 Blows*: `verify` 7 as the policy says, `decode` 7 where it says 0 | agree | `08ec08b` |

### The review loop over the finished branch

After the fixes, the same reviewer read `main...verification-2026-09-14` round by round
under the plugin's contract: every finding checked against the code before anything
changed, every accepted one fixed test first with the workspace tests run, and the loop
repeated until it approved or five rounds were spent. `12-codex-adversarial-review.json`,
`gate_loop`.

| Round | Verdict | Finding | Disposition |
|---|---|---|---|
| 1 | CHANGES_REQUESTED, 437 s | R1F1, correctness, medium: the E-AC-3 path of `oamd` dropped the containers it could not open and the sync errors and unparsed frames of its walk, so a report missing metadata could say clean | agree. The code confirmed it, and showed the TrueHD path skipping unparsable access units, failed extra-data checks and unopenable containers the same way. Fixed on both paths in `d385cfd` |
| 2 | CHANGES_REQUESTED, 385 s, after a first attempt stopped on the reviewer's usage limit | R1F1 resolved. R2F1, correctness, medium: the walk `emdf` and `oamd` share could not tell empty skip fields from skip fields that hold data but no sync word, so a frame whose container was erased was left out and both commands exited clean, while `verify` and both decodes counted it | agree. Fixed in `0f267c2`: the walk counts, once per frame, every frame whose skip fields hold data and no container that opens, in a substream that carries EMDF. Checking the fix on real clips found two more things. `adb0b76` had counted skip fields that carry no EMDF at all into `verify` and the PCM delivery, which two AC-3 files fill in some 1 100 frames and on which `every_eac3_stream_is_clean` then failed; and a first version of this fix judged the stream as a whole, so the AC-3 core of the configuration 4 head, whose skip fields hold other data while its dependent substream carries the containers, read as 1 748 lost containers. Both are closed |
| 3 | CHANGES_REQUESTED, 323 s, after a first attempt stopped on the usage limit again | R1F1 and R2F1 resolved. R3F1, correctness, medium: the walk `emdf` and `oamd` share dropped the bytes the framing skipped, and the TrueHD walk of `oamd` the bytes it left trailing, so a clip whose last frame or access unit is cut short was clean for both while `verify` exited 7 | agree. The code confirmed it, and checking which stream faults of `verify` the walk still dropped found one more: neither command read the CRC of a syncframe. Fixed in `3e5af0e`: both count skipped bytes, TrueHD trailing bytes and AC-3 and E-AC-3 CRC failures into their verdict, and a file with no complete frame or access unit still exits 2. Of the 55 clips of the work directory, `emdf` and `oamd` now exit 7 on the 18 cut inside a frame or an access unit, as `verify` did, and nothing else changes |
| 4 | CHANGES_REQUESTED, 498 s, on a second attempt after the first stopped on the usage limit | R1F1, R2F1 and R3F1 resolved. R4F1, correctness, medium: the TrueHD decoder never checked the object metadata inside an Evolution container that opens, so on a clip whose object metadata will not parse the PCM and WAVE decodes exited 0 and `compare` called the stream bit-exact and clean while `verify` exited 7 | agree. The code confirmed it, and a substream the presentation leaves out behaved the same. Fixed in `77db761` for every TrueHD delivery: each runs the checks of `verify` beside its decode (G4 in §6) |
| 5 | APPROVED, 512 s, on a second attempt after the first stopped on the usage limit | R1F1, R2F1, R3F1 and R4F1 resolved; no new finding | the workspace tests the approval runs passed, 273 |
| 6 | CHANGES_REQUESTED, 331 s | R6F1, reliability, medium: `tools/media_regression.sh` reads the exit code of every step it runs, and `tools/replay_fuzz.py` returned 0 whatever the campaign found, so a silent accept, a fault reported at exit 0, a panic or a timeout could not reach the aggregate verdict the script and the documents claim | agree; the campaign judges itself now and says so in its exit code, G5 in §6 |
| 7 | CHANGES_REQUESTED, 494 s, on a second attempt after the first stopped on the reviewer's usage limit | R6F1 resolved, confirmed by the reviewer with campaigns of its own. R7F1, correctness, medium: the three places that count a lost EMDF container asked whether the frame carried skip bytes before they asked whether a container opened, so a frame of a JOC substream with no skip field at all was silent in all of them | agree; they ask only where a container could be now, G6 in §6 |

Round 4 first stopped on the usage limit of the reviewer after 360 s. While it waited,
the class of R3F1 was checked as a whole instead of case by case, and it was still open:
W4 in §6. Round 5, the last the loop allows, approved the branch with G4 in it.

A sixth round was run after the branch closed the open items of §9, five commits
later. The loop's cap of five counts the debate over one body of work; this was
a fresh gate over new work, and it found one thing, in the script written to
close the item about CI, not in the decoder. Round 7 read the fix for it and
called it resolved, and found one more, this time in the decoder's own
accounting: R7F1, fixed here. Round 8, which reads that fix, is owed: its first
attempt stopped on the reviewer's usage limit after 55 s and it is scheduled for
the reset. Until it returns, the fix for R7F1 stands on its own tests and
measurements -- the two unit tests of the rule, the media suite at 28 of 28, and
55 real inputs whose exit codes do not move -- and not on a second model's word.
Four of the eight attempts these rounds took were stopped by that limit.

## 6. Defects found and fixed

Every decoder fix was written test first: the test named failed on the code before the
fix and passes after it. Five fix branches were merged with `--no-ff`; the rest are
commits on the branch.

| | Found by | What went wrong | Test that failed on the old code | Commit |
|---|---|---|---|---|
| B1 | adversarial review | A substream frame repeated inside one group stalled every later group | `a_duplicated_dependent_frame_is_counted_and_delivery_continues` (media) | `82290a0` |
| B1 | adversarial review | A dependent substream that disappeared held every later group in memory until the end of the stream | `a_dependent_substream_that_disappears_does_not_stall_delivery` (media) | `cc12bc7` |
| B2 | adversarial review | TrueHD delivery ignored the extra-data faults `verify` counts | `a_corrupted_evolution_block_fails_every_delivery_like_verify` | `6fcea74` |
| B3 | adversarial review | The object path took a JOC payload of the wrong declared size and exited 0 | `a_joc_size_mismatch_fails_the_object_output` | `a97a0e8` |
| B4 | adversarial review | A distance- or divergence-only update was never counted as a loss | `a_distance_only_change_is_counted_once_without_an_event` | `2352bf1` |
| B5 | adversarial review | The losses of a trailing ADM event dropped as equal were not counted | `a_popped_trailing_event_still_has_its_losses_counted` | `7a6b93e` |
| B6 | adversarial review | E-AC-3 `decode` and `compare` exited 7 on a tail overrun the policy calls clean | `a_tail_overrun_alone_does_not_fail_the_object_output` | `08ec08b` |
| A1 | audit matrix, FAIL | Three environment variables changed the JOC decode and appeared in no output | `a_joc_lag_override_is_announced_and_recorded` | `f419bb5` |
| A2 | audit matrix, FAIL | The TrueHD WAVE mask set `SPEAKER_ALL` for wide left, dropped channels past bit 31 and misnamed the side pair under `--order stream` | `the_wave_mask_is_written_only_when_it_is_true` | `d298742` |
| A3 | audit matrix, PARTIAL | The 24-bit writer wrapped a sample past 24 bits: 1 << 24 became 0 | `clamp_i24_saturates_outside_the_range` | `3d93d06` |
| A4 | audit matrix, N/I; exploration | A mid-stream configuration change left a WAVE header declaring no data and exited 2; a WAVE output past 4 GiB failed after writing past the limit | `a_configuration_change_leaves_a_consistent_wav_and_exits_7` (media), `a_wave_file_past_4_gib_is_promoted_in_place` | `df7b93b` |
| A5 | audit matrix, PARTIAL | `oamd` refused E-AC-3, and `emdf --dump` stopped at four objects | `oamd_reads_the_emdf_containers_of_an_eac3_stream`, `emdf_dump_prints_every_object` | `acae745`, `3ae1877` |
| A6 | audit matrix, PARTIAL | A reserved `joc_ext_config_idx` was an anonymous error, and on the first frame the object decode exited 2 | `a_reserved_joc_extension_is_named_and_the_matrices_held` | `c5741c8` |
| A7 | audit matrix, PARTIAL | Nothing checked Table 56 or the complexity index | `a_payload_configuration_outside_table_56_is_non_conformant`, `a_complexity_index_that_disagrees_with_the_oamd_is_non_conformant` | `6a00745` |
| A8 | audit matrix, PARTIAL | `verify` had no lossless statistic | `verify_decode_reports_the_lossless_checks_of_every_presentation` | `5ea960e` |
| S1 | command matrix | `info`, `oamd` and `emdf` exited 0 on an empty or random file | `a_file_that_holds_no_stream_is_refused` | `f7a04c3` |
| S2 | command matrix | `decode` exited 0 on an empty file | `a_file_without_a_stream_is_refused_by_every_decode_format` | `0a1ef96` |
| W1 | while fixing A5 | `emdf` and `oamd` read AC-3 headers as E-AC-3: 902 frames and 902 sync errors in a clip of 1 171 frames, exit 7 | `emdf_frames_ac3_streams_as_info_and_verify_do` (media) | `8f5f857` |
| W2 | the merged build | On an E-AC-3 file with no whole syncframe, `info` exited 0 and `decode --format wav` exited 7 and left an empty file | `a_file_without_a_whole_syncframe_is_refused_by_every_decode_format` | `86f4022` |
| T1 | re-run | The ADM regression stage crashed on a fresh work directory and could not pass after 0.3.0 | the stage itself | `27b65ac` |
| T2 | re-run | The two gate scripts exited 0 whatever the verdict | their exit codes, with media and without | `f801d72` |
| G1 | review loop, round 1 | `oamd` reported a walk it could not fully read as clean: containers that did not open, sync errors, unparsed frames and units, failed extra-data checks. Of the 30 clips in the work directory one changes its exit code, the two TrueHD streams spliced end to end, 0 to 7 on the resynchronisation at the splice, where `verify` exits 7 too | `oamd_and_emdf_judge_the_same_unread_metadata`, and the `oamd` check in `a_corrupted_evolution_block_fails_every_delivery_like_verify` | `d385cfd` |
| W3 | while writing the test for G1 | An EMDF container whose declared length disagreed with its syntax opened as if it were right, since the length was checked only against the data after it. A frame whose skip fields held no container that opens left `verify` at exit 0 although it named that frame as the first problem, and the object and PCM decodes held the previous matrices without a word. Every stream measured declares the length exactly, 0 of 23 389 containers in 26 files otherwise, and no clip of the work directory changes its exit code; `13-emdf-container-length.json` | `a_declared_length_that_disagrees_with_the_syntax_is_refused`, `a_frame_whose_container_does_not_open_is_counted`, `a_container_whose_declared_length_disagrees_with_its_syntax_does_not_open` | `adb0b76` |
| G2 | review loop, round 2 | `emdf` and `oamd` left out a frame whose container was erased, its skip fields left the length they were, and exited clean where `verify` and both decodes exited 7. Checking the fix showed that W3 had counted skip fields carrying no EMDF at all into `verify` and the PCM delivery, which failed `every_eac3_stream_is_clean` on two AC-3 files; a frame without a container that opens is now a fault only in a substream that carries EMDF, which also keeps the AC-3 core of a configuration 4 stream out of the count. No AC-3 or E-AC-3 clip changes its exit code | `a_frame_whose_container_is_erased_is_a_fault_for_every_command`, `skip_fields_that_carry_no_emdf_are_not_a_fault`; the configuration 4 head in the media test `the_metadata_scanner_and_the_verifier_count_the_same_containers` | `0f267c2` |
| G3 | review loop, round 3 | `emdf` and `oamd` dropped the bytes their walk skipped or left trailing and never read the CRC of a syncframe, so a clip cut inside a frame or an access unit, or a frame whose CRC failed, was clean for both where `verify` exited 7. Of the 55 clips of the work directory the 18 cut inside a frame or an access unit change, 0 to 7, and no other; a file with no complete frame or access unit still exits 2 | `a_cut_stream_is_non_conformant_for_the_metadata_commands`, `a_frame_whose_crc_fails_is_non_conformant_for_the_metadata_commands` | `3e5af0e` |
| W4 | checking the class of G3 as a whole | `emdf` and `oamd` re-derived the verdict of a stream from their own walk and missed every fault the walk does not read: on copies with a bit changed in a TrueHD substream, a JOC payload that cannot be read, a reserved JOC extension, a payload configuration outside Table 56 or a wrong complexity index, `verify` exited 7 and both commands 0. They run the checks of `verify` on a thread beside their walk now and take its verdict; run after the walk instead, that pass had taken `emdf` to 209 s on the E-AC-3 film below. No clip of the 55 changes its exit code, and on every one both exit as `verify` does; on an 838 MB E-AC-3 film `emdf` goes from 91 s to 123 s and `oamd` from 94 s to 123 s, where `verify` alone takes 118 s; on a 2.6 GB TrueHD film `oamd` goes from 1.5 s to 27 s, where `verify` takes 28 s, each command timed once with the file in the cache; `14-metadata-verdict-parity.json` | `a_bit_changed_in_a_truehd_substream_fails_oamd_as_it_fails_verify`, `a_joc_payload_that_cannot_be_read_fails_the_metadata_commands_as_it_fails_verify`, and the `emdf` and `oamd` checks of the Table 56, complexity index and reserved JOC extension tests | `981d262` |
| G4 | review loop, round 4 | The TrueHD deliveries judged a stream by what their decode read: a substream the presentation leaves out, or object metadata that will not parse, could be broken while the PCM and WAVE decodes exited 0 and `compare` called the stream bit-exact and clean, where `verify` exited 7. Every TrueHD delivery runs the checks of `verify` on a thread beside its decode now and counts its verdict. Of the 24 TrueHD inputs of the work directory only the clip whose object metadata will not parse changes, its PCM decodes from 0 to 7; a PCM decode of presentation 2 from a 2.6 GB TrueHD film took 68 s before and 62 s after, the 28 s scan of `verify` running beside it on another core. Two inputs keep a delivery that does not exit as `verify` does, both unchanged: Up (2009), whose decodes fail a lossless check word `verify` reads only with `--decode`, and the spliced clip, whose presentation 2 decode cannot continue past the splice (exit 2) | `a_fault_only_verify_reads_fails_every_truehd_delivery`; the media test `verify_and_decode_agree_about_a_truncated_object_metadata_element`; the unit test `the_verdict_of_verify_makes_a_decode_unclean` | `77db761` || O1 | closing the open items | `verify` and `compare` read a file that holds no stream, an empty one or a random one, and reported a clean verdict over nothing at exit 0, where every other command refuses it at exit 2 | `a_file_that_holds_no_stream_is_refused`, which now runs both commands | `ea38003` |
| O2 | closing the open items | A substream whose EMDF containers are all broken read like one that carries none, so `verify`, `emdf`, `oamd` and the deliveries called it clean: a frame without a container that opens is a fault only where containers do open, and in such a substream none does. The JOC extension declared in the `addbsi` is the second evidence that a substream carries EMDF, since that extension rides in an EMDF container (TS 103 420 clause 8.3.1) | `a_substream_whose_containers_are_all_broken_is_still_a_fault` | `41011c0` |
| O3 | closing the open items | `codecdatae` was held to nothing. Table 56 prints 1 for it; clause H.2.2.3.7 of TS 102 366, which the table cites, requires 0; every OAMD and JOC payload measured carries 0 | `a_field_outside_table_56_is_named` | `bc78fee` |
| O4 | closing the open items | A seamless branch reached the one restart header it was judged at, and the decoder skips the lossless check word where the branch is, so every other substream of that access unit compared a check word across the splice. Which substream that was depends on the presentation, so presentations disagreed about the same access unit: 0 and 2 of Up (2009) skipped the check at access unit 54205 where 1 and 3 failed it | `the_branch_reaches_every_restart_header_of_the_unit`, and `a_branch_reaches_every_presentation` (media) | `0268009` |
| G5 | review loop, round 6 | `tools/replay_fuzz.py` returned 0 whatever a corruption campaign found, and `tools/media_regression.sh` reads exit codes, so the aggregate verdict could pass over a silent accept, a fault reported at exit 0, a panic or a timeout. A timeout was doubly invisible: a non-zero exit with nothing said is what catching the corruption looks like | the campaign measured both ways on four sites of `talktome-joc-head.ec3`: with the release binary it passes and exits 0; with `$OADEC_BIN` pointed at a stand-in that exits 0 and says nothing, every command counts 4 silent accepts and the tool exits 1 | `c6c3a92` |
| G6 | review loop, round 7 | A frame of a substream that declares the JOC extension carries an EMDF container (TS 103 420 clauses 8.2 and 8.3.1), and a frame with no skip field at all had lost one. The walk `emdf` and `oamd` share, the statistics `verify` keeps and the object path's payload reader all asked for skip bytes before they asked whether a container opened, so the loss was silent in every one and the object decode held the matrices of the frame before it | the unit tests `a_frame_without_skip_fields_loses_a_container_where_emdf_is_declared` and `a_frame_without_skip_fields_is_a_lost_container_where_joc_is_declared`; the material cannot show it, since emptying a frame's skip field desynchronises the block it sits in and the frame stops parsing before its skip fields (measured: 62 of 63 containers, 0 container errors, exit 7 on a mantissa error) | `df4c104` |


## 7. The measuring tools

- **The ADM regression stage** decoded the records in sorted order, so the check of
  the one expected difference opened the DAMF of a sibling record that had not been
  written yet and the stage died on a fresh work directory; and after the 0.3.0 bump
  48 outputs no longer matched their audited hashes because the version string
  changed. It now decodes everything before judging and compares with exactly the
  version bytes put back.
- **The two re-run gates** wrote their verdict into JSON and always exited 0. They
  now exit 1 on a failure and 3 when a gate could not run.

## 8. A decision measured: RF64, not BW64

`11-adm-long-form-fourcc.json`. ITU-R BS.2088-2 names the long form of a
Broadcast Wave file `BW64`; oadec writes `RF64` (EBU Tech 3306 v1) with the same
`ds64` layout. The whole-film ADM of Pi (15 278 169 042 bytes) was offered to every
reader twice, once as written and once with only its first four bytes changed:

| Reader | RF64 | BW64 |
|---|---|---|
| ffprobe | reads | reads |
| EBU ADM Renderer's reader | reads | reads |
| `bwf_info` (DEE 5.2.1) | reads | spins; killed after 566 s |
| `atmos_info` 1.1 | reads | "missing 'RIFF/RF64' chunk" |
| `atmos_info` 5.7.2 | reads | refuses |
| Dolby Atmos Conversion Tool | converts | "must have a 'RIFF' or 'RF64' chunk" |

The file's consumer is a Dolby encoder, so `RF64` stays, as a measured deviation from
BS.2088. The audit's D14 closes on this.

### 8.1 A third reader: the Fairlight import of DaVinci Resolve

Every ADM reader that had accepted the output so far was Dolby's own (the Conversion
Tool, `atmos_info`, `bwf_info`) or the EBU renderer. The owner opened the ADM file of
the Pi head by hand in DaVinci Resolve Studio 21, on the Fairlight page, and the DAMF
set of the same decode after it. Resolve laid each out as one bed track and eleven
object tracks, Object 11 to Object 21, with its Dolby renderer monitoring 7.1.4.
That is what the file says it holds: `atmos_info` 5.7.2 reads a ten-channel bed (L, R,
C, LFE, LSS, RSS, LRS, RRS, LTM, RTM) and eleven objects, ids 0 to 10, over 5 075 800
samples, and exits 0 on both the ADM file and the DAMF set. A reader with no Dolby
code in it reads the same programme. Screenshots are in the work directory
(`resolve/resolve-fairlight-adm-1.png` and `-2.png`).

## 9. Still open

- **The subband 0 residual of three titles.** Mercy, A Man Called Otto and Damsel leave
  72–96 % of their object residual in subband 0, and between −20 and −37 dB of
  Dolby-minus-flat there is explained by no fixed response. The obvious suspect, the
  order of the fixed filter and the interpolated matrix, is ruled out: every reordering
  tried moves subband 0 further from Dolby (§4.5). The error sits at −94 to −97 dBFS.
- **Dolby's refusal of TrueHD object presentations.** `dlbtruehddec` opens 105 of 194 and
  refuses 89, Up (2009) among them. The refusal was narrowed this round and is still
  unexplained. It comes from `decode_oamdi()` as "Selected Dolby TrueHD presentation is
  not available", and it strikes one combination only: the raw object output
  (`out-ch-config=21`) at `presentation=16`. The same titles decode at presentations 2, 6
  and 8, at 8 with the raw output, and at 16 with a channel output. Nothing in the streams
  separates the two groups: every one of the 194 declares a sixteen-channel presentation,
  all carry four substreams and the same flags, and `substream_info` and the dynamic
  object count overlap. Neither does the object metadata, compared this round on twenty
  titles from each group: the program shape is the same (dynamic objects only, an LFE bed,
  no ISF), and object counts, element ids, blocks per payload, sample offsets, ramp
  durations and block statuses overlap. The container is not involved either, since the
  raw stream refuses the parser in both groups and the MP4 wrapper is required for both.
  The association recorded in `docs/audit/conformance-matrix.md`, with
  `channel_meaning.twoch_dialogue_norm`, is unchanged and remains an association. Our
  decode and `truehdd` both read the object presentation of the refused titles cleanly,
  and the 89 are byte-identical to `truehdd`'s output, so the refusal measures Dolby's
  tool and not the streams.
- **The lossless check words of Up (2009).** Six failures across the four presentations
  when the round began; two of them were ours and are fixed (§6), and four remain. The two
  were at access unit 54205, the seamless branch of the stream: the branch reached the one
  restart header it was judged at, so the substreams of the other presentations compared a
  check word across the splice, and presentations 0 and 2 skipped the check where 1 and 3
  failed it. All four skip it now and agree.

  The four that remain are one event, access unit 77078, one failure in each presentation.
  Every restart header there carries a check word of 0x00 while the decoded state folds to
  something else, and the output timing steps as it should: no clock jumps, so this model
  judges no branch. `truehdd` reports a seamless branch at that access unit with an advance
  of 40 and calls the stream conformant, so it recognises a restart this model does not.
  What the evidence does not say is how it tells such a restart from an ordinary access
  unit: a latency change alone cannot be the rule, since the advance breathes with the FIFO
  in every stream measured -- taking it as the trigger judged 578 valid and 164 invalid
  branches on this title alone and turned 14 of the 55 comparison inputs from exit 0 into
  exit 7. Decodes of this title still exit 7; presentation 3 is still byte-identical to
  `truehdd`.
- **A mid-stream configuration change** ends the output, by design rather than by
  omission. `StreamConfig::incompatible_with` names four fields, and every one of them
  changes the shape of what a decode writes: the substream count, `substream_info`, the
  samples per access unit and the sampling frequency. One PCM or WAVE file cannot hold
  both sides of such a change, so the output ends at that access unit in a consistent
  file and the run says where it stopped and exits 7. A change that touches none of the
  four is decoded through, which the unit tests of `crates/oadec-truehd/src/au.rs` hold,
  and the media test `a_configuration_change_leaves_a_consistent_wav_and_exits_7` holds
  the other side.
- **Material that does not exist anywhere in the library**: downmix configurations 1 and
  2, two-point interpolation, EMDF in auxiliary data, a stream carrying object divergence.
  These stay N/T, and the round settled why the first of them cannot simply be made.
  `oadec eac3-joc-config` rewrites `joc_dmx_config_idx` in every JOC payload, so the
  obvious move is to relabel a configuration 3 stream as 1 or 2. It does not work, and the
  reason is in the field: the index selects the number of downmix channels of table 48,
  which sets how much the payload carries. Relabelling the committed fixture to 1, 2 or 4
  leaves a payload that ends early, at bit 2056 of the first frame, in all 63 frames;
  `verify` exits 7 and the object decode refuses the stream. Only 0 and 3 read back, the
  two an encoder writes. The reserved indices 5, 6 and 7 are refused by name, which is the
  one thing the exercise does confirm on a real stream. Configurations 1 and 2 would have
  to be encoded, not relabelled;
  `evidence/verification-2026-09-14/15-dmx-relabel.json`.
- **CI does not run the media suite,** and cannot: the corpus is licensed material, tens
  of gigabytes of it, and no public runner may hold it. Every figure here rests on a local
  run with the work directory. What the round could do is make that run one command with
  one verdict rather than a sequence assembled by hand: `tools/media_regression.sh` runs
  the media suite, the same suite without the corpus, the three TrueHD gates, the object
  gate, the two corruption replays and the ADM harness when its environment is there,
  prints an exit code for each and fails if any of them failed.
- **A substream that carries Object Audio Metadata and no JOC, whose containers are all
  broken,** reads like one that carries none. A frame without a container that opens is a
  fault only where containers open, and the second evidence that a substream carries EMDF
  is the JOC extension declared in its `addbsi`, which an OAMD-only substream does not
  carry; every JOC stream measured declares it in every frame. The object decode still
  refuses a stream whose first frame carries no JOC payload it can start from.

## 10. Reproducing

Scripts and logs of this round are in `E:\oadec-work\verify-2026-09-14\`
(`smoke.py`, `fresh.py`, `dialnorm.py`, `dither_check.py`, `lowband.py`,
`quad_diag.py`, `core_threeway.py`, `kar_core.py`, `bw64_test.py`,
`build_evidence.py`, `final_regression.sh`, `smoke_final.py`, `codex_baseline.sh`,
`codex_round.sh`, `final_summary.py`, `exp_lowband.py`); they are not part of the
repository. The experiment of §4.5 is the local branch `exp/lowband-after-matrix`.
