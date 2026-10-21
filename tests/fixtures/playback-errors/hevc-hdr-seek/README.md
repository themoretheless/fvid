# Received HEVC static HDR during random access

Generate separately with `python3 scripts/generate_hevc_hdr_seek.py`.
Only the existing owned `hevc-pcm-reference-rext8` coded samples and original YUV
oracle are reused. HDR payloads, packetization and container wrappers are
authored here. No private media/parameters, FFmpeg, external codec or network
is used by generation or ordinary Rust tests.

Nine acceptance movies cover hvc1, hev1 and Matroska, each with HDR SEI inline
before the IDR, in a preceding metadata-only packet, or split into separate
mastering-volume and content-light packets. Each movie has three independent
coded sequences, each carrying its own complete static HDR declaration before
its first coded picture. The values remain unchanged inside each sequence.
H.265 D.3.28/D.3.35 require this consistency and describe CLVS persistence:
https://www.itu.int/rec/T-REC-H.265-202601-I/en

Before the fix, sequential playback and rewind matched all pixels and HDR at
each sequence's first output, but seek to cycle 2 in `prefix-hvc1.mp4` and
`prefix.mkv` lost the mastering display. Inline controls decoded the same samples
and HDR correctly. The direct decoder's parameter priming also ignored a valid
received mastering-volume packet.

The regression now checks complete native pixels for every picture, all MP4
sample indices, sequential/rewind EOF, forward/backward/repeated random access,
and all mastering primaries/white/luminances plus MaxCLL/MaxFALL on every
output, including the last picture before the next sequence during lookahead. Both individual-message and combined packets
are checked. `configured.hvcc` verifies that reset restores recorded HDR after
an in-band override; it does not change the recorded reset snapshot.

`bad-parameters.packet` is a separate refusal control: valid HDR followed by an
incomplete PPS must fail priming without changing previously received HDR,
require reset, then allow clean priming after reset. It is not malformed video
playback acceptance.

All 21 generated artifacts regenerate byte-identically. Scope is restoration
of received static HDR alongside parameters, without decoding prior pictures.
Per-output-frame metadata during software lookahead is checked for all nine
movies: the last picture of sequence A must retain MaxCLL 1000 rather than
inherit sequence B's 1400. Hardware seek, layered HDR policy and other SEI
state are not qualified by these witnesses.
