# AAC byte alignment

Generate explicitly with `python3 scripts/generate_aac_alignment_fixtures.py`.
Ordinary tests never run the generator or require a network/FFmpeg installation.

33 authored programmes cover AAC-LD and AAC-ELD with each spatial tree0–7,
plain ELD480/512 with1/2/6 input channels, coupled spatial LD-SBR32/64,
and adapted/high-rate LD-SBR through PCM ingress. Each three-packet programme has
zero, all-one, alternating and last-bit-one terminal AAC padding variants,
in both MP4 and Matroska:132 variants,264 movies. Only padding bits change;
the ASC and every core/spatial syntax bit remain identical. Input cores and
spatial parameters come from existing authored generators, not ISO/private media.

The old decoder accepts all22 original zero controls and rejects `eld-tree0-ones`
specifically with `nonzero AAC ELD alignment bits`. The fix accepts all variants
with byte-exact control PCM, playback, seek and rewind. The older
`aac-eld-spatial-tail/tree7-alignment.mp4` is also an acceptance input now;
its movie and packet bytes are unchanged. Malformed DRC/envelopes remain errors.

The rule applies to terminal AAC byte alignment, not internal LD-MPS frame
alignment, CRC-protected fields, fill-data bytes or ASC downscale fill nibbles.
Those validations are retained. Public ISO conformance streams are fetched only
by the separate optional `scripts/fetch_iso_ld_mps_conformance.py OUTPUT` tool;
their media and parameter sets are not included in this synthetic corpus.
