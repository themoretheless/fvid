# Frozen synthetic audio reference vectors

These inputs contain only the original authored 240 ms / 48 kHz nonzero tone
audio packets and codec configurations for FLAC, ALAC, MP3, MP2, Vorbis and AC3.
Mono is retained except the original stereo Vorbis case. They were extracted
without transcoding from the previous six sample-clock movies, before replacing
their container/video generator. The manifest records each original movie's
SHA-256, packet offsets/sizes/timestamps, delay, duration, padding, references,
and hashes of the coded payload and configuration.

The `.private` extension means Matroska CodecPrivate: synthetic codec setup,
not private media. No private video, audio, frames or parameter sets are stored.
No video or container bytes are stored here.

Ordinary generation reads these fixed independent vectors, checks their hashes,
and builds fresh owned VP9/Matroska bytes. It never reads generated movie files,
runs FFmpeg or invokes any external encoder. This is reference-data reuse,
not an implementation or claim of six native audio encoders.

External reference refresh is explicitly isolated in
`benches/generate_matroska_audio_sample_clock_reference.py`. Its default output
is ignored `benchmarks/data/matroska-sample-clock-reference`, so running a
reference benchmark does not replace these checked inputs. Adoption of newly
encoded vectors and corresponding clock expectations requires separate review.
