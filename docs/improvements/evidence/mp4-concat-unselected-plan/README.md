# Excluded MP4 transport plans

Accepted FVID-0474 after the recorded frozen-source qualification. Three authored40ms AVC/AAC videos retain identical exact
picture/packet and AAC PCM oracles. Two use valid coarse1000/1001Hz cumulative
AAC clocks. The native decoders accept them; the compressed AAC packet planner
does not support their clock. Excluding AAC must leave video concatenation usable.

Final unchanged five-test baseline:two controls pass,three tests fail. Selected
video concat and the public plan/publication path fail on the excluded AAC.
The new writer-guard test cannot reach its setup on that baseline; it is a
control for the newly admitted case,not a separate pre-existing defect or ID.
Final candidate tests all pass,including exact native pixels,80ms duration,
seek/rewind/EOF,one-packet publication prefix,and retained selected-AAC refusal.

A failed excluded transport plan now has an empty placeholder. Successful plans
remain retained for compatibility,and their allocations are still charged.
Cancellation always propagates. The private planned-track mask prevents a
writer from silently selecting a missing transport plan before emitting output.
Existing successful callers and full-source parser validation are preserved.
No performance improvement or public API signature change is claimed.

The first unaccepted candidate omitted all excluded plans. It was replaced to
preserve existing successful-plan retention and budget/selection behavior.
Its source/focused log are retained;it was never accepted.

Broad tests exposed four failures also reproduced on published production code.
Two old authored generators emitted102-byte v0 mvhd payloads rather than100.
Only their reserved-field width and generated fixtures changed;all old Rust
assertions,packet/work/memory thresholds and source-selection checks remain.
These stand repairs add zero IDs. The remaining failures exposed the independent
metadata-probe and concat-decoder-dispatch defects documented in sibling folders.
All recorded final integration/build gates subsequently passed.

Ordinary regressions need no generation,FFmpeg or network. Exact offline fixture
regeneration is recorded. Actual native software decode/seek is tested;no physical
GPU/audio,new window-duration or benchmark claim is made.
