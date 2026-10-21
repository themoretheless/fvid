# Native WavPack file regressions

Generate separately with `python3 scripts/generate_wavpack_files.py`.
The generator imports owned synthetic encoders only. Tests read checked files
without FFmpeg, external codecs, processes or network access.

The 1,317 positive cases cover integer, float, multichannel, custom rates,
hybrid main-only and correction pairs, DSD, variable sample windows, unknown
sample totals, 40-bit origins, ID3 prefixes and APE/ID3 trailers. A valid optional
metadata payload ending in `TAG` checks that audio blocks are not trimmed as tags.
Native `.wv` and `.wvc` files reproduce the standalone opening failure;
associated synthetic `.mkv` videos are codec controls, not substitutes for the
native file-opening regression. No private media or parameter sets are used.

`tests/wavpack_files.rs` checks exact scalar PCM, rewind, partial seek, duration,
EOF failures, explicit resource limits and audio-worker output. Read counters
check that opening materializes only the first frame and later indexing skips
entropy bodies. Malformed headers, lengths, sample windows, totals, trailers,
and mismatched/missing/extra correction frames must fail without fallback.
Player unit tests exercise actual automatic dispatch and companion selection,
including uppercase extensions and a malformed existing companion.

Tags are skipped, not imported as title metadata. DSD seeking replays filter
history from the beginning. These fixtures do not establish device playback,
performance or completeness across all WavPack/other codec tools and profiles.
