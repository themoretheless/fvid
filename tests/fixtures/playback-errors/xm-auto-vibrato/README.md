# Owned instrument auto-vibrato acceptance

Regenerate explicitly: `python3 scripts/generate_xm_auto_vibrato.py`.
All 60 files regenerate byte-identically with PATH empty and denied
subprocess/socket creation; all 86 previous vibrato/tremolo files remain
identical. Ordinary tests only read saved fixtures and need no FFmpeg, codec
tool or network. Samples/parameters are authored, with no private media. The
independently decoded owned VP9/PCM companion contains no XM stream.

Fifty-four linear/Amiga scores supply literal final periods, volume targets and
sample reset positions. The reference renderer interprets no instrument wave,
sweep, phase or pattern-effect commands. Parser checks independently assert all
four parameter bytes for both instrument owners. Four depth-zero controls match
unmodulated reference PCM exactly and differ from intended modulation at equal
lengths. Before implementation, sine-linear differed at 11025 Hz mono with equal
PCM length; worker packet0 also differed (28224 bytes each).

Coverage includes sine/square/both ramps, unknown-type sine fallback, zero depth,
zero rate, half-cycle wrap, sweep integer division and strict cap/equality rules,
key-off and volume restore, instrument versus bare-note and instrument-only
reset, E9/ED/R distinctions, manual no-reset flags, held cycles, speed1,
independent instrument owners/channels, old-amplitude retention on a bare-note
instrument change, and combination with manual vibrato/arpeggio/pitch slide.
The separate final period never feeds back into the pattern's output period.

The delayed-first-note case independently exposed a second bug: EDx started the
first automatic voice, but the next tick was not dispatched. Before that fix,
its equal-length PCM differed directly and in worker packet0. Acceptance now
requires all subsequent auto-vibrato ticks after the delayed onset.

Reference semantics: triggerInstrument and updateVolPanAutoVib in:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c
The sine is the mathematical round(64*sin(-i*2*pi/256)) tracker scale. Sweep uses
fixed-point amplitude and preserves the reference's key-off step-amplitude and
strict greater-than cap behavior. The native computed pitch/mixer profile is
retained; absolute FT2 pitch table/mixer parity is not established.

Every PCM byte matches at 11025/44100 Hz mono/stereo with no ignored commands.
Actual worker acceptance compares independent PCM sequentially, after rewind,
second-row seek and EOF using a mock sink. Physical speaker acceptance,
envelopes/fadeout, remaining unsupported behavior and complete codec coverage
remain unqualified.
