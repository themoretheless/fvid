# EOF wall-time clock anchor (accepted FVID-0483)

The published source 07ac749d4c327509df7ae2816b9a2227fb2d92ce correctly clips all PCM when seeking past unedited PCM/ALAC/IMA4 audio, but returns the earlier last coded packet timestamp to AudioPlayback. Its flush/seek anchor then reports 40 ms instead of the real 48/60 ms endpoint. Raw packet seeking is an intentional, separately asserted contract.

`before-final.log` uses the final EOF test: full independent PCM/video controls pass; two navigation tests fail with 16 actual-worker endpoint discrepancies and eight direct wall-time anchor discrepancies. The earlier initial/expanded logs document test-stand corrections (AAC capture clock rounds each component independently; changing PCM already returns its end marker); they are not accepted defects.

The fixture named `recovered.mp4` is a historical label: it is exact PCM on a coarse 1000 Hz media grid, not the recovered-clock path. Other controls cover AAC, edited PCM and changing descriptions. `fixture-verification.json` proves exact offline regeneration of all 15 authored files. PCM/IMA expectations are analytic; AAC uses the existing synthetic stored oracle. No private media, FFmpeg or network is required.

The fix returns the known endpoint only when wall-time navigation has already placed the unedited PCM floor at/past it. It retains coded-packet navigation and decoding validation. Actual worker tests verify no emitted PCM, position at EOF and successful reseek to zero. There is no physical audio-device qualification claim.

Final shared qualification lives in ../mp4-aac-subtick-seek/qualification-results.json. All final qualification results are successful.

Final acceptance: batch-480-validation.json; all seven qualification gates and three repeated seven-test runs pass. The owned temporary build target was removed after all processes terminated.
