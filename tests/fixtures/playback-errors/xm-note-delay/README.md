# Owned XM EDx delayed note commands

Regenerate explicitly: `python3 scripts/generate_xm_note_delay.py`.
All 46 artifacts regenerate byte-identically with PATH empty and denied
subprocess/socket creation. Ordinary tests read saved artifacts only. No private
media, FFmpeg, network or external codec is used. The owned VP9/PCM companion
is also opened and decoded independently in the worker acceptance test.

Twenty-two scores use an owned signed-16-bit looping ramp. Independent controls
state complete literal note/instrument/volume/effect cells on speed-1 tick rows.
The EDx implementation does not calculate control deadlines or expected cells.
Common explicit speed-4 key-off rows equalize automatic release policy tails.
Baseline delay-two differs from these controls at 11025 Hz mono with equal PCM
lengths, and the actual audio worker differs in its first equal-size packet.

ED1..EDF selects an instrument on tick zero but defers the note, instrument
sample defaults and direct column volume/pan until the named nonzero tick.
A delay at or beyond the row speed never triggers and does not carry into the
next row. ED0 is ordinary tick-zero handling, not E90-style restart. A delayed
zero note recalls the remembered note; empty history stays silent. Ordinary
column slides still execute before the deadline trigger, while 8x/9x fine column
commands skipped at tick zero are not replayed at the deadline. Unknown column
features remain counted rather than silently hidden by delayed handling.
Reference: FT2 getNewNote early selection and noteDelay dispatch:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

Coverage includes ticks 1/2/3, 15 both invalid at speed=4 and valid at speed=16,
no carry, ED0 new/no-note cases, old sample continuation, recalled note, empty
history, direct volume/pan delay, skipped fine commands, column/default ordering,
key-off, channel-local deadlines and speed=1. Two instrument-selection controls
use a second owned instrument with relative note +12, including selection that
survives a canceled note. Acceptance compares every PCM byte at 11025/44100 Hz,
mono/stereo, requires zero ignored features, and verifies worker sequential,
rewind, second-row seek and EOF seek against independent control PCM regrouped
into actual source packet spans. Remaining effects, envelopes, absolute FT2 mixer
parity and overall codec completion remain outside this qualification.
