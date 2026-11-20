# Residual ICC-domain and matrix controls

`parameters.json`/`parameters.bin` hold12 own stateful programmes with84 frames:
fine/coarse, paired/unpaired coding, hold/default changes and1/4/23 bands. Each
frame supplies nonnegative residual ICC differences constrained to the valid
quantization grid. `reference.json` executes unmodified ISO ecDataDec and
mapIndexData with those differences. The test compares1,568 corrected integer
indices and bit consumption with C. Quantization tags follow the separate own
analytical state schedule. It also compares the legacy API
and verifies failed reads preserve the complete parameter state.

Corrections precede coarse-to-fine conversion. A coarse fine-domain value2 with
difference1 therefore becomes4, not3. The original entropy/mapped history must
retain the uncorrected value. Out-of-grid differences are rejected, never
silently saturated or passed to the reference's unchecked table lookup.

`matrix.json`/`matrix-packets.bin` hold192 controls for all8 trees, parameter
resolutions4/9/23,32/64-bin domains, shared/per-box ICC, coarse/fine and held/final
endpoints.384 assembled frames are compared with independently encoded ordinary
fine controls having the already-corrected per-box ICC. Matrix parsing/assembly
uses owned stages, so this is not an independent full-decoder oracle. All384
frames actually change matrix values relative to the uncorrected control.
These component packets intentionally carry an empty opaque residual-audio
extension: decoded ICC data is supplied explicitly to the external-residual
matrix API. They are not native compressed residual AAC playback fixtures.

The separate existing `aac-ld-mps-residual/native-residual-refusal.mp4` and its
saved spatial payloads contain real own compressed residual ICS. An additional
test reads that payload, reconstructs its ICC prefix and obtains the expected
physical table entries while preserving raw syntax. Native playback remains a
specific refusal until the source association, LD phase/bank and QMF path are
wired and qualified.

Explicit generation:

```sh
python3 scripts/generate_aac_mps_residual_icc_fixtures.py
python3 scripts/generate_aac_mps_residual_icc_reference.py /path/to/SAOC_IS7_ReferenceSoftware__20160603.zip
```

The own generator needs no process/network access. The second command explicitly
compiles the external reference for fixture generation only. Ordinary tests use
saved fixtures and owned Rust, without FFmpeg, a compiler or network. No private
source video/audio/codec parameters are included.
