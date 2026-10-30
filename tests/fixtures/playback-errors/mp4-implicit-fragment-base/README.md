# Implicit fragmented MP4 data bases

Owned deterministic neutral VP9 (16x16, 50Hz), three frames per represented
track; two-moof witness has six. One explicit-moof control and eight implicit
base witnesses cover first-traf/moof origin, preceding-traf data end, absolute
and moof resets, ignored foreign tracks with default or per-row sizes, and
resetting the inherited base at a new moof. Foreign bytes are literal dummy
ASCII and are never decoded. No private inputs, FFmpeg or network.

Negative witnesses: media outside mdat, undeclared foreign sizes before an
implicit base, and a 0xffffffff default-sized foreign sample count exceeding
mdat. They test specific refusal before decoding; valid movies test actual
software playback, exact pixels/clocks, seek, rewind, EOF and budgets.

Generate separately: scripts/generate_mp4_implicit_fragment_base.py.
Ordinary tests read checked-in files only. Expected offsets are absolute file
positions authored in manifest.json, independent of parser-derived offsets.

Reference: https://www.w3.org/TR/mse-byte-stream-format-isobmff/#media-segments
includes the single-traf implicit-moof addressing alternative.
