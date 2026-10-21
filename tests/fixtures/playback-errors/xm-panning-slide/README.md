# Owned XM Pxy and column Dx/Ex panning slides

Regenerate explicitly: `python3 scripts/generate_xm_panning_slide.py`.
All 50 artifacts regenerate byte-identically with PATH empty and subprocess /
socket creation denied. Ordinary tests only read saved files; no external codec,
FFmpeg, network or private media is needed. An owned VP9/PCM companion is also
opened and decoded by the independent audio-worker check.

Twenty-four authored scores use the owned signed-16-bit looping ramp. Control
scores expand each tick into a speed-1 row with literal absolute 8xx placement.
Input sample volumes remain fixed at full scale; the A-memory control saturates
upward, so no unmodeled local-volume changes affect the pan oracle. Expected
placements are authored independently of the tested slide code. Common explicit
speed-4 key-off rows equalize end-of-score release policy tails.

Before the fix, separate direct tests fail P-left and column-left at 11025 Hz
stereo, with equal PCM lengths but different bytes. The worker independently
fails its first equal-size packet. Acceptance compares every PCM byte at
11025/44100 Hz, stereo/mono, and requires zero ignored features. Mono additionally
checks that pan slides do not alter the dry mix. Worker comparisons regroup
independent 8xx PCM into source packet spans and cover sequential playback,
rewind, second-row seek and EOF seek.

Pxy executes on nonzero ticks. P00 remembers the last executed parameter per
channel, independent of Axy and column effects. The high nibble moves right and
wins when both are nonzero; the low nibble moves left. Placements clamp at 0/255.
Column Dx/Ex applies before Pxy, without P00-style memory. FT2's D0 byte-wrap
quirk moves completely left rather than doing nothing; E0 is a no-op. The source
reference is panningSlide, v_PanSlideLeft/Right and per-channel dispatch:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

Coverage includes direction, saturation, both nibbles, P00, empty/reset memory,
8xx retaining memory, speed=1 not storing/executing, retrigger, silent-channel
memory, independent channels, direction-changing memory, Dx/Ex, D0/E0, column/P
clipping order, Cx on tick zero and Axy memory independence. This does not qualify
envelopes, other effects, absolute FT2 mixer parity or complete codec coverage.
