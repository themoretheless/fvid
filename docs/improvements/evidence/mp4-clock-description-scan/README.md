# Compact MP4 playback clock setup (accepted FVID-0485)

PlaybackClock::build checks each audio description for actual use before rate negotiation. It walked every virtual sample for each unused entry even when a sparse UniformIndex already identifies descriptions per chunk. This is a playback-open path; FVID-0482 optimized a separate controlled planning header comparison and did not change this loop.

Nine authored movies cover 64/262144 mono PCM frames, 0/1/32 unused identical headers, selected duplicate entries, unused different headers, and real endian changes. The maximum authored audio duration is5.462s, with three neutral VP9 pictures. Analytic PCM is exactly0.125. No private input,FFmpeg or network is used. Two functional regressions pass on the original production code, including actual-worker small controls, rewind, one-native-sample seek and EOF.

The candidate queries readable run starts for sparse compact indexes, preserving the reference per-sample walk for dense/expanded indexes. Private index regressions compare results against the reference query for normal, gapped, shadowed, empty, out-of-count and overflow-addressed runs, plus expanded/dense indexes. No public API or codec behavior changes.

The opt-in release harness times only Mp4AudioReader::open over resident immutable bytes. Reading/copying input, fingerprints, decoding and drops are outside the measured interval. All iterations check identical sample counts, description counts, geometry, duration and three sample coordinates. Default release profile and the same toolchain are used for both binaries. Predeclared criteria are in measurement-plan.json.

An initial benchmark macro-expression compile error is retained in before-build.log; it was repaired before either baseline binary or production edits. It is not an improvement. Initial single baseline probe median16.204416ms is historical, not the acceptance comparison. Paired set1 uses seven alternating-order rounds, nine measured opens plus two warmups per process. Primary median51.291ms ->8.23875ms,83.937percent reduction,7/7wins; all fingerprints agree. The single-header control shows no regression. Background host load is recorded with every round; absolute times are workload/load-specific. No file-I/O,decode,whole-player,UI,RSS or GPU speedup is claimed.

Acceptance required all eight correctness/build gates and a second complete paired measurement set with the same criteria. Both passed; all observations remain in evidence. Fixtures and proof are retained, while the owned target and both benchmark binary copies have been removed after terminal verification.

Only description-usage queries in playback clock setup are optimized. The existing constant-description guard and other opening work remain; total opening is not claimed to be bounded only by the run count.

Final acceptance: batch-483-validation.json. All eight gates pass864 test executions with two unchanged old ignores;three repeats pass15 tests each. Replication median18.172958ms ->2.76375ms,84.791964percent reduction,7/7wins;control does not regress,all fingerprints match. Both required sets pass. The owned target,including both binary copies,was deleted only after every process terminated. Count485,next FVID-0486;goal1000 remains active.
