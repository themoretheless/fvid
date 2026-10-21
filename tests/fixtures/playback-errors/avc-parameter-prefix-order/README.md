# AVC PPS/SPS replacement and avc3 seek history

Regenerate separately with `python3 scripts/generate_avc_parameter_prefix_order.py`.
The generator authors PCM samples, parameter syntax and independent sample
oracles using the existing owned Python bit writer. No FFmpeg, external codec,
network, private media, frames, audio or parameter sets are involved. Ordinary
tests only read saved artifacts.

Twelve acceptance movies cover Y/C depths 8/8, 10/12 and 14/8, both SPS/PPS and
PPS/SPS order, and inline or combined parameter-only prefixes. Each movie has
two monochrome pictures, replacement SPS/PPS with the same IDs, then two
interleaved 4:4:4 pictures beginning with an IDR. The PPS enables 8x8 transforms
and signals twelve scaling-list-presence flags, all absent. PCM pictures are
unaffected by scaling, so their samples are independent of the codec under test.

`pcm_controls_prove_pps_context_and_all_component_samples` establishes that the
new PPS is invalid with the old SPS and valid with the new SPS, where it has
twelve scaling entries. Fully configured old/new controls match saved planar
samples, retaining each component's own precision.

Two distinct baseline failures are reproduced:

- Sequential `y8-c8-pps-sps-inline` failed at its first new picture with
  `AVC in-band PPS has no valid SPS`, despite the new SPS following in the same
  packet and ordinary SPS/PPS controls decoding the same PCM samples.
- Seeking to the new IDR in `y8-c8-sps-pps-prefix` failed with
  `invalid intra macroblock type`, although sequential playback succeeded. The
  seek reset discarded the received parameters in the preceding metadata sample.

Acceptance checks every native sample, monochrome/4:4:4 storage, component
depths, source sample IDs, PTS, EOF, rewind and seek. The final old picture lasts
80 ms across a metadata sample; all other pictures last 40 ms. Direct decoding
and reset are also checked. Parameter-only priming restores history without
decoding preceding pictures, is forbidden after decoding begins, and observes
the configured packet budget.

`truncated-pps.packet` is a refusal control: its two IDs fit, but contextual
syntax is missing. Priming must reject it, require reset, and restore the
original sequence after reset. This is not a malformed-playback acceptance test.

H.264 7.4.1.2.1 defines availability before activation and applies contextual
constraints to active parameter sets:
https://www.itu.int/rec/T-REC-H.264-202408-I/en

These witnesses qualify same-packet PPS/SPS replacement and software avc3 seek
history. They do not establish arbitrary cross-packet replacement, hardware
decoder behavior or complete AVC coverage.
