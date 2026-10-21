# Owned XM volume-column slides

Regenerate explicitly: `python3 scripts/generate_xm_column_volume_slide.py`.
All 32 artifacts regenerate byte-identically with PATH empty and subprocess /
socket creation denied. Ordinary Rust tests only read saved artifacts; no
FFmpeg, network, external encoder or private media is used. The original Axy
fixture generator's 22 artifacts also remain byte-identical.

Fifteen authored scores use the owned signed-16-bit loop instrument. Independent
speed-1 Cxx scores specify literal expected volume at every tick. They cover
6x down / 7x up, zero parameters without effect memory, clamping, fine 8x/9x
controls on tick zero, Cxx on tick zero before later column slides, ordering
against Axy and delayed ECx/Kxx, independent A00 memory, speed=1, and two channels.
The implementation does not compute the control tick values.

Before the fix, column-down has equal PCM lengths but different PCM bytes at
11025 Hz mono; the actual audio worker also produces different bytes in its
first packet. These failures identify the omitted column slide rather than an
unrelated timing failure. The post-fix direct test requires every PCM byte to
match at 11025/44100 Hz, mono/stereo, and no ignored-feature counters. Fine 8x/9x
were already implemented and are acceptance controls, not newly added behavior.
FT2 applies the nonzero-tick volume column before ordinary effects:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

The worker regression regroups independent tick PCM into source packet spans,
then checks sequential playback, rewind, seek to the second source row and EOF.
The second row starts at 20 ms for the speed=1 control and 80 ms otherwise;
source seeks intentionally land at row boundaries. An owned VP9/PCM video
companion is opened and decoded independently. Remaining vibrato, panning-slide,
portamento and envelope semantics are not qualified by this milestone.
