# QuickTime preview prefixes

Accepted as FVID-0435. A valid QuickTime movie can begin with its `pnot` preview descriptor or `PICT` preview atom. The common MP4/MOV recognizer omitted both, even though the indexed parser already handles these top-level atoms. The authored witnesses use no `ftyp`; the parser and software decoder already reproduce exact packets, source clocks and analytic picture planes before the change. The native dispatch instead reports an unrecognized format; automatic owned probe likewise refuses while explicit MOV probe succeeds.

The preview is a newly authored 36-byte QuickDraw version-1 picture: 16x16 frame, full-frame clip region, black paintRect, three NOP bytes and endPic. The pnot descriptor uses version0, type PICT and first preview index1. No picture, source media, codec parameters or example hex were copied. Existing authored three-color VP9 frames are reused; chunk offsets move with the new prefix. `preview-structure.json` independently checks the actual atom order and every preview instruction. `independent-packets.json` records an optional ffprobe check of all15 packets and clocks in five valid movies. FFprobe and network are not used by fixture generation or ordinary regressions.

The only production change adds the two standard atom names to the existing shared prefix predicate. It does not accept a movie without the normal indexed-parser validations, decode standalone PICT images or add preview rendering. Three previously recognized order controls, both new movie prefixes, four separate malformed/no-movie refusals and existing JPEG/PNG/GIF-specific refusals remain explicit. The test checks both parser APIs, actual software VP9 pictures, native stream playback, rewind, EOF, seven seek positions per valid movie and automatic/explicit probe parity. No public API, performance or hardware claim.

Original e73192bb passes2 checks and fails3 for the exact dispatch/probe/error-precedence omissions. With the fix all5 pass. Full qualification results are retained alongside this report; acceptance is recorded only after all required checks finish.

Primary format references:
https://developer.apple.com/documentation/quicktime-file-format/preview_atom
https://leopard-adc.pepas.com/technotes/qd/qd_14.html

Qualification:102 integration suites pass491 checks; root MP4 passes57 and media547. Excluding3 common included clock tests leaves1092 distinct passes. Five focused repeats are excluded. Both builds pass; the existing physical-device and RGB acceptance ignores remain unchanged. Source/test/generator/manifest hashes match the qualified snapshots. Archived logs normalize only trailing whitespace/newlines; raw hashes are retained.
