# Owned XM EEx held row cycles

Regenerate explicitly: `python3 scripts/generate_xm_pattern_delay.py`.
All 42 artifacts regenerate byte-identically with PATH empty and denied
subprocess/socket creation. Ordinary tests read saved files only. No private
media, FFmpeg, external codec or network is used. The owned VP9/PCM companion
is independently opened/decoded by the worker acceptance test.

Twenty scores have independently authored speed-1 tick cells. Expected notes,
channel/global volumes and placements are literal, not produced by the EEx
implementation. Explicit common speed-4 key-off rows equalize release tails.
Baseline hold-one gives 10584 bytes instead of 14112 at 11025 Hz mono because it
omits the held cycle; the actual worker differs in its second equal-size packet.

EEx adds x held cycles. Only the original cycle reads new notes/tick-zero
commands. Held cycles instead run the nonzero-effect dispatcher even at local
tick zero: ordinary volume/pan/global slides and E9x can therefore run at the
cycle boundary, ED0 can restart, while E90 does not. Fine/direct tick-zero commands
are not simply reapplied. A private parallel continuation directory keeps the
public TimelineRow structure unchanged; internal row packets carry a validated
continuation bit. Held cycles retain their originating note row while flow cursors
follow the existing E6x/Bxx rules. Each cycle counts against the row limit.
Reference: FT2 patternDelay, getNextPos and tickReplayer:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

Cases cover EE0/EE1/EEF, last-channel priority including EE0 cancellation,
column/fine/direct commands, Axy/Hxy/Pxy at held tick zero, E9x/EDx/ED0/E90,
speed=1, E6x interaction, backward same-pattern jump and forward pattern jump.
The forward destination's unread row zero is skipped, following FT2's held-flow
behavior. Native one-pass global traversal policy remains in force.

Acceptance compares every independent-control PCM byte at 11025/44100 Hz,
mono/stereo, with no ignored features. Worker sequential/rewind/second-row seek/
EOF checks use independent PCM regrouped into actual cycle packet spans. Explicit
literal trace tests check originating order/row, continuation flags and exact
row limits. The unknown-flag check is an internal packet refusal control, not
playback acceptance for unknown metadata. Envelopes, remaining effects, arbitrary
flow combinations and absolute FT2 mixer parity remain unqualified.
