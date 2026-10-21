# Owned AVC primary pictures with redundant coded areas

Regenerate explicitly: `python3 scripts/generate_avc_redundant_primary.py`.
All 85 artifacts regenerate byte-identically with PATH empty and denied
subprocess/socket creation. Ordinary tests only read saved files and need no
FFmpeg, reference decoder or network. The Extended-profile SPS/PPS and PCM/skip
syntax are authored; there are no private frames or codec parameter sets.

Twelve 32x16 two-picture videos have primary PCM I and P-skip pictures. Expected
Y/Cb/Cr bytes come from an independent integer coordinate pattern; redundant
PCM uses a visibly similar but distinct +1 pattern. Primary-only packet controls
must produce exactly the same samples. Coverage combines one/two primary slices,
ASO, full/partial redundant areas, ranks1/2/127, nonzero NRI classes and same/
alternative PPS (with a different default QP). A primary uniform-P promise must
not be checked against a distinct redundant I representation.

Before the fix, the first valid access unit failed with `AVC slice macroblock
addresses overlap`, both directly and through MP4. The preparation path now
parses redundant headers with their own parameter binding, validates association,
and omits their RBSP from retained primary slices. Primary count/coverage/type
checks remain strict; packet byte limits still apply before header allocation.
Reference marking and reconstruction consume only primary slices.

Normative context: H.264 clause8 uses the primary coded picture for conforming
bitstream decoding. Redundant representations can cover only part of a picture
and are useful for loss recovery; they are not additional display pictures.
https://www.itu.int/rec/T-REC-H.264
https://www.rfc-editor.org/rfc/rfc6184.html

Direct acceptance checks every plane byte through two decode/reset passes.
Forced-software MP4 acceptance checks exactly two primary outputs, rewind,
sync-seek and EOF. Preparation tests require exactly the primary slices, original
NALs and ranges, including exact/one-byte-short packet limits.

Six separate negative cases cover mismatched picture identity/reference class,
unknown PPS, incompatible PPS POC presence, duplicate primaries and a redundant-
only packet. They check error messages, the reset-required latch and successful
reset/redecode. The redundant-only case is a refusal test, not recovery acceptance.
Loss concealment/reconstruction without a primary, data-partitioned NAL types,
other profiles/tools and complete codec coverage remain unqualified.

A separate syntax/sample validator checks all 48 redundant NAL payloads, every
PCM codeword/alignment/sample coordinate and RBSP trailing bit. This confirms
that the omitted representations are valid, distinct PCM areas rather than an
unrelated malformed-payload reproducer.
