# Uncounted coarse-clock tail follow-up for FVID-0420

Two additional authored PCM/ALAC plus VP9 controls combine a 1001 Hz media clock, 8000 Hz native samples, an explicit middle gap and a final empty fragment. The final source packets nominally end at80/1001 seconds, but their recovered native end is40/1001+0.040 seconds. The final declaration ends at100/1001 seconds.

On published f8ba7d59fb6ea9fc0d5c405777efc60902d79036, the new test fails with799 rather than800 native samples. Splitting the tail at its nominal source start carries the previous packet's rounded end into the silent segment and discards the first silence sample. Source/fixture hashes are in before-source.json. The initial reproduction stops at the PCM length assertion; subsequent assertions additionally check ALAC, exact duration, native seek suffix, EOF and the actual playback worker.

The fix splits trailing silence at the recovered last packet endpoint when a recovered clock exists. The declared overall endpoint remains unchanged. Ordinary tests use checked-in authored data and require neither FFmpeg nor network. Generator determinism is checked separately. This is an interaction within the already accepted middle-gap timeline correction; no new improvement is counted.

Qualification passes 256 tests in43 integration suites,14 root units, and locked/offline core/player builds. One physical-hardware test is ignored explicitly. The unchanged owned-library production retains its earlier547 passing checks; those are not rerun or recounted here. This is finite subsystem evidence, not whole-project or hardware validation.
