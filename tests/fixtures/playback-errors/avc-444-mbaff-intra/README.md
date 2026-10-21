# Owned nonseparate H.264 4:4:4 MBAFF intra

Generate separately with `python3 scripts/generate_avc_444_mbaff_intra.py`.
Run offline with `cargo test --offline --no-default-features --test avc_444_mbaff_intra`.
No FFmpeg, system decoder, network or private media is involved.

360 two-frame MP4 videos cross CAVLC/CABAC, 8/10/14-bit components,
frame/field/columns/checker pair layouts, I16 DC, I4 DC, I4 AC, I8 DC and PCM,
and deblocking modes 0/1/2. Owned High 4:4:4 Predictive SPS uses level 4.1,
32x64 coded geometry, nonseparate components and crop [1,1,2,2].
Each pair occupies one slice; both members share one field flag and common
macroblock modes. Y, Cb and Cr have independent residual contexts and samples.
Prediction across different slices is deliberately unavailable. Deblocking mode
0 crosses those boundaries; mode 2 preserves them but filters internal edges.

Compressed blocks use QP 26 and zero filter offsets. PCM uses QPY=0, Cb/Cr
QP offsets +12 and legal filter syntax offsets +6 (resolved +12), making chroma
filtering observable despite the zero luma QP. `witnesses.json` requires changed
samples for both enabled filter modes in every signal family. The I4 AC oracle
also requires the field scan orientation to affect samples. The native decoder
is compared sample by sample with scalar prediction/transform/filter oracles;
software MP4 reading, crop, timestamps, reset and rewind must match too.

Before the fix the first compressed CAVLC frame was refused by the PCM probe.
After shared residual support, a separate PCM reproduction still returned Cb
sample 3 = 100 instead of the independently filtered 101. Filtered PCM now uses
the complete component path; disabled-filter PCM retains its existing fast path
and exact-budget regression coverage.

The I4/I16 oracle reuses the owned scalar monochrome MBAFF authoring helper;
I8 uses the independent exact scalar 8x8 predictor. Scalar deblocking is shared
with the separate-plane fixture author and parameterized for 4x4/8x8 edges.
This is intra acceptance, not a claim of complete H.264 support: nonseparate
MBAFF inter and combinations beyond this matrix still need qualification.
