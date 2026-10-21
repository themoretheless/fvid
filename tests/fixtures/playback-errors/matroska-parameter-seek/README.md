# Matroska AVC/HEVC received-parameter seek history

Generate separately with `python3 scripts/generate_matroska_parameter_seek.py`.
The generator wraps only existing owned synthetic AVC/HEVC source movies.
AVC packed sample oracles are computed from the independently authored PCM
values, including monochrome neutral chroma and mixed-depth normalization.
HEVC oracles split the original owned source YUV. No FFmpeg, external codec,
network, private video, frames, audio or parameter sets are used.

`manifest.json` contains 36 playback acceptance movies: the 12 AVC and six HEVC
parameter-order layouts, each with or without a companion UTF-8 subtitle track.
Each source has actual old-sequence pictures, same-ID parameter replacement,
and an independently decodable new IDR. Parameters are inline with that IDR or
in a preceding parameter-only block. Individual blocks occupy separate clusters
to exercise lazy index scanning. Exactly two real video keyframes are indexed;
the parameter-only block is never falsely marked as a keyframe. Subtitle blocks
contain bytes which must not be passed to either video decoder.

The sequential and rewind control compares every component sample, depth,
geometry and interval for all 36 movies. Two distinct original failures after
seek to the new IDR were:

- `avc-y8-c8-sps-pps-prefix.mkv`: `invalid intra macroblock type`.
- `hevc-32-33-34-prefix.mkv`: `missing HEVC header alignment bit`.

Both same movies already played correctly sequentially. Reset during seek lost
the parameters received in earlier blocks. Acceptance checks fresh-reader seek,
seek after old-frame playback, repeated seek, rewind, EOF and complete pixels
and intervals of the new sequence. A metadata block extends the preceding old
picture's displayed interval; it does not emit an extra image.

`controls.json` separately lists four damaged-stream controls:

- Two `skip-pictures` movies must refuse sequential decoding of truncated old
  VCL pictures but accept seek to the independent new IDR with exact new samples.
  This proves parameter restoration does not decode old picture entropy; it is
  not full playback acceptance of a malformed stream.
- Two `refuse-parameters` movies contain a PPS with valid IDs and missing syntax.
  Parameter restoration must refuse seek, leave reading in a failed state until
  rewind, and restore the initial valid old frame after rewind. Sequential
  reception of that malformed update also refuses. No malformed PPS playback
  support is claimed.

Six Rust tests live in `tests/matroska_parameter_seek.rs`. The 60 generated
artifacts regenerate byte-identically. The README is not a generated artifact.
The fix restores selected-track parameter state under existing budgets, without
decoding or retaining old pictures. Hardware decoder transitions, CodecState
changes and arbitrary cross-packet pending replacement remain separate scopes.
