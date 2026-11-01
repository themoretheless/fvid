# In-band dense arbitrary-tree output permutation changes

Before this fix, the authored `plain32-tree0` permutation movie failed at the
second AU with `LD-MPS arbitrary-tree layout/grid change requires stream
reconfiguration`. All configurations have identical topology, parameter grid,
output count and full-band CLD syntax; only output indices change. The first AU
is identical to the already qualified static programme. The regression header
test independently parses all three configurations and complete spatial frames.

The original arbitrary-tree function indexes its gain history by the preorder
leaf counter, not by the output index. Native reconfiguration now preserves
those histories when source paths and band geometry match, while accepting a
new dense output permutation. Synthesis banks remain attached to their output
indices, retaining their existing tails. Topology, grid, extension-index and
channel-count changes remain explicit transactional refusals. Sparse indices
and partial-band processing are not admitted by this fix.

The public component can also atomically update its dense output permutation.
Invalid lengths, duplicates or sparse indices leave both routing and gain
history unchanged. A reset clears gain histories for the currently configured
routes; a native stream rewind still recreates its initial configuration.

## Independent reference and acceptance

The unmodified ISO `arbitraryTreeProcess` was run through the authored external
reference harness with output indices changed before each block. The corpus
now contains42 static plus42 changing-permutation component programmes over
all seven grids and32/64/128-bin domains, including nested/multiple roots and
all base fixed-gain schedules. The original42-programme reference prefix is
byte-identical (SHA2562d96af68ca6176b96ecf747cf20b68bea0f90d07d7f804eb4377e4b9096f0b0c).

Changing-programme blocks use nonzero first endpoints, so prior-frame gain
history affects the output. A deliberately reset control differs by
0.2537881310044837, proving this test does not silently accept a gain-history
reset. Across957312 scalar original-C comparisons, the maximum absolute error
is4.4335261017991456e-8, with tolerance2e-7*(1+abs(reference)). Original C remains
only an explicit reference generator, never a production or ordinary-test
runtime dependency. Its archive/source/harness/input/output identities are
recorded in the updated reference JSON.

The new48-programme playback corpus cycles identity, reversed and rotated
output indices through three in-band headers. It covers every base tree in
plain32/64, direct shared LD-SBR32/64 and PCM-coupled SBR64/128 domains. Native
PCM is compared with independent saved authored core/QMF, owned base spatial
DSP and a separately written analytic extra split/permutation (F32 absolute
tolerance1e-6). That is component composition, not whole foreign-decoder
qualification. Both decorrelation modes retain source timestamps/durations,
checkpoint replay and damaged-packet retry. MP4/Matroska live readers compare
full playback, nine seek windows and rewind against each complete decode.
The common helper is shared with the48 static programme suite; the dedicated
changing-header test asserts that it is reading the changing corpus.

Three focused integration targets completed13 tests,0 failed,0 ignored. A
separate state unit passed (591 unrelated unit tests filtered). That unit now
accepts a position permutation but verifies atomic rejection of an actual
source-topology change and reset of the current routes. Exact commands and log
hashes are in the adjacent JSON. This is not a full-current-tree gate.

The current macOS probe completed12 runs over MP4/Matroska representatives of
all six domains and3/7/9 source channels. Full playback, two seeks and rewind
all drained through the actual default device:48 phases. This does not certify
subjective quality or physical multichannel speaker wiring. Probe SHA256 and
raw output are in the device JSON.

All305 files across the component, static and changing playback corpora were
reproduced byte-for-byte with subprocess/socket creation forbidden during
authored generation. Inputs and media are synthetic; no private/ISO media or
codec parameter sets were copied. Ordinary tests need no FFmpeg, C compiler or
network. Source prefixes are explicitly delimited in the static manifest for
re-enveloping without duplicating existing spatial data.

Partial-band/LFE trees, sparse positions, topology/grid changes, nonzero EBQ
with active arbitrary trees, active unequal-grid HRTF and inherited-fragment
prefix loss remain open. This does not complete LD-MPS or the codec-gap goal.

Current boundary update (2026-11-02): covered partial grids are now accepted when every actual QMF bin has a coded coefficient. See [covered partial qualification](aac-ld-mps-arbitrary-partial-2026-11-02.md). Earlier refusal statements in this historical report describe its original snapshot.
