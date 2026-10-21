# Owned XM envelopes and fadeout qualification

Regenerate explicitly: `python3 scripts/generate_xm_envelopes.py`.
All 65 files regenerate byte-identically with PATH empty and denied
subprocess/socket creation; all 60 preceding auto-vibrato artifacts remain
identical. Ordinary tests only read saved files, without FFmpeg, codec tools or
network. Samples/parameters are authored, never copied from private media. The
owned VP9/PCM companion is decoded separately and contains no XM stream.

Fifty-six linear/Amiga acceptance scores provide literal final tick gains,
panning and phase-reset positions. Their independent ramp renderer knows no
point interpolation, sustain/loop flags, cursor rules or fadeout algorithm.
Parser checks assert point count/data, flags, sustain/loop indices and fadeout.
Six disabled-feature controls match independent unshaped/unfaded/centered PCM
and differ from intended output at equal lengths. Before implementation,
volume-fall-linear differed at 11025 Hz mono despite equal PCM lengths; worker
packet0 also differed (28224 bytes each).

Coverage includes rise/fall, fixed-point truncation, sustain release, loops and
sustain-at-loop-end behavior, panning envelopes at center/off-center/full scale,
combined volume/pan, quarter-step and oversized fadeout, note/instrument-only/
bare-note/E9/ED reset distinctions, delayed first-note activation, channel volume
and slides multiplying the envelope, independent instrument owners, Kxx release
versus ECx cut, non-revival after completed fade without an instrument reset,
and bounded duplicate point ticks. Reference pan full-scale wrap and sustained
pan key-off behavior are explicit compatibility cases.

The additional unrenderable-loop-index score is a reporting/safety test, not
acceptance of that invalid envelope: decoding stays bounded and Ignored reports
one unrenderable envelope. Enabled zero-length/out-of-range point metadata also
remains reported rather than silently counted as supported.

Reference semantics: triggerInstrument, keyOff and updateVolPanAutoVib in:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c
The native computed pitch and smoothing profile is retained. Absolute FT2 table
and mixer parity is not established.

Every acceptance PCM byte matches at 11025/44100 Hz mono/stereo with no ignored
features. Worker tests compare independent PCM sequentially, after rewind,
second-row seek and EOF through a mock sink. Physical speaker acceptance, Lxx
envelope positioning and complete codec coverage remain unqualified.
