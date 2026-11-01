# Native full-band arbitrary-tree playback integration

Historical integration milestone at4388ad0a84. Dense in-band output permutation
changes are now qualified; see `aac-ld-mps-arbitrary-permutations-2026-11-01.md`.
Topology/grid and sparse-position boundaries remain open.

This supersedes the component-only playback boundary recorded at8429dc0934.
The owned post-shaping arbitrary-tree stage is now connected between the
base-tree dry/wet sum and spatial PCM synthesis. Its configured dense leaf
indices determine output channels; base-tree canonical permutations are not
applied again. Fixed surround/LFE gains apply once per base source, before
synthesis, rather than again to the additional leaf channels.

ASC/native channel counts, source-window assembly and in-band count checks now
include arbitrary-tree leaves. The native decoder reports an unknown speaker
mask and no inherited core PCE positions for these explicitly indexed outputs;
it does not invent a standard speaker layout. Synthesis allocates one bank per
output. The new gain/path/fixed-gain buffers are included in retained-memory
accounting, including checkpoints. This is retained payload accounting, not
peak allocation or RSS measurement.

Gain history survives same-layout headers and downmix-gain reconfiguration.
Reset clears it. A layout, position permutation, extension-index or parameter-
grid change is an explicit transactional reconfiguration refusal until its
history mapping is qualified. Source-format changes cannot silently change a
player's channel count. A focused unit test checks gain-history preservation,
atomic rejection and reset. Invalid frame suffixes retain native checkpoint
state and allocation counts and allow an intact packet to be retried.

The original `tree7-extra-ott` movie is now a passing waveform acceptance test,
with an independent saved synthetic core and a separately written analytic
split composition. Its generator was corrected to take prefixes from the
actual core control: the downscaled *spatial* rows' `core_bits` included their
existing spatial suffix, so treating those rows as raw core duplicated side
data. The acceptance now requires exactly three source windows and768 samples,
preventing that mistake from hiding behind the former constructor refusal.

The new48-programme corpus exercises all eight base trees in six domains:
plain32/64, direct shared LD-SBR QMF32/64, and PCM-coupled LD-SBR64/128. Each
contains three authored AUs, one additional full-grid OTT, and independently
varied fixed surround/LFE gains. MP4/Matroska output matches independently saved
core PCM/QMF followed by an analytic extra split, base owned spatial DSP and
synthesis (absolute F32 tolerance1e-6). This is component composition, not an
independent whole foreign decoder. Native tests cover both decorrelation modes,
source timestamps/durations, damaged-suffix retry and checkpoint replay.
Live reader tests compare full playback, nine seek windows and rewind byte-for-
byte with each stream's complete decode. The earlier42-programme independent
C component oracle remains byte-identical and covers nested/multi-root trees.

Nine focused integration targets completed:32 passed,0 failed,0 ignored. The
separate state unit test passed (591 unrelated unit tests filtered out). Exact
commands and log hashes are recorded in the adjacent JSON; this is not a full
current-tree gate.

The actual macOS output probe completed12 MP4/Matroska runs, representing all
six domains and3/7/9 source channels. Full playback, two seeks and rewind all
reached device drain:48 complete phases. This proves callback/queue/navigation
progress on the default output, not subjective quality or physical multichannel
speaker wiring. The device JSON pins the probe executable SHA256 and raw output.

Both authored generators reproduced159 files with subprocess/socket creation
forbidden. The explicit C reference step is separate from ordinary tests and
was re-run with byte-identical output. No FFmpeg, libav, private media or copied
ISO media/parameter sets are required by ordinary tests or production playback.

Remaining work includes partial-band/LFE splits, sparse positions, arbitrary-
tree layout/grid changes, and nonzero EBQ with active arbitrary trees. General
active unequal-grid HRTF and inherited-fragment prefix loss also remain open.
Full LD-MPS and the overall codec-gap goal are not complete.
