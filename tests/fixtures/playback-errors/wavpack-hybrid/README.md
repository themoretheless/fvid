# Owned WavPack hybrid main streams

Generate separately with `python3 scripts/generate_wavpack_hybrid.py`.
No private source, external codec, process, network or generated artifact is read.
Ordinary automated tests consume checked-in fixtures and do not run generation.

105 accepted 60 ms VP9/WavPack videos cover:

- 96 nonzero entropy streams: 8/16/24/32-bit mono, stereo and false stereo,
  SNR, bitrate, signed bitrate deltas, stereo balance including both saturation
  branches, and zero error limits (exact truncated codes).
- Header shifts, joint stereo, lossy clipping and INT32 zero/one/duplicate bits,
  including absent low-bit streams and 32-bit lossy shift redistribution.
- Shaping flags/metadata on main streams: shaping affects correction offsets,
  not the independently decodable main stream.
- Six zero-run streams with nonzero predictor histories and slow-level decay.
- Three six-channel frames combining stereo, mono and false-stereo components.

The encoder chooses explicit quantizer intervals and emits sign/branch bits;
scalar expected PCM is computed from those intervals, predictor controls and
integer fixup. Per-slot traces retain zone, error limit, quantized midpoint and
branch count. They include all five zones, nonzero quantization and zero-limit
codes. Each movie has raw full-header frames, compact Matroska packets and three
neutral VP9 frames. Tests cover exact PCM, CRC, reset/rewind, partial-block seek,
reader presentation and headless audio-worker timing. No physical audio device
or complete WavPack profile coverage is claimed.

Five malformed videos exercise missing/short hybrid profiles, unsafe exponents,
truncated main entropy and truncated correction entropy. Refusal tests require
the specific error and a subsequent good frame must decode successfully. The
former blanket correction refusal is superseded by acceptance in the separate
`wavpack-correction` matrix.

There are 1,139 generated artifacts totaling 3,217,328 bytes, excluding this README.

## Optional independent reference

Ordinary tests do not need the reference. To reproduce the external comparison,
build a separately supplied WavPack checkout at commit
`c9515833d1867d9ec03774d954369107f2e88a73` as a static library, then compile
`benches/wavpack_hybrid_reference.c` against its headers/library. Invoke
`python3 benches/check_wavpack_hybrid_reference.py --oracle /absolute/path/to/oracle
--reference-commit c9515833d1867d9ec03774d954369107f2e88a73 --report /tmp/report.json`
as one command. All 105 concatenated raw streams were compared byte-for-byte,
with the reference requiring zero CRC errors. The external library is not linked
into FVid, fixture generation or ordinary tests.

Hybrid correction and shaping now have a separate acceptance matrix. Float-hybrid
combinations and other predictor/profile combinations remain unqualified.
