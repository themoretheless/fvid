# AAC wall-time seek below media ticks (accepted FVID-0484)

The published source at 07ac749d4c327509df7ae2816b9a2227fb2d92ce loses the fractional media tick in `seek_time`. Six authored AAC960/neutral VP9 controls cover 48 kHz and 50 Hz media scales, finite 1/1.25 ms single edits, nonzero edit origins and elst V0/V1. Independent stored synthetic PCM validates the entire intended clip before seek assertions.

`before.log` and `before-worker.log` prove extra/replayed samples, including complete clip replay at its end on the 50 Hz grid. Worker and direct tests assert exact PCM suffixes, sample counts, first PTS, no PCM at EOF, exact endpoint anchors and raw navigation recovery. No FFmpeg/network is needed.

The fix retains the existing decoder preroll path, remembers an exact rational clipping floor and anchors the wall clock on the selected source packet's native sample lattice. It leaves raw tick navigation intact. `initial-anchor-phase.log` records a rejected intermediate implementation that incorrectly aligned on a zero-origin lattice; the existing authored fractional-composition fixtures now exercise clock anchors too. This is a repair within the same candidate, not another counted improvement.

`after-phase-focused.log`: four AAC tests and three EOF-clock tests pass. The earlier integration log covers the first implementation only. Final qualification is recorded separately in `qualification-results.json` and `final-integration.log`; all seven reviewed results pass. No speedup or physical audio-device qualification is claimed.

The first expanded qualification exposed an intermediate regression on explicit empty terminal AAC windows (`integration-zero-window-failure.log`). Anchor selection now skips zero-duration windows while ordinary decoding continues to validate their payloads. The unchanged `mp4_audio_zero_window` regressions must pass in final qualification. No additional ID is assigned for this candidate repair.

Final acceptance: batch-480-validation.json; all seven qualification gates and three repeated seven-test runs pass. The owned temporary build target was removed after all processes terminated.
