# Owned WavPack custom sampling rates

Generate separately with `python3 scripts/generate_wavpack_custom_rate.py`.
The generator reads no prior generated fixtures, private data, external codecs
or network. Ordinary tests consume the checked-in vectors directly.

Thirty accepted videos cover mono, stereo and six-channel frames, three-byte
and four-byte metadata, 37,500 Hz, 0xffffff, 0x1000000 and 0x7fffffff Hz. The
extended form ignores the reserved high bit, including an all-ones last byte.
High frequencies exercise the representable format and decoder timebase; these
are not claims about physical audio-device support.

Each 750-sample component has literal predictor seeds and a zero-residual stream.
Independent scalar float PCM is interleaved in channel order. Three complete
frames have raw headers and standard compact Matroska packets. Matroska uses
nanosecond timestamps derived from sample counts, plus three neutral VP9 frames.
Tests compare exact PCM and rates after reset/rewind, through raw decoding,
container mapping and the reader's decoded-packet presentation path.

Five malformed short videos exercise wrong metadata lengths and zero rates in
both metadata forms, including a reserved-bit-only value. Refusals must reach
the specific rate error and leave the next good frame decodable.

There are 251 generated artifacts totaling 1,270,037 bytes, excluding this README.
This is finite metadata coverage, not complete WavPack acceptance or a hardware,
worker pacing, resampling, or arbitrary-rate seek qualification.
