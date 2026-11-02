# Bare FLAC native-sample Duration seek — FVID-0517

Accepted batch528:one navigation capability,six rate/channel variants. Total517/1000.
This is in-range precision,distinct from FVID-0089 raw EOF exhaustion and
FVID-0063 integer duration publication.

Original7a56ea2bf5 decodes every full mathematical PCM oracle correctly and
passes raw frame-seek,EOF,rewind,budget and neutral-video controls. A Duration
seek at1ns in8kHz mono returns0ns and the actual worker emits native sample0;
the first sample at/after the request is sample1 at125us. Final original baseline
passes2/fails2;the precise-reader and actual-worker failures are retained in
baseline.log with source hashes. The initial resource assertion expected the
wrong error wording;that harness mismatch is retained in initial-baseline.log,
not counted as another media failure.

seek_time now computes ceil(target_ns*rate/1e9),clamped to the native extent
before narrowing. It locates/decodes the containing coded frame but suppresses
PCM earlier than the requested native floor. Published timestamps are exact
sample/rate rationals. Raw seek_to keeps its previous returned frame coordinate
and full coded-frame output. Both raw navigation and rewind clear the precise
presentation floor. No parser/decoder algorithm or public signature changes.

The6 source files use4099 own mathematical PCM16 samples per channel at8/44.1/
48kHz,mono/stereo. Expected f32 values are independently normalized from those
integers;none are decoded from an encoded golden. A standalone generator uses
FFmpeg only for lossless seekable FLAC encoding with finalized metadata. All14
files regenerate byte-identically. Bare FLAC is audio-only:the short authored
neutral VP9 companion is an independent video control,not the audio reproducer.
No private media,frames,audio or codec parameter sets were copied.

Tests cover fractional targets and both sides of native/coded frame boundaries,
EOF,Duration::MAX,raw/precise mode changes,rewind,exact waveform/native stamps,
public time anchors and actual AudioPlayback worker captures. File/packet-count
admission stays strict;related FLAC structural/CRC regressions remain unchanged.
Five final gates pass94 Rust test executions,zero ignored,and core/player checks.
Three repeats pass13 tests each. The new4-test executable also passes with
PATH=/usr/bin:/bin and no FFmpeg. All2750 source/fixture hashes stay unchanged
throughout qualification;exact commands and timings are retained. No physical
sound device,visual UI,other-container FLAC or performance qualification is claimed.
