# Owned XM pitch slide qualification

Regenerate explicitly: `python3 scripts/generate_xm_pitch_slide.py`.
All 46 artifacts regenerate byte-identically with PATH empty and denied
subprocess/socket creation. Ordinary tests read saved artifacts only; they do
not need FFmpeg, codec tools or network. Samples/parameters are authored, with
no private source material. The separately decoded owned VP9/PCM companion does
not contain XM.

Forty-two scores qualify ordinary 1xx/2xx, fine E1x/E2x and extra-fine X1x/X2x
slides on both linear and Amiga pitch laws. The manifest supplies explicit
per-tick periods and sample-phase reset positions. The test oracle knows no
slide effect numbers, parameters, memories, clamps or scheduling rules: it
renders the owned 64-frame signed16 ramp from those literal traces. It uses the
existing native pitch/mixer profile, not an absolute FT2 mixer oracle.

Coverage includes up/down, six independent parameter memories, zero/default
memory, speed-one not executing/storing ordinary parameters, note-trigger reset,
instrument-only preservation, E9/ED phase/pitch resets, held-cycle ordinary
execution versus fine/extra-fine one-shot setup, half-period linear finetune,
Amiga finetune rounding, upward clamp and FT2's signed downward clamp behavior.
The reference for command semantics is pitchSlideUp/Down, finePitchSlideUp/Down
and extraFinePitchSlide in:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

Two no-slide control scores prove an equal-length PCM mismatch against the
requested slide and match the independent constant-period reference exactly.
The initial pre-fix test attempt exposed missing release-tail samples in the
oracle, not a valid slide baseline; the oracle also needed positive zero when
summing silent voices. Both controls were corrected explicitly. The final
no-slide reproduction isolates pitch change without relying on those errors.

All PCM bytes must match at 11025/44100 Hz mono/stereo, with no ignored commands.
The actual worker must match independent reference PCM packet by packet through
sequential playback, rewind, second-row seek and EOF. These are mocked-output
worker checks, not physical speaker acceptance. Exact FT2 pitch tables, tone
portamento, arpeggio, vibrato, envelopes and overall codec coverage remain open.
