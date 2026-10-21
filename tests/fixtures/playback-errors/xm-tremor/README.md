# Owned XM Txy tremor

Regenerate explicitly: `python3 scripts/generate_xm_tremor.py`.
All 50 generated artifacts reproduce byte-for-byte with PATH empty and denied
subprocess/socket creation. Ordinary tests only read saved artifacts. No private
media or codec parameters, FFmpeg, external codecs or network are used.
The owned VP9/PCM companion is decoded independently; it does not contain XM.

Twenty-four scores use the owned signed-16-bit loop and independent speed-1
control scores containing literal Cxx volume writes at authored tick positions.
Expected on/off levels are written out, not calculated by Txy code. Common
explicit speed-4 key-off rows equalize release tails. Before the implementation,
phase-11 differed in PCM at 11025 Hz mono despite equal lengths; the actual
worker also differed in packet zero (28224 bytes on both sides).

Coverage includes zero/default and remembered parameters, asymmetric/maximum
phase lengths, parameter changes without phase reset, blank rows retaining
mute, new notes with/without instruments, instrument-only reset, E9x versus
E90, EDx, Rxy, zero fine/ordinary volume writes, panning/global volume preserving
mute, volume-column ordering, independent channels, held cycles and speed-one
commands not storing parameters. The reference behavior is FT2 tremor(),
triggerInstrument() and the ordinary tick dispatcher in:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

Every PCM byte matches its independent score at 11025/44100 Hz, mono/stereo,
with no ignored features. Actual audio-worker tests regroup the independent
PCM into source packet spans and check sequential output, rewind, second-row
seek and EOF. These tests qualify the native scheduling/mixing behavior, not
absolute FT2 mixer parity, physical audio output, envelopes or all XM effects.
