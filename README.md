# oadec

Object-audio decoder engine for **Dolby TrueHD with Dolby Atmos** (all four
substreams, including the 16-channel object presentation and its object audio
metadata) and **E-AC-3 JOC** (Dolby Digital Plus with Dolby Atmos), written from
scratch in Rust.

Decoded programs are written as **DAMF** (`.atmos` master file sets) or **ADM BWF**
so that a licensed Dolby Encoding Engine can re-encode them.

Status: bootstrap. See `docs/` for the design, the verification strategy and the
evidence reports produced at every milestone.

## Licence

GPL-3.0-only. `oadec` is an independent project and is not affiliated with or
endorsed by Dolby Laboratories. Dolby, Dolby Atmos, Dolby TrueHD and Dolby Digital
Plus are trademarks of Dolby Laboratories.
