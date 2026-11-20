# AAC-ELD access-unit origin

Twelve own eight-AU programmes cover 480/512-sample mono/stereo cores with
activation only in the first AU, only in the last AU, or in both. Each has a synthetic MP4 video
and an audio-only Matroska control. No private or ISO media/parameter sets are used.

The previous synthesis retained the identically zero first quarter of the
ELD window, delaying every returned frame and dropping a known final quarter.
The independent oracle superposes complete four-block cosine/window impulse
responses on a global time axis. It selects the causal AU interval, including
the nonzero final quarter, without decoding, trimming a decoded file or adding
padding. The next AU cannot contribute there because its leading quarter is zero.

`tests/aac_eld_au_origin.rs` checks full PCM in both containers, immediate complete
frames, checkpoint replay/reset and the nonzero EOF quarter. The actual worker
regression checks playback timestamps, seeks and rewind. Both PCM tests fail
specifically on the old origin and pass with the correction.

Generate explicitly with `python3 scripts/generate_aac_eld_au_origin_fixtures.py`.
Normal tests consume these frozen assets and require no generation, network,
FFmpeg or foreign codec.
