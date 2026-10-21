# Retained E-AC-3 transient windows (metadata stage)

Regenerate separately: `python3 scripts/generate_eac3_transient_windows.py`.
Three owned stereo six-block cases encode left/right window presence, exact
locations/lengths, zero values and the maximum location code referencing later
PCM. Each raw stream has three frames and a neutral VP9 Matroska video; a
no-transient control records the unchanged audio-header boundary.

`tests/eac3_transient_windows.rs` accepts metadata retention, location conversion
to sample units, inactive-channel clearing, exact audio-bit offsets and the same
parameters in all actual Matroska audio packets. `checked_frame` verifies CRC.
The existing unit fixture now also requires retained zero-valued windows rather
than merely skipping their bits. Eight artifacts / 116,202 bytes reproduce
exactly with subprocess/socket denied. Existing 291 presentation and 271 SPX
artifacts remain byte-identical. No private data, encoder, FFmpeg or network.

This is NOT PCM transient-correction acceptance. `pcm_processing_accepted=false`
in the manifest is deliberate: the decoder still does not perform the E.2.7
history-buffer synthesis/crossfades. The new Frame::transient_windows data must
feed that implementation; these parser gates cannot replace its independent PCM,
frame-boundary, reset, seek, channel/downmix and expanded-programme acceptance.
Normative basis: ETSI TS 102 366 V1.4.1 E.1.3.2.21-.23 and E.2.7:
https://www.etsi.org/deliver/etsi_ts/102300_102399/102366/01.04.01_60/ts_102366v010401p.pdf
