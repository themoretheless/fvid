# AAC LD-SBR/MPS single-rate synthesis, 2026-11-01

## Confirmed cause and correction

The AAC shared LD-SBR/QMF interface and the generic MPS PCM interface do not
use the same32-band synthesis modulation. The decoder previously passed both
through the generic MPS32 phase2n-127. The AAC single-rate LD-QMF convention
uses2n-31. Dual-rate64 uses2n-255 in both contexts.

The distinction is explicit in original AAC `InitSbrSynFilterbank` with
`FBMODE_LDQMF`,inside `audio.zip` in the official SAOC reference archive:
https://standards.iso.org/iso-iec/23003/-2/ed-2/en/SAOC_IS7_ReferenceSoftware__20160603.zip
Archive SHA256 `b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`.
This source evidence precedes the correction;no phase was fitted to ISO WAVs.

A separate explicit source-reference driver compiles the unmodified archived
function. A high-precision PI translation-unit control isolates its float PI
constant. All20480 real/imaginary matrix entries agree exactly with the direct
formula under precise PI. The generic32 matrix differs by0.02886254;the64
control stays identical. Original PI rounding is retained as a separate measured
1.727889e-7 coefficient difference,not silently changed in archived sources.

Owned synthesis now has a distinct single-rate AAC convention with a separate
matrix cache. Shared AAC construction selects it through `new_ld_sbr_qmf`.
Generic `new_qmf`,PCM analysis/synthesis,and64/128 bank conventions remain as
before. Session reset,SSC validation and reconfiguration retain the input
convention;fixed clip-gain changes recreate the correct synthesis bank.
The analysis window,HF signal,matrix/decorrelator logic and timestamps are
unchanged. No foreign code is linked into production.

## Specific synthetic acceptance

Eight authored eight-AU programmes cover480/512,single/dual rate,and constant /
in-band gain-reset variants. MP4 includes synthetic video;Matroska supplies the
paired audio transport. No private/ISO media,frames,audio or parameter sets are
copied. Own tree212 uses explicit CLD0dB and ICC1. Direct core/LD analysis,
HF/noise/gain and absolute-time convolution produce independent whole PCM.
The first matrix frame's M1/M2 interpolation gives alpha²;gain epochs at AU3/6
clear synthesis history while preserving the continuous SBR signal.

Before correction,all eight videos decode successfully. Four dual-rate controls
match exactly;four single-rate PCM comparisons fail with max errors
0.0004693–0.0006592. After correction,all eight match independent frozen PCM
byte-for-byte. This is waveform acceptance,not successful refusal. Tests cover
both transports,full playback,frame-edge sample seek/rewind,checkpoint replay,
reset,both decorrelators and both native frontends,including gain reconfiguration.
Generation is explicit and deterministic over two runs with network/process
creation disabled. Ordinary tests use frozen files without external tools.

## Original ISO references

The24kHz original LD-QMF2 full-stream comparison improves from RMS0.09650983604,
max error0.2980642626 to RMS7.771841685e-6,max error1.525133848e-5. The latter
is below half a PCM16 step. Both renderer modes improve identically,with zero
lag,no delay search,no gain fitting,and exact complete output geometry.

Only this original input changes. All six other renderer/output comparisons
(LD5151,ELD5151,48kHz LD-QMF1 in both modes) are byte-identical to the previous
audit. The previous FDK mismatch does not justify preserving the defective
shared-interface convention:the original AAC function independently specifies
it,and the new own controls reproduce the exact numerical failure.

The small AAC-LD5151 numerical residual and broader LD-MPS tools/profiles remain
open. These measurements do not certify complete codec conformance.

## Evidence and validation

- [Original function/matrix audit](aac-ld-mps-single-rate-reference-2026-11-01.json).
- [Before/after own reproducers](aac-ld-mps-single-rate-reproducers-2026-11-01.json).
- [Deterministic frozen generation](aac-ld-mps-single-rate-generation-2026-11-01.json).
- [Eight original comparisons](aac-ld-mps-single-rate-iso-2026-11-01.json).

[Fresh test/device report](aac-ld-mps-single-rate-tests-2026-11-01.json):
693 pass,0 fail,1 pre-existing ignored integer/float RGB acceptance. Media590,
integration102 across20 targets,and new audio worker1. FFmpeg/ffprobe are
blocked with zero invocations;builds are offline. Scope includes both generic
PCM and shared input,frame-length/source slicing,gain reconfiguration,STP/GES,
empty groups,clock/alignment and existing residual controls/refusals. Those
refusal tests do not establish new residual-profile acceptance.

Three actual macOS audio-device runs (both single-rate480/512 and a dual-rate
control,all with in-band gain changes) complete full/seek25%/seek60%/rewind drain.
This is device/navigation evidence,not a subjective listening-quality verdict.
Existing compound reference tests select the AAC convention explicitly for
shared single-rate sources;standalone PCM cases retain the generic MPS bank.
Previously frozen media and numerical golds are unchanged. The final new
checkpoint/frontend test retains exact replay equality and uses the same1e-7
frozen-PCM tolerance as other acceptance tests for platform math portability;
observed complete own PCM on this macOS run is byte-identical.

Optional source-reference execution is separate from ordinary tests:

```sh
python3 scripts/validate_aac_ld_sbr_synthesis_reference.py \
  --source-archive /path/to/SAOC_IS7_ReferenceSoftware__20160603.zip \
  --work-dir /path/to/external/research \
  --report /path/to/report.json
```
