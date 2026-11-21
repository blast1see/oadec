# AC-3 and E-AC-3 fixtures: a dependent substream, and coupling

Four short streams of generated signals, for what the authored scene under
`crates/oadec-cli/tests/fixtures` does not carry: a programme of more than 5.1
channels, a stereo programme, AC-3, and a 5.1.2 JOC stream with an AC-3 core
and height channels, the carrier of the JOC downmix configuration 2 and 4
tests. Nothing here is taken from a title.

They stand in a folder of their own because the tests of the command line walk
every stream under their fixtures and hold the list to what it was.

| File | Bytes | SHA-256 | What it is |
|---|---:|---|---|
| `eac3-71-tones.ec3` | 163 840 | `d0d9ed8aba5f72bbc4205f7f385ae1c762b276f89411b123196e69b6838fc91b` | 40 frame groups of a 7.1 programme: an independent substream (`acmod 7` and LFE) and a dependent one (`acmod 5`, `chanmap 0x1a00`: Ls, Rs and the rear pair). Dolby Encoding Engine 5.2.1, `pcm_to_ddp` with `encoder_mode=ddp71` at 1024 kbit/s, from one sine a channel at -20 dBFS: L 400 Hz, R 630 Hz, C 1000 Hz, LFE 55 Hz, Ls 1600 Hz, Rs 2500 Hz, Lrs 4000 Hz, Rrs 6300 Hz. The first 40 groups of that encode, cut at a frame boundary. No coupling |
| `joc-512-tones.ec3` | 110 592 | `250c8102e6b008f7062903e887f765be4082b174a3996f0974a3d49792375500` | 24 frame groups of a 5.1.2 JOC stream: an AC-3 core (`acmod 7` and LFE) and an E-AC-3 dependent substream (`acmod 5`, `chanmap 0xa010`: L, R, Vhl, Vhr), JOC downmix configuration 4, 15 objects. Authored with `oadec atmos-author` from a scene of seven static objects, one sine each at -20 dBFS, at (-1, 1, 0) 400 Hz, (1, 1, 0) 630 Hz, (0, 1, 0) 1000 Hz, (-1, -1, 0) 1600 Hz, (1, -1, 0) 2500 Hz, (-1, 1, 1) 4000 Hz and (1, 1, 1) 6300 Hz, and a 55 Hz LFE bed at -20 dBFS, 96 000 samples; encoded by `dee_ddpjoc_encoder` of the Dolby Encoding Engine 5.7.2 with `--encoder bluray --data-rate 1152 --start 0`, and groups 8 to 31 cut out with `tools/ec3_cut.py`. Each core channel carries its own tone, the others at least 90 dB under it: L 400, R 630, C 1000, LFE 55, Ls 1600, Rs 2500, Vhl 4000, Vhr 6300 Hz. The carrier of the configuration 2 and 4 tests of `oadec-engine/tests/joc_configs.rs` |
| `eac3-20-coupled.ec3` | 32 256 | `c4f71d7449f59e6b39cd4c3819702cd8501bc4ff5276546db5d6cf452150bdef` | 63 frames of stereo E-AC-3 at 128 kbit/s that uses coupling and rematrixing. FFmpeg's `eac3` encoder (`ffmpeg version 2026-11-19-git-01ca75d350`, `-b:a 128k -bitexact`) |
| `ac3-20-coupled.ac3` | 32 256 | `505220dbe220dbdc31eba77440ebc173f2719e47a33e5a31e6eb47ea897729a9` | the same signal as AC-3: FFmpeg's `ac3` encoder, `-b:a 128k -bitexact`. Coupling and rematrixing |

The stereo signal is two seconds at 48 kHz, 24 bits: a band of noise the two
channels share above 5 kHz, noise of each channel's own between 1 and 4.5 kHz,
and a tone of 440 Hz in both. The noise comes from FFmpeg's `anoisesrc` with
its seed given, so the signal is the same on every run:

```
ffmpeg -filter_complex "anoisesrc=d=2:c=white:r=48000:a=0.1:seed=1,highpass=f=5000:p=2,asplit=2[s1][s2];anoisesrc=d=2:c=white:r=48000:a=0.06:seed=2,highpass=f=1000:p=2,lowpass=f=4500:p=2[l];anoisesrc=d=2:c=white:r=48000:a=0.06:seed=3,highpass=f=1000:p=2,lowpass=f=4500:p=2[r];sine=f=440:d=2:r=48000,volume=0.3,asplit=2[t1][t2];[s1][l][t1]amix=inputs=3:normalize=0[L];[s2][r][t2]amix=inputs=3:normalize=0[R];[L][R]join=inputs=2:channel_layout=stereo[out]" -map "[out]" -c:a pcm_s24le -bitexact src.wav
ffmpeg -i src.wav -c:a eac3 -b:a 128k -bitexact -f eac3 eac3-20-coupled.ec3
ffmpeg -i src.wav -c:a ac3  -b:a 128k -bitexact -f ac3  ac3-20-coupled.ac3
```
