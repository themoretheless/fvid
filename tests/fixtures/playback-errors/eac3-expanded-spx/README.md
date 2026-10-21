# Expanded E-AC-3 spectral extension

Regenerate separately: `python3 scripts/generate_eac3_expanded_spx.py`.
Thirty owned six-block 48 kHz programmes combine a sounding block-varying base
5.1 and an SPX-dependent rear pair. Attenuation none/[5,12]/[15,31], blends
31/0/8/16/24 and unity/changing final gain form the matrix. The SPX copy region
wraps 37..61 into 61..97. Both signal-only and noise-bearing cases are present.
Raw bundles, individual-syncframe Matroska and six-block MP4 are authored;
MP4 dec3 explicitly describes base 5.1 plus the rear dependent pair.

Independent-envelope SPX controls preserve the same synthesis parameters and
noise sequence. The assembly oracle independently maps roles and applies only
final-dependent gain/clamp. PCM is exact. This reuses existing qualified SPX
synthesis; it is not a new independent mathematical or external-codec oracle
for SPX itself. The direct test also checks second/third-frame rear-channel PCM:
signal-only output repeats, while noise-bearing output must advance. Attenuation
metadata and the sounding six-channel base are explicitly probed.

The shared `tests/eac3_mp4_aht.rs` suite now covers 162 MP4 cases and 126
coupling/SPX cases through raw and split-Matroska workers. Checks include direct
assembly/reset/dispatch, PCM, masks, PTS/duration, raw priming, rewind and partial
seeks (MP4 5/40/80 ms; raw/Matroska 40 ms into the second programme). This is
synthetic acceptance, not physical device, GUI or performance qualification.
No production decoder change was needed.

271 artifacts / 4,290,977 bytes excluding README reproduce exactly with process
and socket construction denied. The original 31 SPX-noise artifacts remain
byte-exact, as do presentation and enhanced-coupling assembly artifacts. No
private media, external encoder, FFmpeg or network. Generation stays separate
from ordinary tests. Other rates, quantizers, transient processing, same-stream
SPX/coupling combinations and dynamic layouts remain unqualified.
