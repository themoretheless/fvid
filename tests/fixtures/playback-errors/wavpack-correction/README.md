# Owned WavPack hybrid correction pairs

Generate separately with `python3 scripts/generate_wavpack_correction.py`.
No private sources, previous generated artifacts, external codec/process or network
are read. Ordinary tests consume checked-in fixtures without generation.

306 accepted 60 ms VP9/WavPack videos contain independent main and correction
frames. The matrix covers all four integer widths, mono/stereo/false stereo,
SNR, bitrate/delta and zero error limits, explicit and implicit shaping state,
compact/expanded shaping metadata, positive and negative shaping weights,
old/new negative shaping, signed shaping deltas, CROSS_DECORR and joint stereo.
Predictor terms include 1/8/17/18 and stereo -1/-2/-3 with live histories and
adaptive weights. Eighteen six-channel cases combine stereo, mono and false
stereo components. Scalar oracles retain both lossy and corrected PCM.

Raw `.wv`/`.wvc` pairs and compact `.packet`/`.additional` pairs are controls for
standard Matroska BlockAdditional ID 1. The generator alternates default/explicit
ID spelling and element order. Tests verify exact samples, timebases, EOF,
reset/rewind, sample-exact partial-block seek, reader presentation and headless
audio-worker capture. Pair header/window matching, correction CRC, missing bits,
trailing words, short additional headers, duplicate IDs, unknown-ID isolation,
shared packet-byte admission and atomic last-component/layout failure are checked.
A different ID is a lossy-control case, not malformed correction acceptance.

Laced correction currently has a specific refusal fixture/test. That passing
refusal is not acceptance of laced lossless playback. Float-hybrid combinations,
DSD, custom channel identities and exhaustive tool combinations remain unqualified.
No physical audio-device qualification is claimed.

4,605 generated artifacts total 7,679,313 bytes, excluding this README. The old
source probe reproduced ignored container correction (lossy output) and an
independent negative predictor CRC failure. The latter exposed incorrect channel
history updates in terms -1/-2 and swapped output/missing histories in -3; these
are corrected and covered by the main-stream scalar/reference checks.

## Optional independent reference

Use the separately built reference WavPack commit
`c9515833d1867d9ec03774d954369107f2e88a73` and the harness documented in
`../wavpack-hybrid/README.md`. Run `benches/check_wavpack_hybrid_reference.py`
with `--fixtures wavpack-correction --correction` for paired lossless PCM, or
`--fixtures wavpack-correction --lossy` for main-only PCM, in addition to its
required oracle, reference-commit and report arguments. All 306 streams match
byte-for-byte with zero reference CRC errors in each mode. The reference stays
outside production, fixture generation and ordinary tests.
