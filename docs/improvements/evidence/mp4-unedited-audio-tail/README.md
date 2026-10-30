# Uncounted unedited MP4 audio tail follow-up

Three contiguous authored AAC/PS/PCM/ALAC packets followed by a duration-is-empty fragment declare one additional packet-length source interval. No elst exists. The existing owned decoder renders the interval; native playback before this fix publishes only coded PCM despite reporting the full duration. Final original-production regression:1 control passes,3 acceptances fail.

Route the unedited declared-tail case through the existing variable source/silence scheduler. Its source packets drain first, then bounded zero chunks preserve the declaration. No codec history, container index or public API is changed. Tests verify all source DTS/PTS/durations and actual VP9 frames, independent PCM and literal silence, seek, rewind, EOF, actual audio workers and a physically short two-day tail without duration-sized allocation.

A legacy presentation-end regression intentionally measured only the coded PCM of its two empty-tail witnesses. Its expected output is now the same original PCM plus160 literal mono zero frames; seek suffix equality is retained. Every old media/oracle payload is unchanged, and both generators reproduce their checked-in outputs offline. The original manifest and the failed old-oracle run are retained. This is acceptance of the intended tail behavior, with no removed checks or relaxed tolerances.

Qualification:309 passing checks in59 integration suites,56 root MP4 units, locked/offline core/player builds. One physical-hardware test is explicitly ignored. The owned-library production is unchanged from its earlier547 passing tests. No new improvement ID is counted; this completes another variant of FVID-0419. This is finite evidence, not whole-project or hardware certification.
