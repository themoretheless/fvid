# Fixed MP4 time-header lengths

Pure Python-authored 16x16 neutral VP9, three independent 20ms frames.
V0/V1 controls have complete mvhd (100/112-byte payload) and mdhd (24/36).
Twelve companions omit the final 1/4 bytes or append one byte of exactly one
header; packet bytes, durations and timescales are identical. Generated offline
by scripts/generate_mp4_time_header_sizes.py, with no private input or FFmpeg.
Tests distinguish refusal of malformed headers from successful 60ms RGB playback,
exact packet clocks, seek, rewind, EOF and packet-budget enforcement.
