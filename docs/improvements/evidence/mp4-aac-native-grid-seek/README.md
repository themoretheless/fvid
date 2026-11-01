# Changing AAC exact-grid seek: FVID-0484 follow-up (zero new IDs)

Six authored short MP4 movies exercise AAC LC1024 stereo48kHz and Main960 mono32kHz, finite split edits, reordered and unused opaque descriptions, fragmented MP4, and a complete final AAC packet. The 14112000Hz media grid represents the native packet durations exactly, so clock recovery is not selected. Independent scalar IMDCT/overlap references verify nonzero PCM; all movies decode sixteen neutral VP9 pictures. A separate seventh movie selects unavailable source time and must report an error instead of successful EOF. Twelve fixture/reference/manifest files regenerate byte-identically without FFmpeg, network or private media.

Published cb49847885db262e60a805610289add7c824d31f passes full PCM/video, raw tick compatibility, and unavailable-source refusal controls, but fails both direct and actual-worker precise seek regressions. The final baseline has three passing and two failing tests. At1ns the first sample repeats; at500001ns another native sample repeats; near edit/stream end the backend clock anchors early. Earlier baseline snapshots retain the progressive test-stand history.

Select the existing exact rational Duration navigation for edited timelines whose used descriptions are AAC, rather than requiring every catalog entry to be identical. Unused foreign headers do not choose the path. Existing selected-rate native landing, preroll, clipping, source-extent refusal and raw tick behavior are retained. This extends the already accepted lost-fraction cause FVID-0484, not an independent improvement. Counter remains485,next FVID-0486.

Qualification is recorded in qualification-results.json and repeated actual-worker/seek controls in repeat-results.json. No performance or physical audio-device claim. Mixed codec-family timelines and remaining MP4 timing paths are not certified by this follow-up.

Accepted in batch-486-validation.json. All seven gates pass876 executions;three repeats pass29 tests each. The owned temporary build target is removed after terminal verification. Fixtures and evidence remain.
