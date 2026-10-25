# Native Dolby word-order regressions

Generate separately: `python3 scripts/generate_dolby_word_order.py`.
Normal tests read checked-in bytes without FFmpeg or network access. The generator
uses repository-authored neutral VP9, AC-3 tone and E-AC-3 spectra. It swaps only
16-bit audio words, preserving logical CRC/parameters, packet lengths, MP4 boxes,
video and timestamps. No private source media or parameter sets are copied.

ETSI TS 102 366 v1.4.1 Annex F.1 explicitly permits little-endian Dolby bitstreams
in ISO BMFF; Annex F.2 requires complete six-block E-AC-3 samples. Byte-split MP4
samples are a separate nonconforming compatibility extension, not a missing
standard coding tool. Source: https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.04.01_60/ts_102366v010401p.pdf

The 82 generated artifacts (2,771,016 bytes, excluding this README) give 54 LE or
per-syncframe mixed-order variants in MP4, Matroska and elementary streams.
Controls include native/coarse clocks, grouped AC-3, selected E-AC-3 programmes,
dependent 8-channel output, CRC-damaged first headers and corrupt unselected
payloads. Mixed-order streams exercise robustness beyond a uniform LE stream.

The parent decoder refuses `ac3-le.mp4` with `AC-3 packet holds no syncframe`.
Header-only normalization also leaves a distinct dependent-presentation failure:
`invalid E-AC-3 presentation audio block`. The raw bad-first-header companion
reproduces setup poisoning (`6 ch after ... 1 ch`); a test extracts those exact
raw bytes from the short neutral movie before checking the repaired reader.

`tests/dolby_word_order.rs` checks intended PCM/time/geometry acceptance, native
seek/rewind, the real scheduling worker with a capturing backend, complete-word
header prefixes, CRC refusal and unchanged demuxed payload words. Bad-first raw
AC-3 keeps its silent audible window and later valid tone. These finite fixtures
do not qualify every codec tool, arbitrary corruption or physical devices.
