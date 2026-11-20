# Defined mixed-cutoff ArbitraryTree history

The former guard rejected every resolution change involving mixed split
cutoffs. An isolated build of the exact prior Decoder component at79125a8c73
reproduces that refusal for the own12/[11,12] ->4/[4,4] transition. The prior
component used canonical QMF-map stubs; this is not represented as a full old
player run. The same authored MP4/Matroska now passes native playback.

## Implemented state semantics

History now stores both the23 indexed gain values and a per-leaf defined-band
mask. Processing follows the preorder split/leaf routing with optional stack
coefficients: initialized rows have a value; unwritten rows have none. Pushes
and pops propagate that definedness through the actual coded cutoffs. The last
visited split determines which history entries are updated, matching the source
algorithm even when it differs from the minimum cutoff on the leaf path.
Unwritten entries preserve their old value and definedness. Undefined writes
clear their mask without fabricating a coefficient.

A new header can inherit unchanged source/split paths exactly when all bands
addressed by its canonical QMF map have defined prior gain history. Process also
checks the caller's actual map before use. Both failure paths are atomic. This
admits defined mixed-cutoff transitions, including dormant-history returns,
without using any unknown value. Output permutations retain logical leaf
history, reset restores values and masks, checkpoints clone them, and retained
memory accounting includes the new mask vector.

All active QMF bins must still have explicitly coded coefficients at every
split. Active-uncovered behavior, changed source/topology paths, sparse output
positions, EBQ with active arbitrary trees and active unequal-grid HRTF remain
open. Undefined dormant history is still an explicit refusal; the reference's
uninitialized behavior is not claimed as intended normative playback.

## Independent C qualification

696 own transition scenarios span simple nested-left, nested-right and separate-
root splits, all eligible mixed-cutoff state pairs,32-bin maps and return.
Two separately compiled copies of the unchanged original arbitraryTreeProcess
use zero and pattern initialization of automatic storage.648 scenarios remain
finite and byte-identical across all three blocks.48 first diverge at a precise
later block; only their preceding defined prefix is saved as gold. Raw undefined
outputs stay in the external reference cache. The owned mask implementation
accepts all648 and refuses all48 at exactly the independently observed block.
1,548,288 scalar comparisons have maximum absolute error3.913235131769444e-9,
below2e-8. Reset is checked against a fresh component after accepted sequences.
C's fixed CLD input workspace is padded for transport only; owned tests use
actual short coded rows. Original C never reads that padding for covered bins.
Archive, algorithm, harness, input/output hashes and compiler flags are pinned
in the source receipt. The reference is not linked into production or invoked
by ordinary tests; this is a post-shaping component comparison, not a foreign
whole-audio-decoder comparison.

## Media, navigation and delivery

1,716 own three-AU programmes are playback acceptance: every accepted component
transition on tree7 in plain/shared-LD-SBR32, plus all mixed initial states
returning through full4 on the other seven base trees and three split shapes.
96 own refusal videos cover every undefined scenario in both domains. Every
header/profile/frame is parsed completely and all active bins are confirmed
covered. Refusals occur at the exact later header; they preserve retained state
and permit valid retry with checkpoint-equivalent output. They are not intended
playback acceptance of undefined history.

Accepted MP4 videos and Matroska produce identical nonzero PCM with exact source
duration. Both decorrelation modes check clocks, checkpoint restore and damaged-
packet retry. Readers check full playback, nine seek boundaries and rewind.
The explicit authored generators reproduce3,630 files byte-for-byte with process
and socket creation forbidden. They consume saved independent reference
classification, never compile or execute C. Ordinary tests need no FFmpeg,
network or fixture generation.

32 unique focused tests pass without failures or ignored tests. The rebuilt
macOS probe completes12 runs and48 default-device drains for playback, two seeks
and rewind, including nested/separate-root shapes and4/8/10 source channels.
This establishes device delivery, not subjective listening or physical speaker
wiring certification. Exact commands and log hashes are in adjacent JSON.
This remains focused qualification; whole codec parity is not complete.
