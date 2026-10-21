# Sample-grid origins around CodecDelay

Generate separately with `python3 scripts/generate_matroska_preroll_rounding_regressions.py`.
Generation authors short VP9/PCM videos and uses only existing owned synthetic
Opus/Vorbis controls. Tests use saved bytes without FFmpeg, tools or network.

An exact sample timestamp can occupy either adjacent integer nanosecond in
container metadata. A sample-aligned origin and CodecDelay must cancel even
when their stored representations differ by one nanosecond. A negative
sample-aligned origin must trim the known sample count, preserving its first
non-padding neighbour. The original 44.1 kHz same-sample-origin and negative-grid
controls reproduced one missing useful sample; matching 48 kHz videos now
cover the other adjacent representation too.

Physical sample-origin recovery admits only those adjacent nanosecond values,
not a whole container tick. Strict off-grid negative origins and explicit
positive seek floors still round the requested boundary up. Source packet
timestamps retain literal CodecDelay subtraction.

The suite compares independently authored complete source and retained sample
bytes, video playback, worker playback/seek, known Opus/Vorbis preroll, and
overflow refusal. It distinguishes entirely inaudible source windows from a
malformed stream. All 46 generated artifacts regenerate byte-identically; the
39 prior non-manifest artifacts were unchanged by the two added 48 kHz cases.
