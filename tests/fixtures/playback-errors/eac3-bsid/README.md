# E-AC-3 compatible bitstream identifiers

Generate separately with `python3 scripts/generate_eac3_bsid.py`. Authoring imports
only the owned synthetic bit writers; it does not invoke the older playback
fixture generator's external-reference workflow. No private media, generated
source artifacts, FFmpeg, external process or network is used.

ETSI TS 102 366 V1.3.1 E.1.3.1.6 requires version-16 E-AC-3 decoders to accept
backward-compatible BSID 11 through 15:
https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.03.01_60/ts_102366v010301p.pdf

Thirty-six positive cases cross all six identifiers 11..16 with long, short and mixed
transform windows, spectral extension, and spectral extension attenuation. Each
contains three non-silent stereo syncframes at 48 kHz, 96 ms total. Matroska videos
contain matching owned VP9 controls. The only differences from version-16 audio
controls are the five BSID bits and recomputed frame CRC. Reserved 9/10 and later
17/31 are negative cases.

`tests/eac3_bsid.rs` checks frame geometry, exact PCM equivalence, native `.ec3`
and Matroska playback, rewind and unsupported-ID refusal. PCM equivalence here
proves compatible-ID behavior against the unchanged version-16 decoder; it is
not an independent proof of all E-AC-3 reconstruction tools. Elementary streams
trim the existing 256-sample decoder delay; container streams retain it for the
container's presentation clock, checked explicitly against the same control.
Before the fix, the acceptance test fails specifically at `long-bsid11` header
recognition. Ordinary tests read the fixtures and never regenerate them.
