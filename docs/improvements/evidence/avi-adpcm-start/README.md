# AVI ADPCM delayed stream origin — saved qualification work

Base: 189d3e01606030108f44156595749c4840fe6dbd.

The baseline (`before.log`) accepts authored zero-origin controls but rejects nonzero AVI ADPCM dwStart explicitly. The candidate converts stream start units into native sample origin, preserves independent block seek alignment, and reports absolute endpoint duration and EOF. PCM/G711 record clocks remain unchanged. The old unsupported-start unit expectation is replaced by packet, seek and endpoint acceptance assertions.

AVI stream start is expressed in dwScale/dwRate units:
https://learn.microsoft.com/en-us/windows/win32/api/vfw/ns-vfw-avistreaminfow

The pure Python generator authors 144 IMA/MS mono/stereo cases with original/equivalent clocks, single/grouped/nested/indexed records, and starts 0, 1, 73 and u32::MAX. All 175 fixture files regenerate byte-identically. Huge origins are logical timestamps; the reader does not insert silence for the prefix.

Validation: after.log contains 12 passing integration tests, including 432 actual software audio-worker sessions, literal PCM, native clocks, seek/rewind/EOF, malformed records and budgets. unit.log contains 13 AVI audio unit passes; container.log contains 17 AVI parser unit passes. check.log records the player configuration build check. Ordinary tests require neither FFmpeg nor network. Physical audio output and AVI video playback are not qualified by the neutral MP4 video control.

This saved work is deliberately not yet counted as FVID-0533. Complete final qualification and evidence review before changing the accepted registry (currently 532). No performance claim.
