# Packed MP4 language audit (not yet accepted)

The unchanged parser masks each 5-bit component but only rejects zero. Thus the QuickTime unspecified word 0x7fff becomes three DEL characters, nonletter components produce punctuation/control characters, and the reserved top bit can alias an otherwise valid language.

Format reference: https://developer.apple.com/documentation/quicktime-file-format/language_code_values . Packed ISO codes use three lowercase letters in the low15 bits, with the top bit zero; 0x7fff is unspecified. Legacy Macintosh numeric-code mapping is a separate capability, not implemented by this audit.

The independent generator authors48 short VP9 MP4 cases, including mdhd v0/v1, valid eng/rus, und, zero, unspecified, reserved bit and each nonletter component position. Independent ffprobe evidence checks all packet hashes and raw PTS. Offline Rust regressions check root/owned readers, probe metadata, actual video frames, seven seek positions, rewind and EOF. No FFmpeg/network dependency in ordinary tests.

Baseline session31986 finished exit101: actual playback passed, metadata failed specifically on the unspecified code (three DEL characters). The fix rejects reserved-bit and nonletter packed codes. All13 focused/related checks now pass (2 new +8 extended-language +3 existing metadata). Expanded qualification session24059 is running. No ID has been accepted. Existing437 accepted changes remain the published baseline6f1944608bc0f79efc1c75e0d497f3c54edee237.

Final: accepted FVID-0438. Corrected114-suite integration539 pass; MP4 57, WebM67, media547 pass, with4 shared events excluded=1206 distinct passes. Both locked/offline build configurations pass. All55 fixture files regenerate identically after jobs terminate. Original two ignores retained.
