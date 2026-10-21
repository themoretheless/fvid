# Owned XM Hxy global volume slides

Regenerate explicitly: `python3 scripts/generate_xm_global_volume_slide.py`.
All 34 artifacts regenerate byte-identically with empty PATH and denied
subprocess/socket creation. Ordinary tests only read saved fixtures, without
FFmpeg, external codecs, network or private media. Original Axy/column/fine
artifacts also remain byte-identical after extending the common wrapper.

Sixteen scores contain the owned signed-16-bit looping sample. Independent
controls expand every tick into a speed-1 row with a literal Gxx value. Channel
sample volumes stay fixed at full scale (the A-memory control slides upward
into saturation), so Gxx alone represents the exact expected global envelope.
The shared wrapper's explicit control-effect option uses Gxx here and Cxx for
the earlier local-volume suites. Expected values are authored, not calculated
by the Hxy implementation. Common speed-4 key-off tails equalize policy release.

Baseline down has equal PCM lengths but different bytes at 11025 Hz mono.
Worker generation zero also differs in its first equal-size packet. Acceptance
requires every PCM byte at 11025/44100 Hz, mono/stereo, plus zero ignored features.
The worker checks sequential playback, rewind, second-row seek and EOF against
independent Gxx PCM regrouped into actual source packet spans. The owned VP9/PCM
companion is also decoded; this does not claim XM audio is embedded in that video.

Hxy executes on nonzero ticks; H00 remembers the last executed parameter of the
same channel. The upper nibble wins, values clamp to 0/64, and each channel's
operation applies immediately in channel order. Opposing slides near clipping
are deliberately noncommutative. Cases cover direction, both nibbles, clamping,
H00/empty memory, Gxx retaining memory, Axy/Hxy separation, speed=1 not storing,
silent-channel effects, both clipping orders and independent channel memories.
Reference: globalVolSlide and per-channel dispatch in the FT2 replayer:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c
Other tracker effects, envelopes and absolute FT2 mixer fidelity remain open.

Two extra controls ensure an initial empty H00 stays empty after rewind even
though a later row stored H02, and that G00 muting followed by H10 preserves
sample phase against independently scheduled G00/G01/G02/G03 control ticks.
