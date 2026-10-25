# Selected E-AC-3 programme clocks beneath MP4 ticks

Generate separately: `python3 scripts/generate_mp4_eac3_programme_clock.py`.
Normal tests read checked-in bytes without FFmpeg or network access. Generation
reuses the repository-authored neutral VP9 and authored E-AC-3 spectra from
`eac3-mp4-programmes`, with no private media or parameter sets.

There are 522 accepted movie/programme combinations across 900/1000/1001 Hz
media clocks and cumulative floor/ceil/nearest endpoints. The seven base families
cover all eight independent IDs, 48/44.1/32 kHz, heterogeneous layouts, dependent
8-channel presentations, 3/6-block frames, placeholder sample-rate metadata and
unequal packet windows of 768/2304/1536 native samples. Separate single-programme
controls preserve the selected encoded samples and native PCM history.

The parent player puts packet 1 of `dual-48000-900-floor.mp4` at 31.111111 ms
instead of 32 ms. Native count recovery advances only for selected independent
frames; dependent and unselected substreams must not advance that clock.
Corrupted unselected payloads remain irrelevant to the selected programme.

`tests/mp4_eac3_programme_clock.rs` checks intended playback acceptance, native
PCM/geometry/timestamps/duration, five sub-tick seek targets, rewind and the actual
worker with a capturing backend. Missing and corrupt selected-programme tests
are explicit refusal checks, separate from the valid playback matrix.

The matrix does not qualify coarse-clock edits, access units split across MP4
packets, every E-AC-3 coding tool, physical devices or complete codec coverage.
