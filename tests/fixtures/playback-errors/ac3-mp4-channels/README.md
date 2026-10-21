# AC-3 MP4 channelcount placeholder

Regenerate separately: `python3 scripts/generate_ac3_mp4_channels.py`.
Three 48 kHz 5.1 syncframes from the repository's existing authored synthetic
tone are muxed with owned neutral VP9 video. The generator records the source
SHA256 and writes exact dac3 rate/bsid/mode/acmod/LFE/bitrate fields from the frame.
`placeholder.mp4` declares sample-entry channelcount=2; `native-six.mp4` declares
6. Both carry identical AC-3 samples. `control.ac3` preserves the three coded
frames for native PCM comparison. No private samples or parameter sets.

Before the fix the placeholder file opened as 2 channels instead of native 6
(`/tmp/fvid-ac3-mp4-channels-before.log`). The native MP4 reader now derives AC-3
output geometry and speaker mask from the syncframe, as it already did for E-AC-3.
`tests/ac3_mp4_channels.rs` compares every native PCM byte against the raw reader
after accounting for its 256-sample priming, compares both MP4 variants, checks
track metadata/mask/PTS/duration and captures worker rewind/40 ms partial seek.
This reuses the qualified AC-3 decoder, not a new independent AC-3 algorithm
oracle. No physical device, GUI or performance qualification is claimed.

Nine artifacts / 146,532 bytes excluding README regenerate byte-exactly with
subprocess/socket construction denied. Ordinary tests do not run generators,
invoke FFmpeg or require network. Other profile/container combinations remain
separate gates; this is not all-codec completion.

Matroska follow-up: `metadata-two.mkv` deliberately declares two channels while
its clean AC-3 frames contain 5.1; `native-six.mkv` is the correctly described
control. Both preserve the same coded samples and owned neutral video. The old
Matroska reader decoded the first as two channels
(`/tmp/fvid-ac3-matroska-channels-before.log`). Selected AC-3 channel count,
speaker mask and selected-track description now follow the syncframe. The
shared container regression compares all four MP4/Matroska variants with raw
native PCM, checks exact clocks, and captures rewind/40 ms partial seek. Explicit
codec downmix remains supported; unselected Matroska descriptions still use
container metadata until those tracks are opened.

CRC-invalid first-header regression: `bad-first-header.ac3/.mp4/.mkv` changes
only the first frame acmod to mono without repairing CRC. Correct container
metadata and two subsequent clean 5.1 frames remain unchanged. The old automatic
probe replaced six channels with one (`/tmp/fvid-ac3-damaged-geometry-before.log`).
Container setup now uses `ac3_decoder::checked_syncframe`, which requires a full
frame passing the same CRC check as the decoder before trusting geometry.
If no trusted first header exists, original container channels are retained and
speaker roles remain unknown rather than copied from damaged bits. Tests require
first-frame silence and byte-exact subsequent six-channel PCM against an explicitly
configured native decoder, plus reader/worker rewind and partial seek. Ordinary
lightweight `syncframe` remains available for packet walking and concealment.
This does not infer native layout from later frames or repair contradictory
container metadata when the first coded header is damaged.
