# LD-SBR reference renderer alignment and zero-core harmonic qualification

## Observed cause

Native ordinary LD-SBR uses complex CLDFB/QMF. The native reference probe's
real/complex option selects spatial decorrelation; it does not switch ordinary
LD-SBR to Low Power. Original FDK automatically selects Low Power for ordinary
ELD without LD-MPS. Its public AAC_QMF_LOWPOWER parameter selects complex0,
Low Power1 or automatic-1. The primary implementation is
[CAacDecoder_SyncQmfMode](https://android.googlesource.com/platform/external/aac/+/ee76460efbdb147e26d804c798949c23f174460b/libAACdec/src/aacdecoder.cpp).

The external driver now defaults explicitly to complex and accepts
--qmf=complex|lowpower|auto. It checks the API result and reports the requested
mode. Both optional reference workflows request complex explicitly;the LD-SBR
PCM workflow also exposes --fdk-qmf for controlled cross-renderer observations.
The native DSP is unchanged. No foreign codec enters production or normal tests.

## Controlled reproduction

Original ISO twi24/48 inputs and all ASC/AU/native PCM hashes reproduce the
October31 report exactly. Explicit auto and Low Power each reproduce the old
FDK PCM hashes exactly. Changing only the FDK QMF request gives these full-stream
zero-lag RMSE values (no prefix removal,delay/gain fitting or normalization):

| Input | Auto/Low Power | Explicit complex | Complex maximum absolute error |
|---|---:|---:|---:|
| twi1_24 |0.007378128924436|0.000008989427035|0.000025779008865|
| twi1_48 |0.016493538354241|0.000010535002658|0.000044822692871|

Both inputs contain zero core spectra in every AU. The syntax-only strict-noise
inspector verifies max_sfb0,empty spectral data,LD-SBR CRC and temporal coefficient
reconstruction across all96 original AUs. It separately checks192 authored AUs.
These controls isolate HF adjustment/synthesis from AAC-core time-origin issues.
This comparison is between native float and original fixed-point PCM16,not a
normative whole-codec waveform acceptance threshold.

The new optional mode-isolation tool checks all24 own programmes. Default FDK
complex matches explicit complex;auto equals Low Power in every ordinary ELD
case. Full-stream complex maximum error is0.0000516772270203 (under1.7 PCM16
LSBs);largest RMSE is0.000011138437045. Independent native/frozen float error is
exactly0 on this platform. Protocol corruption and invalid QMF arguments retain
specific refusal tests. Refusal is not counted as playback acceptance.

All26 original LD-SBR inputs have been re-fetched with original ISO MD5s and
inventory SHA256 verified. Complete complex-mode raw comparisons preserve every
previous input/ASC/AU/native PCM hash. The other24 inputs retain substantial raw
zero-lag differences. AAC-core time origin and possible remaining DSP differences
require separate qualification;they are not closed by this reference-mode correction. LD-MPS/tools and the overall codec-gap objective
remain open. Earlier automatic-mode reports remain historical cross-renderer
observations;their numbers are preserved.

## Authored acceptance and navigation

24 new own eight-AU programmes/48 movies cover480/512 core frames,single/dual-rate
CRC LD-SBR,fine envelope24/36/48 and noise7/30. max_sfb0 makes all core PCM zero.
Independent zero-current-energy gain/noise/harmonic formulas and direct complex
CLDFB synthesis supply frozen float gold. No private/ISO media or codec parameter
sets and no foreign/native decoder supply these assets. The formulas follow the
existing independently qualified gain/boost contract,including Cor.1 bounds.

Native MP4/Matroska PCM is identical to frozen gold.16 fine-envelope increments
multiply PCM by8 exactly without clipping or phase restart. The actual audio
worker checks all48 movies through playback,live-history seeks,frame boundaries,
EOF and rewind,including15-slot phase carry. A real macOS device drains
n480-e36-q7-x2 through full/seek25%/seek60%/rewind. This does not certify listening
quality. Two explicit offline regenerations with sockets/subprocesses disabled
reproduce all74 generated files byte-for-byte. Normal tests never generate them.

## Reproducible original source pin

A freshly downloaded Gitiles archive has SHA256
`d018f6356b7d7bca5af4df9eca076fe21ece81e5251854882989c51e37af6286`,different from the
historical transport hash because Gitiles stamps tar members at download time.
The content pin is the complete Git tree of commit
`ee76460efbdb147e26d804c798949c23f174460b`:
`c3900d70bbdf4129d37f8cd5ab4a6589a34eb95b`,independently verified against its
[primary commit object](https://android.googlesource.com/platform/external/aac/+/ee76460efbdb147e26d804c798949c23f174460b?format=JSON).
All425 files/9746870 source bytes match,including paths,contents and executable
bits. The verifier ignores transport timestamps,checks bounded decompression,
rejects duplicate/escaping paths,links and collisions,and fails on a changed tree.
It never updates its expected tree automatically. Historical archive/report
hashes are retained. Original sources/builds stay outside authored fixtures.

Nine pure-stdlib tests use synthetic archives and independently frozen Git
object hashes. They verify timestamp independence and reject content/path/mode
changes,duplicate/escaping paths,links,collisions and compressed/payload bounds.
No Git executable,network or FDK is needed by those ordinary tests.

## Validation and commands

23 scoped tests pass,0 fail,0 ignored:13 Rust tests across five related AAC/LD-SBR
integration targets,one actual worker regression and nine Python source-pin tests.
The prior1138-test October31 gate is not presented as a fresh whole-workspace run.
New production DSP changes are unnecessary for this renderer mismatch.

```sh
python3 scripts/test_fdk_reference_source.py
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline --no-default-features --features player --test aac_alignment --test aac_eld_cldfb_phase --test aac_eld_sbr_noise_range --test aac_ld_sbr_q31 --test aac_ld_sbr_silent_harmonics
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline --no-default-features --features player --lib silent_core_harmonics_keep_worker_phase_across_seek_and_rewind
cargo --config profile.dev.package.fvid-media.opt-level=2 build --offline --no-default-features --features player --example aac_ld_sbr_inspect
```

`validate_aac_ld_sbr_qmf_modes.py` reproduces the optional authored three-mode
cross-check;`validate_aac_ld_sbr_fdk_pcm.py --fdk-qmf complex` reproduces the26-input
ISO comparison. Both require explicit pinned source archive,reference/native
probes and external work/output paths. The former never fetches media;the latter
uses separately fetched ISO inputs. No ordinary test invokes either workflow.
See matching qmf-tests,qmf-modes,complex-fdk,legacy-reproduction,zero-core-inspection,
generation and device JSON reports dated2026-11-01 for hashes/counts/metrics.
