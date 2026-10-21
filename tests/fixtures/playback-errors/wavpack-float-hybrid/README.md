# Owned WavPack hybrid float and foreign-ID lacing regressions

Generate separately with `python3 scripts/generate_wavpack_float_hybrid.py`.
No external codec, generated artifact, private source, subprocess or network is
read. Ordinary tests consume checked-in artifacts without running generation.

216 accepted videos cover 135 paired hybrid float cases (mono/stereo/false
stereo, SNR/bitrate-delta/zero-limit, zero/one/shared/sent/optimized WVX mantissas,
three exponent ranges) and 81 main-only overshoot/shift controls. The scalar
oracles preserve IEEE words, including non-finite bit patterns, rather than
passing them through float arithmetic. Lossy and corrected output are separate
controls. Tests cover raw and compact pair APIs, Matroska, neutral video,
reset/rewind, partial-block seek, presentation and finite PCM worker windows.

Main streams without WVX must normalize oversized lossy cores by changing the
exponent; the strict lossless WVX mantissa bound does not apply to that path.
The malformed WVX-bound and float-CRC witnesses preserve the corresponding
specific refusals, including successful recovery on a following valid pair.

Three separate Xiph/fixed/EBML-laced movies carry an unrelated BlockAddID 2.
The main WavPack frames remain independently decodable, with six exact 20 ms
windows and exact seek. These are not laced-correction acceptance witnesses.
The codec's true correction is ID 1, and its established producer disables
lacing when correction is present. No new correction-lace wire format is
invented here; the previous ID 1 refusal witness remains a refusal.

2,682 generated artifacts total 4,671,238 bytes, excluding this README.
Before the fix, acceptance failed at the actual float mantissa bound and the
foreign-ID lacing guard, rather than an unrelated malformed fixture condition.

## Optional reference

Use `benches/wavpack_hybrid_reference.c` against a separate WavPack checkout at
`c9515833d1867d9ec03774d954369107f2e88a73` and invoke
`benches/check_wavpack_hybrid_reference.py` with
`--fixtures wavpack-float-hybrid --correction`, or `--lossy`, plus its required
oracle, reference-commit and report arguments. All 216 streams match byte-for-byte
with zero reference CRC errors in both modes. The helper writes native IEEE
words for floating streams; it does not numerically reinterpret them as integers.
The preceding 184 float/optimized-INT32 controls also match independently.

Finite worker tests are not physical-device/non-finite-output qualification.
Other predictor/profile/extra-bit combinations, custom channel identities and
codec-wide completeness remain outside this finite matrix.
