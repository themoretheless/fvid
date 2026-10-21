# CodecDelay, sample coordinates and known empty PCM

Generate separately with `python3 scripts/generate_matroska_audio_delay_regressions.py`.
These are authored 60 ms VP9/8 kHz mono PCM videos. No private media, external
codec, FFmpeg or network is used. Rust tests consume the saved files.

Source packets retain literal nanosecond CodecDelay subtraction, including the
one-nanosecond case. Audible PCM positions restore the nearest whole-sample
delay; the one-nanosecond delay represents zero samples at 8 kHz, so subsequent
audible packets remain at 20 ms and 40 ms. This changes no encoded timestamps,
packet data or expected sample values.

`all-preroll.mkv` and `all-padding.mkv` have valid PCM whose entire audible
windows trim away. Once indexed, their duration is known zero, rather than
unknown. Rewind and seek must remain empty without inventing a negative source
packet at EOF. The all-padding case has three complete 20 ms tail discards.
The video still decodes all three frames independently.

The other cases cover zero, fractional-packet and whole-packet delay, overlapping
signed padding, full samples, exact packet source times and worker playback.
`overflow-delay.mkv` remains an explicit zero-output refusal control. Existing
seven movie files remained byte-identical when adding the all-padding movie;
the updated one-nanosecond expectation concerns presentation coordinates only.
