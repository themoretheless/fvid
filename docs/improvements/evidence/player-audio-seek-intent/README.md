# MP4 audio pending navigation intent

Accepted FVID-0469 after all9gates and20repeated final runs passed. Seven offline regressions exercise actual native MP4 PCM decoding with a controlled delayed output flush. FVID-0469 fixes one pending-navigation clock problem;direction/paused/restart variants and source controls add no IDs.

The source is an authored200ms mono8kHz16-bit PCM MP4 with1600indexed sample frames and an independent scalar F32 oracle. A separate short five-frame16x16 VP9 movie has exact YUV/PTS controls. No private media,FFmpeg,encoder or network. Generator execution is separate from normal tests.

A bounded Condvar gate delays the first actual backend flush while the worker retains its old25ms/125ms position. A release guard drops before Player even after assertion failure,preventing worker join hangs. Successive relative jumps must compose from pending user intent rather than the stale device position;the actual device clock must stay actual until navigation is processed. After release,tests verify real flush targets,exact decoded PCM suffixes,paused resume and natural EOF. Fractional75ms+1ns intent must release to the acknowledged75ms MP4 sample anchor,then follow the actual100ms clock rather than freezing forever.

Headless egui and real native audio/video workers qualify behavior. No physical sound device,new native-window duration,hardware-codec or performance claim. Existing native viewport-close limitation remains at ../player-ab-clock/native-smoke/finding.json.

Baseline:2pass5fail;candidate:7pass0fail on the unchanged seven-case source hash. Twenty repetitions pass140case executions. All9gates pass1596physical-source-distinct checks and both builds;five old ignores unchanged. The initial candidate failed compilation because private fields were missing from legacy fixtures;the failure is preserved separately and excluded from passing checks. All22Worker and2mock-handle initializers were completed without changing assertions or bodies(existing-test-integrity.json). No unsupported performance or hardware claim.
