# Native flat PCM BW64 playback

Baseline e934f0a26b rejects authored BW64 as `not a RIFF/WAVE file`: one independent RIFF/video test passes and four format tests fail in `before.log`. No prior tests were disabled or relaxed.

Normative source: ITU-R BS.2088-2 sections 2.4 and 4.2, https://www.itu.int/rec/R-REC-BS.2088-2-202511-I/en . BW64 uses ds64 for extended sizes, but its two dummy words must be ignored on read. They are not the RF64 sample count. BW64 also has no RF64 fact-count replacement. The shared size walker retains its existing RF64 diagnostics for malformed ds64 metadata; the end-of-file envelope names BW64 explicitly.

Sixty-four positive cases cover sixteen integer/float/extensible profiles across ordinary size markers, repeated-ID tables, finite 32-bit sizes, and a nonzero 64-bit dummy value. All have no fact chunk and expose `declared_frames=None`, independently of the dummy bits. Exact native PCM, encoded packets, fractional-duration seek, rewind and EOF match RIFF controls. Playback workers exercise four seek targets per case (256 sessions). Sixteen base signals also decode byte-identically with FFmpeg 9.0.2, kept optional and separate from ordinary Rust tests.

Eight inherited authored malformed ds64 controls verify specific failures, not merely any refusal. Complete PCM followed by absent tail metadata fails EOF. Both 0xffffffff and 0x100000000 data declarations default-refuse; under explicitly raised limits, the short available prefix decodes exactly and fails EOF. Private capacity assertions bound retained PCM and packet buffers; this is not a peak-RSS or complete multi-gigabyte playback measurement.

This capability plays the flat PCM stream. ADM object rendering, channel-allocation XML interpretation, export/remux, physical audio devices and streaming large files are outside its acceptance claim. Synthetic fixtures contain no private media. Their generator and external reference check run separately; offline Rust tests do not invoke an encoder or network.
