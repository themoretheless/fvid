# FVID-0533 — AVI ADPCM delayed stream origin

Base: 189d3e01606030108f44156595749c4840fe6dbd.

The baseline (`before.log`) accepts authored zero-origin controls but rejects nonzero AVI ADPCM dwStart explicitly. The candidate converts stream start units into native sample origin, preserves independent block seek alignment, and reports absolute endpoint duration and EOF. PCM/G711 record clocks remain unchanged. The old unsupported-start unit expectation is replaced by packet, seek and endpoint acceptance assertions.

AVI stream start is expressed in dwScale/dwRate units:
https://learn.microsoft.com/en-us/windows/win32/api/vfw/ns-vfw-avistreaminfow

The pure Python generator authors 144 IMA/MS mono/stereo cases with original/equivalent clocks, single/grouped/nested/indexed records, and starts 0, 1, 73 and u32::MAX. All 175 fixture files regenerate byte-identically. Huge origins are logical timestamps; the reader does not insert silence for the prefix.

Validation: after.log contains 12 passing integration tests, including 432 actual software audio-worker sessions, literal PCM, native clocks, seek/rewind/EOF, malformed records and budgets. unit.log contains 13 AVI audio unit passes; container.log contains 17 AVI parser unit passes. check.log records the player configuration build check. Ordinary tests require neither FFmpeg nor network. Physical audio output and AVI video playback are not qualified by the neutral MP4 video control.

Accepted as FVID-0533 in batch 544; registry count 533. The final baseline log and JSON repeat the unchanged final four-test stand on the original runtime: two controls pass and two delayed acceptance tests fail on the specific start-origin refusal. The candidate is restored with guarded source hashes after this comparison.

Final qualification logs contain 157 passing Rust test executions and two pre-existing physical-device ignores, plus successful core/player cargo checks. This includes 14 AVI audio units, 17 AVI parser units, the native player factory (18 delayed equivalent-clock cases), 39 audio-worker tests, 43 CPAL tests and 27 related integration tests; the final acceptance and three external-tool-free repeats add 16 executions. The maximum origin storage test retains three logical block index entries, no PCM span vector and a coded-file capacity at most 8192 bytes. This verifies the reader representation, not process RSS. Malformed files must return typed Invalid with the expected whole-block/count reason.

All175 fixture files regenerate byte-identically and all4392 candidate-source hashes remain stable. candidate-source.json is the final freeze; source-hashes.json records the earlier published implementation. No performance claim.
