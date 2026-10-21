# 3D Main depth-layer chroma constraints

Regenerate separately: `python3 scripts/generate_hevc_3d_chroma_constraints.py`.
Four explicitly authored VPS/SPS configurations declare a dependent depth layer
with effective profile 8 and chroma_format_idc 0/1/2/3. Only 0 conforms to
H.265 I.11.1.1. Three coloured variants must produce Invalid, specifically
`HEVC 3D Main depth layer must be monochrome`, before picture-tool dispatch.
The old parser accepted them (`/tmp/fvid-hevc-depth-chroma-before.log`).

Four short MP4 files embed these parameters using the existing public synthetic
HEVC seed. Tests read each MP4 hvcC, verify its SPS matches the authored standalone
SPS and parse that exact configured SPS with the authored VPS/layer context.
These are parameter refusal/acceptance witnesses, not intended picture-playback
acceptance: their original seed pictures are not a claimed scalable sequence.
No private media/parameter sets, external process or network is used. Fourteen
artifacts / 60,291 bytes excluding README regenerate exactly with subprocess and
socket construction denied. Ordinary tests do not regenerate or invoke FFmpeg.

Normative basis: ITU-T H.265 V11 (01/2026), I.11.1.1, 3D Main profile:
https://www.itu.int/rec/T-REC-H.265-202601-I/en
Its depth-layer active SPS restriction requires chroma_format_idc=0. This is a
profile violation, not evidence that conforming colour depth playback is missing.
Other profiles/tool bindings are not qualified by this check. Existing actual
monochrome DIM/skip-intra picture tests remain separate positive gates.
