# ArbitraryTree extension index changes

An authored three-access-unit programme moves an unchanged ArbitraryTree from
SSC extension index 0 to 1 and back to 0. An ignored kind-12 extension occupies
the other slot; it carries no frame payload. Before the fix, the native probe
failed specifically with `arbitrary-tree layout/grid change requires stream
reconfiguration`. This is a parameter-address change, not a layout change.

The renderer now retains leaf gain history when source paths and parameter
resolution remain identical, independently of extension position. The new
stage keeps its current extension index for looking up current-frame CLD.
Checks for changed split paths, sources, output count and grid remain intact.
No history is copied by output position, and no unsupported topology conversion
is introduced.

The explicit authored generator creates 48 programmes spanning all eight base
trees and the existing PCM/shared-LD-SBR QMF domains. MP4 and Matroska each
contain all three headers. They retain independently authored source PCM/QMF,
nonconstant CLD and nonunity fixed gains. Ordinary tests compare continuous
waveforms against independent analytic split composition, both decorrelation
modes, timestamps, checkpoint restore, truncated-packet retry, reader seeks and
rewind. Header tests verify the exact 0/1/0 extension placement and complete
frame syntax. Generation is separate from tests and reproduces 146 files
byte-for-byte with subprocess and socket creation forbidden.

This qualifies extension relocation only. General topology/grid changes,
sparse positions, active-uncovered bands, nonzero EBQ with active arbitrary
trees and active unequal-grid HRTF remain open. Full codec parity remains open.

Qualification completed: 15 integration tests and the independent state unit
passed, with no failures or ignored tests (591 unrelated unit tests filtered).
The state unit verifies that a genuine split-path change remains refused
atomically. This is a focused qualification, not a full-current-tree gate.
The rebuilt macOS probe completed 12 MP4/Matroska device runs across all six
input domains: 48 complete drains for full playback, quarter seek, 60-percent
seek and rewind. This is device delivery evidence, not subjective listening
or physical multichannel wiring certification. Commands, log hashes, before-fix
probe output and executable hashes are in the adjacent JSON receipts.
