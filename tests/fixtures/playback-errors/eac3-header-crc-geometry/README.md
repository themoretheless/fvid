# CRC-invalid E-AC-3 first-header geometry

Regenerate separately: `python3 scripts/generate_eac3_header_crc_geometry.py`.
Three owned six-block 48 kHz stereo+LFE frames are authored. The first frame's
acmod is changed to mono+LFE without repairing CRC; two clean three-channel
frames follow. Correct three-channel container metadata is preserved in neutral
VP9/MP4 and Matroska videos. Standalone damaged and clean bundles are also saved.
No private input, encoder or network.

Before the fix, MP4 setup trusted the damaged header and advertised two channels
(`/tmp/fvid-eac3-crc-geometry-before.log`). Automatic setup now uses
`eac3_decoder::checked_frame`, requiring a complete first frame passing the same
CRC criterion as decode. On failure the original container channels remain and
speaker roles are unknown rather than derived from damaged bits. Lightweight
`frame` probing and ordinary decoder concealment are unchanged.

The regression proves the damaged syntax says two channels, the checked probe
refuses it, and a clean control is accepted. Both containers retain three
channels, emit silence for the bad frame, and preserve following PCM against an
explicitly configured native decoder after its raw priming is accounted for.
Reader/worker rewind and 40 ms partial-sample seek are checked byte-exactly.
This is a first-header/geometry and concealment regression, not a new independent
codec oracle or physical-device qualification. Later-frame layout inference,
contradictory container metadata, and corrupted dependent channel maps remain
separate concerns.

Four artifacts / 83,548 bytes excluding README reproduce exactly with subprocess
and socket creation denied. Ordinary tests do not regenerate, invoke FFmpeg or
use network. This does not establish all-codec completion.
