# Owned XM 0xy arpeggio and E3x glissando

Regenerate explicitly: `python3 scripts/generate_xm_arpeggio_glissando.py`.
All 72 artifacts regenerate byte-identically with PATH empty and denied
subprocess/socket creation. The shared tone-portamento generator also preserves
all 58 prior artifacts byte-for-byte. Ordinary tests read saved files only and
require no FFmpeg, codec tools or network. All samples/parameters are authored;
no private media is used. The owned VP9/PCM companion is decoded separately and
does not contain an XM stream.

Sixty-eight linear/Amiga scores supply literal per-tick periods, volume targets
and sample-reset positions. Their independent ramp renderer interprets no
arpeggio/glissando commands, clocks, quantization searches or parameter memories.
Before implementation, arp-four-linear differed in PCM at 11025Hz mono despite
equal lengths; worker packet0 also differed (28224 bytes each). Two E30 control
scores separately isolate equal-length quantization failure: they match literal
raw-portamento traces exactly and differ from the intended glissando output.

Arpeggio coverage includes both/individual/max nibbles, parameter changes,
000/other-effect restoration, new-note and ED ordering, key-off phase retention,
held-cycle local tick0, independent channels, speeds1/2/5/6/16/17/31, fractional
and negative finetunes, zero offsets requiring quantization, relative octaves,
the high-note ceiling and its exact/below-boundary indices. E3x coverage includes
main/column portamento, nonzero mode values, disabling without immediate pitch
rewrite, ordinary slides unaffected by quantization, arpeggio using the real
rather than quantized period, held cycles and independent channel modes.

The command semantics follow arpeggioTab, arpeggio, period2NotePeriod,
setPortamentoCtrl, portamento and getNewNote in the FT2 reference:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_tables.c
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c
The backward remaining-tick clock, table overflow above index16, high-note
ceiling and separate real/output periods are intentional. Pitch values retain
the existing native linear/computed-Amiga profile; absolute FT2 table and mixer
parity is not established.

Every PCM byte must match at 11025/44100Hz mono/stereo with no ignored commands.
Actual worker acceptance checks independent PCM sequentially, after rewind,
second-row seek and EOF through a mock sink. Physical speaker acceptance,
vibrato/tremolo, envelopes and full codec coverage remain open.
