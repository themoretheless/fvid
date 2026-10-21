# Owned XM vibrato and tremolo acceptance

Regenerate explicitly: `python3 scripts/generate_xm_vibrato_tremolo.py`.
All 86 files regenerate byte-identically with PATH empty and denied
subprocess/socket creation. All 72 preceding arpeggio/glissando artifacts remain
identical. Ordinary tests only read saved fixtures; no FFmpeg, codec tool or
network is required. Samples and parameters are authored, never copied from
private media. The independently decoded owned VP9/PCM companion contains no XM.

Eighty linear/Amiga scores provide literal pitch/volume targets and phase-reset
positions. The reference ramp renderer interprets no modulation effect numbers,
waveform tables, phase counters, parameter memories or volume-control ordering.
Before implementation, vib-sine-cycle-linear differed at 11025 Hz mono despite
equal PCM lengths; actual worker packet1 also differed (28224 bytes each).
Four off controls isolate equal-length pitch/volume failures while matching
independent unmodulated period/volume traces exactly.

Coverage includes 4xy/6xy vibrato, Ax speed/Bx depth, 7xy tremolo, E4x/E7x
wave/no-reset controls, sine/ramp/square cycles, waveform3 alias, independent
nibble memories, full-nibble phase wrapping, blank/other-command output behavior,
Axy/6xy memory, double vibrato and main-column override ordering, note/instrument/
E9/ED/R reset distinctions, speed1 not storing ordinary parameters, independent
channel modulation, Ax result-dependent Rxy tick-zero gating, tremor/tremolo
interchange, volume/pan writes, clipping and zero-period cursor freeze. Tremolo
ramp tests cover both signs of the vibrato phase, preserving the FT2 sign-source
quirk. A no-reset E7x mode remains responsive, following the hardened reference
rather than reproducing an original FT2 interrupt-lock bug.

Command semantics: doVibrato, vibrato, vibratoPlusVolSlide, tremolo,
triggerInstrument, v_Vibrato and waveform controllers in:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c
The sine table is the mathematical floor(255*sin(i*pi/32)) tracker scale:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_tables.c
Values retain the existing native linear/computed-Amiga pitch and smoothing
profile; absolute FT2 table/mixer parity is not established.

Every PCM byte must match at 11025/44100 Hz mono/stereo with no ignored commands.
The actual worker matches independent reference PCM sequentially, after rewind,
second-row seek and EOF through a mock sink. These do not prove physical output,
instrument auto-vibrato, envelopes/fadeout, other unsupported effects or complete
codec coverage.
