# Owned XM Axy volume slides

Regenerate explicitly: `python3 scripts/generate_xm_volume_slide.py`.
Twenty-two artifacts regenerate byte-identically with empty PATH and denied
subprocess/socket creation. Ordinary tests only read saved files. No private
media, external codec, FFmpeg or network is needed. `video.mkv` is a short owned
VP9/PCM companion, checked by the audio-worker regression.

Ten input scores use the owned signed-16-bit looping ramp instrument. Independent
control scores expand every tick to a speed-1 row with a literal Cxx volume;
control values are authored in the generator, not computed by the tested mixer.
An explicit common speed-4 key-off row equalizes both end-of-score release tails.
This matters: comparing unequal automatic tail lengths would reproduce a timing
difference instead of the missing slide. The corrected baseline has equal PCM
lengths but differs on the first downward slide at 11025 Hz mono.

Coverage includes upward/downward slides, high-nibble priority with both nibbles
set, low/high clamping, A00 memory, empty initial memory, memory across blank
rows and new note triggers, channel-local memory, and speed-1 rows that execute
no nonzero tick and therefore cannot update slide memory. FT2 replayer source:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

The acceptance test compares every PCM byte at 11025/44100 Hz and mono/stereo,
including zero ignored-feature counters. A second test regroups independent
control PCM into the actual source packet spans and checks audio-worker rewind,
80-ms seek, EOF seek and audible endpoint. No slide implementation is used to
construct the expected PCM. This qualifies Axy only; volume-column slides,
combined portamento/vibrato slides, envelopes and other XM effects remain outside
this milestone.
