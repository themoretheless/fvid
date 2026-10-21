# AV1 received HDR versus opening metadata

Run `python3 scripts/generate_av1_hdr_sequence.py` explicitly to regenerate.
Ordinary Rust tests only read the saved artifacts. The wrapper copies the exact
configuration and coded key picture from the owned synthetic
`av1-separate-tiles-w128-h128-groups2` seed. No private media, new external encode,
FFmpeg or network is used. Generation also works with an empty PATH and denied
subprocess/socket creation. Eleven generated artifacts are byte-identical.

Each of three packets contains the original independent key picture and sequence
header, preceded by the original temporal delimiter. An authored HDR_CLL metadata
OBU follows the sequence header and precedes its frame header. Received
MaxCLL/MaxFALL are 1000/400, 1400/500 and 750/250. The metadata OBU has type 1,
two unsigned big-endian 16-bit fields and a trailing one bit; no layer extension
is used. AV1 metadata scope begins where it appears:
https://aomediacodec.github.io/av1-spec/#general-metadata-obu-semantics

The direct native decoder accepted all three distinct values before the fix.
MP4 and WebM instead returned 1000/400 for picture 1, where 1400/500 was expected:
the saved opening snapshot overrode the live decoder. Both container regressions
now require all exact original planar pixels and each received light statement,
including sequential replay, rewind, EOF and repeated forward/backward seek
replay from the indexed entry point. MP4 sample indices are also exact.
These are playback acceptance tests, not unsupported-stream refusal controls.

This witnesses received CLL priority in single-layer software AV1. It does not
qualify dynamic HDR10+, layered HDR policy, hardware output or all metadata types.

The `zero-` variant uses CLL 1000/400, 0/0 and 750/250. Before its fix,
all three direct/MP4/WebM checks retained 1000/400 at picture 1 instead of the
explicit 0/0 statement. Its acceptance tests also verify exact source pixels,
EOF, rewind and repeated seek replay. Additional controls check an explicit
zero statement in the recorded decoder reset configuration, absent metadata
(padding) versus zero presence, and last-received precedence in both packet
scanners. Decoder state tracks receipt separately from numeric light values;
opening fallback cannot overwrite a received zero statement. Existing generic
HDR merge semantics for unrelated codecs/containers are unchanged by this fix.
