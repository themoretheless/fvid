# HEVC range-extension transform skip limits

These synthetic 64x64, three-frame MP4/YUV pairs use HM 18.0 with
transform-skip maximum dimensions 8, 16 and 32, at 8 and 10 bits. Each has
an enabled and disabled counterpart (disabled retains the baseline 4x4 limit).
Scaling lists are enabled. The saved decoded YUV agrees between HM encoder
reconstruction, HM decoder and FFmpeg reference decoding.

Before the fix, PPS range-extension parsing failed, surfaced by decoder setup
as `HEVC PPS has no valid SPS`. `large_transform_skip_matches_oracle` accepts
all pairs and verifies every sample after decoder reset and playback rewind.
Normal tests use only the saved fixtures; no external decoder is invoked.

Generate separately with `scripts/generate_hevc_context_samples.py --large-skip`
using 3, 4 or 5, and the required HM encoder, decoder and configuration paths.
The flag sets the permitted size; it does not prove which sizes the encoder
actually selected. The block unit test forces a nonzero skip block at each
size. The transform unit test verifies that default nonflat scaling lists are
ignored for skip blocks larger than 4x4, as specified in H.265 8.6.3.

Cross-component prediction, chroma QP offset lists and other unsupported PPS
extension families remain explicit refusals.
