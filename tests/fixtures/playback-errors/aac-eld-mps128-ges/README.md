#128-band AAC-ELD / LD-MPS GES regression corpus

Generate own videos separately with
`python3 scripts/generate_aac_eld_mps128_ges_fixtures.py`.
396 programmes each have MP4 and Matroska versions:792 acceptance movies.
They cover64/88.2/96kHz,480/512 core frames,all8 trees,both TTT variants,
23/9/4 parameter grids,repeated headers,fragmentation,delayed windows and
in-band GES -> ordinary -> GES switches. Disabled flags still warm histories.
18 bare core controls and36 precise late-corruption refusals are separate:
trailing bytes,in-band rate changes and reshape runs exceeding4/15 slots.

The initial own movie refuses specifically with `invalid LD-MPS GES geometry`.
Acceptance requires complete PCM/layout/count,owned/frontend exports,
AudioStream playback,forward/backward/middle/EOF seek,rewind,repeated ranges,
source-window checkpoints and atomic retries.66 paired ordinary movies prove
active GES has a signal effect; ignoring GES signalling must fail.
AudioWorker capture additionally verifies792 programmes and live-history seeks.

Generate own component stimuli separately with
`python3 scripts/generate_aac_ld_mps_ges128_component_fixtures.py`.
48 sequences cover all trees and dense,silent,cancellation,high-only,low-only,
upper-only signals. The last mode puts energy solely in bins64..127.
Freeze independent outputs with
`python3 scripts/generate_aac_ld_mps_ges128_reference.py /path/to/official.zip`.
This optional research step compiles the pinned unmodified ISO GES algorithm
and original mapper,with a fully initialized23-band envelope workspace.
No foreign implementation is linked into FVid.1,837,056 frozen scalar outputs
qualify the component,not a full compressed-stream decoder. Whole PCM tests
compose the analytical ELD core with the separately qualified spatial session.

All media and spatial syntax is locally authored synthetic data. No private
source frames,audio or codec parameter sets. Own generators and ordinary tests
use no FFmpeg,network or external process; C research generation is separate.
Raw native-rate full-complex output is qualified,not full profile conformance:
profile-specific output adaptation above48kHz,physical audio output and real-time
throughput remain separately unqualified. Residual128 remains open.
