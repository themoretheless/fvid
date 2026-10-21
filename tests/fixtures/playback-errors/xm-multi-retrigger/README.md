# Owned XM Rxy multi-note retrigger

Regenerate explicitly: `python3 scripts/generate_xm_multi_retrigger.py`.
All 70 artifacts regenerate byte-identically with empty PATH and denied
subprocess/socket creation. Ordinary tests read saved files only, with no
FFmpeg, external codec, network or private media. The worker acceptance also
opens/decodes the owned VP9/PCM companion independently.

Thirty-four scores use the owned signed-16-bit loop and independent speed-1
scores of literal note/instrument/volume cells. All sixteen volume operations
have authored four-step level arrays. No Rxy code calculates expected levels,
trigger ticks or state resets. Common explicit speed-4 key-off rows equalize
release policy tails. Baseline volume-op-0 has equal PCM lengths but different
bytes at 11025 Hz mono: even no volume change still requires phase restarts.
The actual audio worker separately differs in its second equal-size packet.

Rxy retains separate nibble memories, a counter spanning rows, volume-table
clamping and direct volume/pan overrides on firing. Tick-zero counter stepping
depends on the volume-column handler's result, not merely its input byte. E9x,
EDx and instrument resets differ from bare E90/new notes without instruments.
Held cycles use the normal nonzero dispatcher even at local tick zero.
Reference: FT2 multiNoteRetrig, doMultiNoteRetrig and triggerInstrument:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

The initial volume-memory control incorrectly expected the transformed volume
past a direct-column override; it was corrected to the FT2 rule that direct
volume wins on every trigger. The original missing-Rxy baseline remains an
independent reproducible failure, and production override behavior was retained.

Acceptance covers sixteen transforms, period/volume memory, column/direct-zero/
fine-zero/pan-zero gates, instrument/E9x/EDx counter resets, bare E90 preserving
counter, held cycles, speed=1, empty history, independent channels and rewind
with initially empty arguments. Every PCM byte must match at 11025/44100 Hz,
mono/stereo, with no ignored features. Worker tests regroup independent control
PCM into source packet spans and check sequential, rewind, second-row seek and
EOF. Unsupported column effects/envelopes and absolute FT2 mixer parity remain
outside this qualification, as does complete codec coverage.
