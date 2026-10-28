# MP4 time-header FullBox flags

Separately authored by `scripts/generate_mp4_time_header_flags.py`: three owned
16×16 neutral VP9 frames at 50 Hz, 60ms total. No FFmpeg, network or private media.

ISO/IEC 14496-12 defines `mvhd` and `mdhd` as FullBox(version, 0). The version-0
and version-1 controls have zero flags. Eight malformed companions explicitly
set the low or high reserved flag bit in one time header; all other fields,
packet bytes, sample indexes and media offsets are unchanged.

The refusal tests require the specific invalid time-header-flags error from
both parsers and before native decoder admission. Separate acceptance tests
check both healthy versions: packet bytes and limits, clocks, exact RGB,
seeking, rewind and repeated EOF. Refusal is not counted as valid playback.

Reference: ISO/IEC 14496-12:2015 §§8.2.2.2, 8.4.2.2.
