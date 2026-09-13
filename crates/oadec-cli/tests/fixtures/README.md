# Test fixtures: one authored Atmos scene, encoded twice

Two short bitstreams of the same synthetic programme, so that CI can run
`oadec decode --format adm` and `--format damf` end to end without any film
material. Everything here was generated from `authored-scene.json`; nothing is
copied from a commercial title.

| File | Bytes | SHA-256 | How |
|---|---:|---|---|
| `authored-scene.json` | 1 372 | `1c12a6255439012927293e83b467edb98085bc02014fd6791f6eff7e0c0316c7` | the scene, written by hand |
| `authored-scene.ec3` | 112 896 | `c4481f2599328b3c21c98b9f8390bca7ec448077937bf41f74f834ce89f9efe8` | Dolby Encoding Engine 5.2.1, job `fixture-joc.xml` (E-AC-3 JOC, 448 kbit/s) |
| `authored-scene.mlp` | 546 456 | `d782d604093578e8c73a190e37ef7a6f7a4b5b7421cfaa274f9bfd406b026061` | Dolby Encoding Engine 5.2.1, job `fixture-thd.xml` (TrueHD Atmos, 12 spatial clusters) |
| `fixture-joc.xml`, `fixture-thd.xml` | | `bfb426123acd703a937c134a282fd4fb5b96178e8de0775cdfa83dd76bc24eb8`, `fad4d1ddcb0531d234c078d99fe292952c13e56c365ae7150aa96a0920a807e9` | the DEE jobs, verbatim (their paths name the machine they ran on) |

## The scene

Two seconds at 48 kHz, a silent LFE bed and six objects, each a sine at
−30 dBFS that identifies it by frequency, or impulses:

| Object | Signal | Metadata |
|---|---|---|
| front-left static | 440 Hz | (−1, 1, 0) |
| front-right static | 660 Hz | (1, 1, 0) |
| top centre static | 880 Hz | (0, 0, 1) |
| left to right in one second | 1320 Hz | five positions from (−1, 0, 0) to (1, 0, 0) at 0, 0.5, 1.0, 1.5 and 1.979 s |
| rear centre at minus six | 1760 Hz | (0, −1, 0), gain −6 dB |
| impulses front centre | impulses at 0.5, 1.0, 1.5 s, −20 dBFS | (0, 1, 0) |

The encoder re-times and clusters the metadata (the TrueHD encode carries
11 objects, the JOC encode 15, both with 1536-sample ramps), so the decoded
outputs are compared with each other and against what the scene makes certain
(track identity by frequency, event order, the ramps the profile replaces), not
sample by sample with the authoring master. DEE applies the authored gain into
the object audio and writes 0 dB, so the −6 dB object arrives 6 dB quieter with
no gain field; that is the encoder's behaviour, established in the remediation
report of 2026-09-10.

## Regenerating

```
oadec atmos-author authored-scene.json -o authored-scene
C:\dee\dee.exe --xml fixture-joc.xml     # writes authored-scene.ec3
C:\dee\dee.exe --xml fixture-thd.xml     # writes authored-scene.mlp
```

The jobs expect the DAMF set and the outputs under
`E:\oadec-work\audit\adm\remed\fixture`; adjust the four `<path>` elements
for another machine. A regenerated encode is not guaranteed byte-identical to
the committed one; the tests assert structure and identity, not the encoder's
bytes.
