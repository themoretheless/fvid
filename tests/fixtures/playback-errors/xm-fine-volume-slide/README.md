# Owned XM EAx / EBx fine volume slides

Regenerate explicitly with `python3 scripts/generate_xm_fine_volume_slide.py`.
All 30 artifacts regenerate byte-identically with PATH empty and subprocess /
socket creation denied. Ordinary tests only read saved files, with no FFmpeg,
network, external codec or private media. The companion `video.mkv` is owned
synthetic VP9/PCM and is independently opened/decoded by the worker test.

Fourteen input scores use the owned signed-16-bit looping ramp. Independent
speed-1 Cxx scores state literal expected volume at every tick; the tested fine
slide code does not generate those values. End-of-score release tails have an
explicit common speed-4 key-off row, avoiding unrelated length differences.
Baseline up-memory has equal lengths but different PCM at 11025 Hz mono, and
worker generation zero also differs in its first packet.

EAx / EBx execute once on tick zero. Their memories are channel-local, separate
for up/down and independent of Axy and column fine slides. Zero arguments reuse
their own direction's last nonzero argument. Volume clamps to 0/64. The column
executes first on tick zero; ordinary column slides begin on nonzero ticks.
These rules follow the FT2 replayer's fineVolSlideUp/fineVolFineDown and dispatch:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

Cases cover up/down memory, independent directions, empty memories, clamping,
column/fine clipping order, ordinary column ticks after fine, independent A00
memory, column commands not storing fine memory, retrigger, channel locality
and speed=1 (fine slides still execute/store on tick zero). Direct acceptance
compares every PCM byte at 11025/44100 Hz in mono/stereo and requires no ignored
features. The worker test regroups independently rendered Cxx PCM into source
packet spans, then verifies sequential playback, rewind, second-row seek and
EOF seek. Other effects, envelopes and complete XM fidelity remain unqualified.
