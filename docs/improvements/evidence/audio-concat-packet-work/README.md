# Concat packet-work accounting for standalone sources

Accepted FVID-0477. With a remaining packet budget,the concat planner used
u64::MAX for G.711,Ogg,FLAC and MPEG sources. Subtracting it incorrectly marked
the entire budget spent. A reached later packet could therefore escape active
byte admission in the plan,while execution correctly rejected it without output.

Final unchanged five-test baseline:four controls pass,one acceptance fails in
all24 source/memory variants. Twelve short authored inputs cover mono/stereo
G.711,both laws,multiple blocks and the2048-frame boundary,Opus,Vorbis,mixed
Vorbis/Opus links,constant FLAC and MPEG Layers I/II/III. Every source is decoded
against an independent PCM oracle and its actual decoded_frames statistic.
Every later padded Opus packet is valid under its sufficient byte limit.
Exact budget exhaustion preserves the complete first waveform;healthy reached
successors preserve both waveforms. The only failing behavior is planning
acceptance of a reached oversized successor. Final candidate passes all five.

The additive library API standalone_packet_work reuses codec admission and
stored decoder indexes. G.711 descriptor,decoder and predictor share their block
window. Joint Ogg sums actual selected-link packets,excluding headers. FLAC/MPEG
use their owned frame indexes,and all counts retain the active prefix bound.
No PCM is decoded to obtain the count. Container-owned counts stay with callers;
None retains their conservative fallback. Existing APIs are not changed and no
migration or performance improvement is claimed.

The initial stand included incompatible successor channel geometry. Existing
whole-input compatibility already refused those cases,so the final fixtures
use matching geometry and valid packet padding to isolate byte admission.
That stand and its source/manifest are preserved;it adds no accepted item.

The first broad qualification exposed an independent pre-existing Matroska
counter/legacy-exporter mismatch. It is reproduced separately in the sibling
container-index evidence and corrected without weakening decoder admission.
All final selected integration,unit and configuration gates subsequently pass.

All fixtures/oracles are repository-authored;ordinary tests need no generator,
encoder,FFmpeg or network. Tests exercise real software decoding/export and
atomic refusal/publication. No physical audio/GPU or benchmark claim is made.
