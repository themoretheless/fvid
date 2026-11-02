# AVI native seek index selection

PCM and G.711 already preserve exact native seek coordinates, but locating their
physical AVI record walked the sample table linearly. Both constructors build
that table by advancing a checked clock through nonempty records. Consequently
record ends are strictly increasing and the first end greater than the target
can be found with `partition_point`. The existing clamping, within-record skip,
packet windows, EOF state and ADPCM path retain their behavior. No additional
index, cache or allocation is added.

## Baseline and measurement

The original production file is from commitc786604295dd117f89b606ae0d8a009f6f5bce5a.
`before-source.json` records the actual baseline sources,including the new manual
benchmark and fixture setup. All three new correctness tests pass before the
implementation changes;this is a measured CPU opportunity,not an audio defect.
The old default-dev executable is built successfully and copied before changing
the production method. Its SHA256/compiler/build command are preserved in
`before-executable.json`;candidate identity is recorded separately.

The benchmark calls the actual public `AviAudioReader::seek_to`,black-boxing the
mutable receiver,query and result. Parsing and query construction happen outside
the timer. Each file uses520 varied native queries,including boundaries and wide
clamps,with a checksum checked against independently computed expected landings.
Five pairs alternate execution order of the preserved old and new executables.
A second independent five-pair comparison runs after correctness qualification.
`first-comparison` preserves the first results;`comparison.json` and individual
`pair-*` files hold the final series.

The measured configuration is the default dev profile on the compiler/host
recorded with both executables. These are public-method CPU measurements;
packet I/O,decoding and end-to-end player timing are outside the timer. The small
16-record controls can incur roughly12ns extra per lookup. Keep the complete
matrix,including these regressions,instead of claiming every size speeds up.

## Authored files and correctness

The generator writes15 AVI files with neutral VP9 plus PCM16,A-law or mu-law and
independent float32 PCM controls. Profiles hold1,16,1024 or65536 one-frame records,
or16384 irregular1/3/7/2-frame records. Audio lasts at most1.366seconds;nonzero
starts are included. An independently playable neutral Matroska video control is
also stored. AVI video playback support is outside this audio-reader change.

```sh
python3 scripts/generate_avi_seek_index.py
cargo test --locked --offline --no-default-features --features player --test avi_seek_index
cargo run --locked --offline --no-default-features --features player --example bench_avi_seek_index -- 0
```

The ordinary tests use checked-in files,with no FFmpeg/network dependency or
timing assertion. Complete independent PCM/native clock controls,exact first
packet boundaries after dense/irregular seeks,raw/wall-time navigation,negative
and EOF/MAX requests,rewind,repeated EOF and real audio-worker tail/anchor checks
remain separate from timing. Record-budget refusal stays distinct from acceptance
of the same valid file under normal limits. Existing tests and expectations are
unchanged. `qualification-results.json` lists all related gates and return codes.

## Accepted evidence

FVID-0521,batch532,total521/1000. Across both epochs,the30 dense65536-record
pairs improve at least243.718x,the30 irregular16384-record pairs at least94.727x,
and the30 medium1024-record pairs at least8.317x. The16-record controls add
7.13..13.70ns in per-case median lookup cost;the complete matrix is retained.
All old/new checksums match and both epochs use the same preserved executables.
Five qualification gates pass109 test executions,zero ignores,and both
core/player checks. Three repeats pass3 tests each;3 tests pass without FFmpeg.
All32 fixture files regenerate identically and2860 source/fixture hashes remain
unchanged. Public interfaces and existing test expectations retain their behavior.
