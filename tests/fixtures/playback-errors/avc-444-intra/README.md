# Nonseparate H.264 4:4:4 compressed intra

Generate separately: `python3 scripts/generate_avc_444_intra.py`.
Run offline: `cargo test --offline --no-default-features --test avc_444_intra`.

60 owned progressive High 4:4:4 Predictive level 4.1 examples use one shared
macroblock header and three consecutive component residuals. I4 DC prediction
with zero and signed DC residuals, I8 DC prediction without residuals and I16
DC prediction with zero/positive/negative coefficients
cover 8/8, 10/12 and 14/8 component precision, isolated and shared slice boundaries, neighbour prediction, crop,
decoder reset, software MP4 output, timestamps and rewind. Expected samples are
computed by independent scalar integer equations in the authoring script.
No FFmpeg, network or private source parameters are used by generation or tests.

These examples qualify progressive CAVLC intra with deblocking disabled.
This matrix alone does not establish compressed CABAC, inter, field/MBAFF or enabled filtering
support for nonseparate 4:4:4.

Syntax and reconstruction follow H.264 (06/2019), residual syntax for
ChromaArrayType 3, shared intra prediction modes, 8.5.5 component transforms,
and 8.5.9 chroma QP derivation. This is not three separate_colour_plane slices.

Baseline reproduction (2026-10-21): decoding `i4-y8-c8-crop0` with the previously
built native decoder reports `AVC non-4:2:0 CAVLC non-PCM macroblocks are not
connected`. The acceptance test requires actual decoded samples, not that refusal.

Acceptance on 2026-10-21: all 60 cases passed native decode, exact three-plane
comparison, reset and MP4 packed output/crop/PTS/duration/sample-index/rewind.
A second independent generation matched all 241 generated artifacts byte for byte.

Shared-parser regression verification on 2026-10-21 also passed all 213 AVC
unit tests plus existing `avc_pcm_formats` (3 tests), `avc_444_pcm` (2),
`avc_monochrome_intra` (1) and `avc_422_intra` (1). Integration test sources
were compiled against the same offline, no-default-features native library.

Enabled loop filtering is qualified separately in `../avc-444-deblock`
and `../avc-444-pcm-deblock`.
