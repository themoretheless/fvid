# Owned multichannel WavPack regression fixtures

Generate separately with `python3 scripts/generate_wavpack_multichannel.py`.
Ordinary tests read checked-in artifacts, without FFmpeg, network or private data.

The manifest contains 115 accepted 60 ms VP9/WavPack videos. The matrix covers
3/6/8 channels, paired and mono components, false stereo, 8/16/24/32-bit integers,
float and optimized integer extra bits, and legacy/new channel metadata. Extra
cases cover short legacy metadata, 20/256/4096 channels and 40-bit block indices
and total sample counters. Metadata payload lengths include 1/2/3/5/6/7 bytes.

Each frame has raw full-header `.wv` and standard compact Matroska `.packet`
representations. Independent standalone components and scalar interleaved PCM
are controls. Tests verify every channel, component CRC, reset, speaker masks,
neutral video, EOF, rewind, sample-exact partial-block seek, headless audio-worker
capture and software stereo routing. This is not a physical-device qualification.

Fourteen malformed raw frames target sequence, geometry, sample windows, total
counts, format, channel metadata and CRC errors. Five `raw_only` cases exercise
fields that Matroska compact mapping removes: their movies are valid normalization
controls, not malformed-video acceptance claims. Four separate mapping cases
exercise truncation, overrun, trailing bytes and unsupported stream version.
Layout-change and failed-last-CRC controls verify stream binding across seek/reset
and atomic publication of layout and PCM.

The generator returns 8,988 artifacts totaling 49,843,973 bytes (excluding this
README). Hybrid/correction streams, DSD, custom channel identities and other
unqualified WavPack metadata/tools remain outside this finite acceptance matrix.
