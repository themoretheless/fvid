Synthetic finite HLS fMP4 manifests, split from the committed `fragmented/video.mp4`
(25 frames, five fragments). No private media is used.

Regenerate with `python3 scripts/generate_hls_fixtures.py`; no FFmpeg or network.
`media.m3u8` uses separate resources; `byterange.m3u8` uses explicit init/first
segment offsets and implicit subsequent offsets in one object. Assembly tests
compare every packet and timestamp against the original fragmented MP4.
These tests qualify parsing and assembly, not HTTP playback, live HLS, encrypted
HLS, MPEG-TS, or alternate rendition selection.
