# Owned XM Lxx envelope-position acceptance

Regenerate explicitly: `python3 scripts/generate_xm_envelope_position.py`.
All 71 files regenerate byte-identically with PATH empty and denied
subprocess/socket creation. All 65 preceding envelope artifacts remain identical.
Ordinary tests read saved files only and need no FFmpeg, codec tool or network.
Samples/parameters are authored, with no private media. The independently decoded
owned VP9/PCM companion contains no XM stream.

Sixty-four linear/Amiga scores supply literal post-position tick gains, pans and
sample-reset positions. The independent ramp renderer interprets no Lxx commands,
segment search, interpolation accumulator, sustain or loop algorithms. Before
implementation, mid-segment-linear differed at 11025 Hz mono with equal PCM
lengths; actual worker packet0 also differed (28224 bytes each). Four omitted-Lxx
controls separately match independent unpositioned gain/pan traces exactly and
differ from intended output at equal lengths.

Coverage includes zero/no-memory behavior, exact/interior/last/beyond-last
positions, single-point envelopes, forward/backward motion without restarting
the sample, loop interior/end, sustain landing/bypass/release, fractional
interpolation, positions before the first node, volume-sustain-gated pan movement,
disabled-volume sustain bit behavior, pan loops, held rows executing setup only
once, speed1, prior key-off/fade not being rearmed, new-note/instrument ordering,
active versus selected instrument ownership, independent channels, empty history
and disabled envelopes.

Reference semantics: setEnvelopePos, getNewNote and updateVolPanAutoVib in:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c
In particular the pan gate intentionally tests the volume sustain flag, and a
position before the first point retains the reference's signed elapsed behavior.
Wide intermediates and bounded point access avoid reproducing undefined memory
or arithmetic behavior on malformed input. The separate zero-point enabled
shape test reports one unrenderable envelope; it is not playback acceptance of
invalid metadata.

Every acceptance PCM byte matches at 11025/44100 Hz mono/stereo, with no ignored
features. Actual worker tests compare independent PCM sequentially, after rewind,
second-row seek and EOF through a mock sink. Physical speaker acceptance,
absolute FT2 table/mixer parity, other unsupported behavior and complete codec
coverage remain unqualified.
