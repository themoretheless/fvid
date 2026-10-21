# Owned XM tone portamento acceptance

Regenerate explicitly: `python3 scripts/generate_xm_tone_portamento.py`.
All 58 artifacts regenerate byte-identically with PATH empty and denied
subprocess/socket creation. Ordinary tests read saved files only; no FFmpeg,
external codec or network is needed. No private samples, video or parameter
sets are used. The owned VP9/PCM companion is decoded separately; it contains
no XM stream.

Fifty-six linear/Amiga scores have independently authored literal periods,
volume targets and phase-reset positions. The reference ramp renderer knows no
3xx/5xx/Fx commands, parameter memories or scheduling rules. It uses the existing
native linear/Amiga pitch profile, rather than claiming absolute FT2 mixer/table
parity. A second independent trace qualifies per-channel target/speed state.
Before implementation, up-linear produced different PCM at 11025 Hz mono with
the same length. The actual audio worker differed in equal-size packet1
(28224 bytes each), isolating the missing slide/phase behavior.

Coverage includes upward/downward target arrival and clamping, speed memory,
blank-row pause, target reversal, equal target, empty speed/history, instrument
selection retaining active tuning, original-note recall through E9/ED, full
column speed, shared main/column speed, column preparation priority, two steps
per tick when combined, Axy/5xy shared volume memory, held cycles, speed-one
storing preparation, key-off with continued sample phase, independent channels,
and column E90/ED/K00/pitch-slide ordering.

Reference semantics: preparePortamento, getNewNote, portamento and
portamentoPlusVolSlide in:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c
The initial controls had an Amiga note57 target rounded to1079 rather than1078;
and a linear ED trace incorrectly resumed the former upward direction after
arrival. Those literal controls were corrected to the native pitch law and FT2
persistent-direction behavior, without changing production to fit the errors.

Every PCM byte must match at 11025/44100 Hz mono/stereo, with no ignored
features. The actual worker must match independent PCM for sequential output,
rewind, second-row seek and EOF. These use a mock sink, not physical speaker
acceptance. Glissando E3x, vibrato/arpeggio, envelopes, exact FT2 pitch tables
and overall codec coverage remain unqualified.
