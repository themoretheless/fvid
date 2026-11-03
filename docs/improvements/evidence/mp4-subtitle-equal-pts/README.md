# Rejected audit candidate — zero accepted changes

The proposed co-timed subtitle witness does not reach subtitle extraction: the existing MP4 STTS checks reject its nonterminal zero-duration runs as `invalid decode-time run`. The explicit-duration controls passed. This result does not prove a subtitle endpoint defect, so no parser or subtitle behavior is changed and no ID is counted.

`rejected-generator.py`, `rejected-test.rs` and `rejected-manifest.json` are literal diagnostic snapshots, not ordinary tests or an accepted regression. For isolated historical replay on the base commit recorded in audit.json, restore the first two to scripts/generate_mp4_subtitle_equal_pts.py and tests/mp4_subtitle_equal_pts.rs, respectively, run generation explicitly, then execute the test command recorded in audit.json. The media are entirely authored, with no private input. The snapshots preserve why this candidate was rejected; they are outside automatic test discovery.
