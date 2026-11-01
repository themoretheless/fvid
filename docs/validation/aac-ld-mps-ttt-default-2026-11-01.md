# Predictive TTT default ICC acceptance

Native LD-MPS previously rejected parameter data-mode0 for every predictive
TTT ICC region with `predictive LD-MPS TTT ICC requires non-default data`.
This confused the entropy/default coding mode with a physical correlation.
The default index is0 and dequantizes to coherence1. Fine index5 dequantizes to
physical coherence0; the existing predictive-transform singularity guard still
rejects that value. There is no change to matrix arithmetic or dequantization.

The incorrect per-region default prohibition is removed. The raw syntax tests
that asserted its error are replaced by positive movie acceptance. Malformed
slot/extension tests and all777384 truncated-prefix transaction checks remain.
Raw numerical components still retain their separately documented scope.

## Independent source evidence

The pinned official reference archive is supplied separately:
https://standards.iso.org/iso-iec/23003/-2/ed-2/en/SAOC_IS7_ReferenceSoftware__20160603.zip

SHA256 `b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`.
In unmodified `sac_bitdec.c`,`ecDataDec` and `mapIndexData` set default indices
to `ICCdefault=0`;the dequantization table maps this to1.0000. A following hold
retains that index. The same table maps index5 to0.0. The predictive matrix
formula divides by physical coherence,not by the entropy data-mode number.

The full original spatial parser/renderer processes both the default/hold and
explicitly coded unity frames on the same authored input PCM. All72 whole-stream
outputs (36 programmes × two decorrelators) are byte-identical between those
two syntax representations. No owned prepared parameters or matrices are fed
to the original decoder. Archived codec sources are unchanged;the harness
adapts raw float IO and supplies stereo input. 7272 output channels are explicitly
reordered from LF/RF/C/LFE/LS/RS/AL/AR to WAVE mask0x63f order before comparison.
This is a speaker-order conversion,not sample fitting or channel searching.

The original output oracle contains811008 float scalars. Native default/hold
PCM agrees within maximum absolute error1.1496013e-9 at zero lag;the regression
limit is1e-7 full scale. No delay search,trimming,gain fit or channel fit occurs.
This independently qualifies the new syntax acceptance on these programmes;
it does not certify every LD-MPS tool/profile or the AAC core as a whole.

## Authored regression and playback

The36 programmes cover trees525/7271/7272,predictive modes0/1,parameter grids
4/9/23 and two histories: default in an independent first frame,and default
after coded non-unity history followed by hold. Each has an explicit-unity
coding control. All use three nonzero authored AAC-ELD core AUs,48kHz output
and64-bin spatial processing. Both MP4 and Matroska movies are checked in.
No private/ISO media,frames,audio or codec parameters are copied into fixtures.

Before the fix,the new playback test fails on the specific default-prohibition
message after its explicit-unity control has decoded. After the fix,all owned
default/control PCM is byte-identical. Tests check both decorrelators,native
checkpoint replay,public/owned MP4 and owned Matroska decoding,live rewind/seek
at sub-frame/frame/EOF positions,and repeated interval extraction. Existing
frame,assembly and in-band suites also pass:13 focused tests total,0 failures.

Three actual macOS output-device runs (one per tree,23 bands,predictive TTT
mode0,reset/hold,the default Complex renderer)
complete full playback,25% seek,60% seek and rewind with device EOF drain.
These short32ms programmes qualify scheduling/drain;they are not a subjective
audio-quality or sustained-performance test. The device JSON retains output.

Rerunning the fixture generator leaves the148-file corpus byte-identical:
146 authored movie/packet/manifest files are regenerated;the independent
PCM/reference metadata files are retained unchanged.
The source-reference generator is separate and requires the external pinned
archive and a C compiler. Ordinary tests only read committed artifacts and do
not invoke FFmpeg,reference decoders or network services.

## Explicit reproduction

```sh
python3 scripts/generate_aac_ld_mps_ttt_default_fixtures.py
python3 scripts/generate_aac_ld_mps_ttt_default_reference.py "$ISO_ARCHIVE"
cargo test --offline --no-default-features --features player \
  --test aac_ld_mps_ttt_default --test aac_ld_mps_frame \
  --test aac_ld_mps_assemble --test aac_ld_mps_inband
```

Reference/source hashes are in `aac-ld-mps-ttt-default-reference-2026-11-01.json`.
Other codec gaps,including unequal HRTF grids and unqualified energy-quantization
configurations,remain open. This correction does not complete the global goal.

## Subsequent split-region extension

The counts above describe the original36-programme milestone. The corpus now
contains48 programmes after adding twelve split-region cases. See
[aac-ld-mps-ttt-split-2026-11-01.md](aac-ld-mps-ttt-split-2026-11-01.md)
for current counts and the explicit unmodified-reference versus modified-source
count diagnostic boundary. The original source discrepancy is preserved,
not silently replaced by the diagnostic waveform.
