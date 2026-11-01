# MP4 seek crossing an occupied edit frontier

Three authored MP4 movies combine AAC LC1024 stereo48kHz and Main960 mono32kHz with a first edit lasting1000010ns and a second lasting1ms. Classic,reordered descriptions and fragmented variants use an exact14112000Hz media clock. The independent scalar AAC IMDCT/overlap references prove the full49-frame stereo plus31-frame mono presentation and precise suffix waveforms;each clip also decodes sixteen neutral VP9 pictures. All six movie/reference/manifest files regenerate identically without FFmpeg,network or private media.

Published aef465fb75db78501c794a0eb97b18112732299f passes independent full playback and raw-navigation compatibility but fails direct and actual-worker clock regressions. Seeking at1000001ns or1000009ns correctly emits the second edit's PCM yet returns1000010ns/flushes at1000010ns,earlier than the first available sample at1031260ns. The first edit's last native sample occupies time until49/48000 seconds;continuous playback carries this frontier into the second edit. This is a missed frontier after segment traversal,not the previously repaired initial Duration-to-tick loss:exact rational requests already reach native alignment.

Reapply each segment's precomputed minimum start whenever native rounding traverses into another segment,then derive the new segment's native anchor. Existing PCM clipping,preroll,format changes,source extent checks and raw tick semantics remain. No public API change. The final focused test passes all four checks without changing the baseline test or fixture bytes. Wider qualification and repeat evidence are recorded separately;acceptance requires all gates terminal and green.

Scope:edited native-sample seek transitions and actual software AudioWorker backend flush anchors. No performance,physical audio hardware or complete MP4 audit claim.

Accepted FVID-0486 in batch-487-validation.json:seven gates pass880 executions,three repeats pass33 tests each. Two pre-existing ignored tests remain. Owned build target removed after all handles completed. Counter486,next FVID-0487.

Parallel main advanced to509 entries before delivery. Both original ID486 and batch487 were already occupied;this local fix is renumbered to FVID-0510/batch511. Standalone qualification is preserved in standalone-qualification. Combined-state qualification must pass before acceptance or push;no parallel entries are removed.
