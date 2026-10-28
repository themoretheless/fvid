# Unknown MP4 media duration

Authored separately by `scripts/generate_mp4_unknown_duration.py`, using owned
16×16 neutral VP9 frames. No external codec, FFmpeg, network or private media.
All movies contain three frames, with timescale 50: exactly 60 milliseconds.

Each regular/fragmented source has a known-duration control and version-0 and
version-1 `mdhd` companions whose duration field is all ones. This marker means
unknown, not a numeric duration (ISO/IEC 14496-12:2015 §8.4.2.3).
The reader can recover the duration from the complete sample index. Tests check
both parser wrappers, exact packet bytes, probe duration, native RGB, seek,
rewind and repeated EOF. These are valid-input acceptance cases.

Reference: https://hezhaojiang.github.io/slave/MP4/ISO-IEC-14496-12-Base-Format-2015.pdf
