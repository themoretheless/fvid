# Owned XM E9x sample retrigger

Regenerate explicitly: `python3 scripts/generate_xm_retrigger.py`.
All 46 artifacts regenerate byte-identically with empty PATH and denied
subprocess/socket creation. Ordinary Rust tests read saved artifacts only;
no private media, network, FFmpeg or external encoder is used. The owned
VP9/PCM companion is separately decoded by the audio-worker test.

Twenty-two scores use the owned signed-16-bit ramp: forward loop, unlooped and
ping-pong controls. Independent speed-1 scores explicitly state the note or hold
at every tick. Expected restart ticks are authored literal arrays, not calculated
by the E9x implementation. Row-zero instrument/volume and supported 8xx/9xx
commands are retained in controls; E9x and original speed commands are absent.
Common speed-4 key-off rows equalize policy release tails. Baseline every-tick
has equal PCM lengths but different bytes at 11025 Hz mono; the actual audio
worker also differs in its first equal-size packet.

E91..E9F execute on nonzero ticks divisible by their argument. E90 restarts a
remembered note only on tick zero (or plays the explicit new note); it is not
interval memory. E90 with key-off remains key-off. Empty note history is a no-op.
Repeating a note reselects the current instrument's keymap/pitch and restarts
phase/direction without restoring channel volume or pan. Previous 9xx sample
offset is not reapplied. Reference: FT2 retrigNote, getNewNote and triggerNote:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

Coverage includes intervals 1/2/3/15, no extra tick-zero restart for nonzero E9x,
E90 distinction, new pitches, empty history, speed=1, volume/pan retention,
offset reset, naturally ended samples, ping-pong direction, independent channels,
key-off and instrument-only rows. Two extra controls select a second instrument
with relative note +12, qualifying instrument/pitch reselection rather than just
cursor reset. PCM must exactly match at 11025/44100 Hz, mono/stereo, with no
ignored features. Worker comparisons regroup independent note-control PCM into
source packet spans and require sequential, rewind, second-row seek and EOF.
Envelopes, remaining effects and absolute FT2 mixer parity remain unqualified.
