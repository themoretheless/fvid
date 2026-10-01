# HEVC range-extension residual rotation

The owned decoder implements `transform_skip_rotation_enabled_flag`. For an
intra 4x4 transform block, skip or transquant-bypass residual samples rotate
180 degrees. Ordinary inverse DCT/DST blocks, inter blocks and larger blocks
keep their existing order. The old public residual-decoding entry point retains
its default behavior; full HEVC decoding obtains the switch from SPS.

The normative rules are H.265 clause 8.6.2, especially equations 8-297 and 8-298:
[ITU-T H.265 V10 (2024)](https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.265-202407-S%21%21PDF-E&lang=e&type=items).
Rotation occurs after scaling in the skip case and without scaling in bypass.
The implementation therefore reverses reconstructed skip/bypass residual arrays,
preserving position-dependent scaling-list weights before rotation.

## Acceptance fixtures and independent reference

The fixture generator creates eight three-frame 64x64 synthetic MP4 streams:
8/10-bit 4:2:0, skip/bypass, and rotation enabled/disabled. It uses `testsrc2`,
libx265 with `tskip=1:max-tu-size=4`, and `lossless=1` for bypass. Loop filters
are disabled. VPS/SPS profile signalling is rewritten to RExt with no Main/Main10
compatibility claim, and only the rotation flag is set in SPS extensions.

Generate separately from ordinary tests:

```sh
python3 scripts/generate_hevc_smoothing_samples.py --rotation --hm-decoder /path/to/TAppDecoder
```

Saved YUV oracles come from unmodified HM 18.0 decoder algorithms. Reference
source archive:
[HM-18.0](https://vcgit.hhi.fraunhofer.de/jvet/HM/-/archive/HM-18.0/HM-HM-18.0.tar.gz),
SHA-256 `c91b0f678cf8a77599cfcd72484c2ef1c10cbbf3850982fb7d43f2b54c3edfd8`.
For the local macOS reference build, CMake used Release/x86_64 (the upstream
build assumes SSE), and its warnings-as-errors policy was disabled for modern
Clang warnings. No decoder algorithm was changed. Decoder options explicitly
set output luma/chroma depth and disable comparison against the original
encoder's decoded-picture hash, since parameter signalling is intentionally
changed after encoding.

The generator verifies that HM and FFmpeg agree for skip and unrotated bypass,
that their rotated-bypass outputs differ, and that each enabled/disabled pair
has different reconstructed samples. FFmpeg's current
[residual decoder](https://ffmpeg.org/doxygen/trunk/hevc_2cabac_8c_source.html)
rotates skip coefficients in its non-bypass branch; its output is not used as
the rotated-bypass oracle. This distinction prevents a refusal-only or
inactive-tool fixture from being mistaken for acceptance.

`tests/hevc_rext_smoothing.rs` compares every sample through direct decoding
and software MP4 playback, then repeats after reset/rewind. It verifies profile,
depth, SPS switch and PPS skip/bypass signalling. A CABAC impulse unit test also
checks rotation of skip/bypass intra blocks and unchanged inter samples.
Before the fix, the enabled skip fixture reproduced specifically
`remaining HEVC SPS range-extension tools are not implemented`.
Ordinary tests invoke neither reference decoder nor FFmpeg and use no network.

Other RExt tools and unsupported chroma formats remain incomplete; this does
not claim complete range-extension profile support.
