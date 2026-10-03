# Matroska lacing regression controls

Generate with `python3 scripts/generate_matroska_lacing_fixtures.py`. The input
FFV1 gray packets and pixel goldens are synthetic, committed FVid encoder
outputs; regenerate those with `generate_ffv1_gray_fixtures.py`. Neither
fixture generator invokes FFmpeg or reads private source media.

`matroska-lace-{xiph,ebml,fixed}.mkv` contain four valid FFV1 frames. The
acceptance tests verify exact packet bytes/offsets, decoded pixels, frame
clocks, arbitrary packet reads, interval decoding and public owned dispatch.
BlockGroup fixtures verify fractional per-frame duration and that negative
padding affects the first frame and positive padding the last.

`matroska-lace-delay.mkv` reproduces the probe CodecDelay omission: the
presentation start must be -5 ms while the encoded packet clock remains +5 ms.
The regression failed with +5 ms before the probe correction.

The invalid-size fixtures are malformed-container refusal tests.
`matroska-lace-no-clock.mkv` is a **capability refusal**, not a playback
acceptance: FFV1 codec-derived lace duration without DefaultDuration/BlockDuration
still needs implementation. Content encoding is also not implemented here.

Ordinary `matroska_lacing` tests require no external codec executable. The
separate optional `ffmpeg_lacing_reference` benchmark compares decoded pixels
with FFmpeg. Lacing follows https://www.matroska.org/technical/notes.html.

Generate the Opus controls with `python3 scripts/generate_opus_lacing_fixtures.py`.
Literal synthetic DTX packets exercise 20/40/10/20 ms codec durations in Xiph
and EBML laces without DefaultDuration. Explicit BlockDuration takes precedence
and distributes 90 ms equally across four packets. Tests verify both reader
implementations and public probe dispatch; invalid packet framing is a malformed
input refusal, not an unsupported-codec fallback. These are timing acceptance
tests, not acceptance of an Opus PCM decoder. The optional reference benchmark
checks container ticks and independently decodes 4320 samples per valid fixture.
