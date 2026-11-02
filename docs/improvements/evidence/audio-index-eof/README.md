# Indexed audio payload EOF — FVID-0516

Accepted batch527:one common problem across8 payload adapters,not an ID per
format,codec or fixture. Total516/1000.

## Specific before failures

Original f87769a3c2 returns the final packet again for an index >= packets().
WAVE/AIFF/AU baseline passes2 normal-playback/video controls and fails3 separate
EOF tests:each one-frame file repeats2 bytes at index1. After auditing related
APIs,the same cause was reproduced for AAC(root/owned),FLAC,MPEG audio,Dolby
and XM. Coded baseline passes3 decoded-tone/actual-worker/refusal controls and
fails6 EOF tests. Coded accessor source hashes match the original commit;the
previously checked PCM candidate was already present during that second audit.
See both baseline-source.json files and baseline.log files.

This is an indexed-accessor bug. Ordinary AudioStream guards already reach EOF
and pass actual-worker controls before the fix;no prior ordinary-worker failure
is claimed. A consumer stopping on empty indexed payload cannot terminate when
the last payload is implicitly repeated. XM similarly synthesizes another tail.

## Correction and compatibility

The8 payload adapters now return empty bytes at/beyond their declared count,
including usize::MAX. Bounds are checked before offset math or XM allocation.
All valid payloads remain identical. XM packets() includes its one valid release
tail,which remains nonempty. Time/seek selectors and the private AAC timestamp
planner are unchanged. See API-MIGRATION.md for the deliberate out-of-range
behavior change and explicit last-valid-index selection. No signature changed.

The18 authored PCM cases span1/2048/2051 frames,mono/stereo and WAVE/AIFF/AU.
Eight coded/tracker cases cover AAC,FLAC,MPEG layersI/II/III,AC3/EAC3 and a
from-scratch silent XM row. Valid indexed payloads match stream bytes;PCM has
independent mathematical levels,and silent MP1/XM have exact independent zero
and frame-count checks. Tone cases must decode finite,nonzero PCM. Actual worker
captures preserve exact PCM/native timestamps and anchors. Both audio-only
corpora have separate authored neutral VP9 controls;those videos do not pretend
to reproduce an audio accessor failure. No private media/codec parameters copied.

## Uncounted discoveries and corrections

The first Rust harness had an ambiguous Vec element type;harness-compile.log
retains that compiler failure. It is not a media failure. The final original
baseline proves the actual EOF defects after the harness correction.

The first encoder FLAC used a pipe and carried a provisional incorrect nonzero
frame-size claim. It was refused before EOF and was not accepted as an EOF
reproducer. Seekable generation finalizes its STREAMINFO. The original pipe
bytes are retained byte-identically as a separate specific refusal control;
see ../coded-audio-index-eof/flac-fixture-contract.json and the initial logs.
Parser checks were not relaxed. RFC9639 section8.2 permits refusal when
STREAMINFO is incorrect/incomplete:
https://www.rfc-editor.org/rfc/rfc9639.html#section-8.2

Extended qualification also found a pre-existing native_aac_media assertion
requiring metadata-free WAVE length. Restoring all8 changed production files to
the original commit reproduces exactly the same failure. Both original and
candidate files have identical PCM;34 extra bytes are intended LIST/INFO/ISFT
source-encoder metadata(FVID-0269). The strengthened test requires exact PCM
size,declared RIFF size,an independently serialized expected metadata suffix,
exact full size and existing rate/fact/stereo/audibility checks at16/48kHz.
No exporter code changed and no extra ID was counted. Original comparison,
initial expanded qualification and source hashes are retained. Initial PCM-only
qualification is archived separately rather than presented as final coverage.

## Final qualification

All14 gates pass826 Rust test executions,one pre-existing ignored,and core/player
checks. Three repeats pass24 tests each. Two new executables also pass14 tests
with PATH=/usr/bin:/bin,where FFmpeg is unavailable. Generation is standalone:
PCM,MP1,XM and neutral video are authored;public FFmpeg encoders only generate
six short tone files outside test execution. All37 files regenerate identically.
All2734 hashes stay stable throughout final qualification. Exact commands and
logs are retained. No performance,physical device or visual-UI claim is made.
