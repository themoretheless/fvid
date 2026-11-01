# Edited MP4 concat packet-work admission

Accepted FVID-0479. All seven final qualification gates completed successfully.

Published source 5838dc29e4 uses the indexed sample count for at most one media
edit and u64::MAX for repeated edits. Neither is actual decode work. A finite
edit can present only one of three indexed PCM packets; repeated edits can
replay packets and AAC can restore a retained source-boundary checkpoint.
Subtracting the inaccurate count falsely exhausts the remaining concat budget
and skips admission of a later segment that execution actually reaches.

The final identical six-test published-source baseline is four passing controls
and two failing acceptances. Both modes (unbounded controlled allocation and
64 MiB) reproduce the error for the minimal PCM and fifteen authored
PCM/ALAC/AAC combinations: early end, repeated media, preroll, repeated preroll
and an empty leading interval. Thirty-two plan variants falsely accept a
reachable packet above its byte cap. Execution correctly refuses without a
published file. This is plan/execution correctness, not an execution quota bypass.

The correction retains cheap raw index counting for unedited tracks. Edited
sources use the existing MP4 decoder with all packet, cancellation, RSS and
controlled allocation admission retained. PCM goes directly to io::sink;
progress publication is disabled and the previous reader/index is dropped
before opening the bounded decoder. Actual decoded-frame work matches concat
execution, including edits, preroll, checkpoints and supported normalization.
No public API or codec support changes. Planning edited sources now performs
additional bounded decoding and can cost more CPU; no performance improvement
or latency qualification is claimed.

Forty-three committed authored files regenerate exactly without FFmpeg, network
or private media/codec parameters. PCM expectations are analytic or copied
from prior independent authored reference oracles. Ordinary tests read bytes
without running a generator. Controls verify complete source PCM and packet
work, exact budget exhaustion, healthy complete two-segment PCM publication,
and actual native software pictures, seek, rewind, EOF and edited audio from
a short VP9/PCM hybrid. All variants count as one independent correction.
No physical audio/GPU or native-window duration claim.

Historical initial two- and three-test baselines isolate the same minimal
failure. One control expansion used decode-only fields on CopyStats and failed
to compile; it is a corrected stand assumption, not a media failure or an ID.
The final baseline restores the published production file and then restores the
candidate in a guarded finally block. Final test/generator hashes are identical
for before and qualification. Superseded initial logs are history only.
