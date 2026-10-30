# Wide MP4 decode clock with signed composition offsets

`tfdt` V1 supplies an unsigned64-bit decode time; `trun` V1 supplies a signed32-bit composition offset. The in-memory sample already represents DTS asu64 and PTS asi64. A DTS of2^63 with offset-1 yields PTSi64::MAX, and offset-i32::MAX-1 yields another representable PTS. Narrowing DTS before adding the offset incorrectly refuses both files.

The authored witnesses retain one existing colored16x16 VP9 picture and use a1GHz media timescale. Their file sizes remain small. No private media, codec parameters, FFmpeg or network is used. The generator and ordinary tests are separate. The control starts atzero. An explicit DTS2^63 with zero offset checks actual PTS overflow refusal.

Original ec61d3669b passes the overflow refusal but fails both acceptance tests: parser admission and native picture/seek. The fixed arithmetic composes in i128 then validates the resulting i64 PTS. Packet span and unsigned decode-end checks remain in place. The regression compares exact DTS, PTS, duration, packet bytes, decoded YUV planes, repeated seek and EOF in both parser ownership paths and the software video reader.

Qualification passes908 distinct tests:305 in58 integration suites,56 MP4 root units and547 owned-library units. Locked/offline core/player builds pass. Two physical-hardware tests are explicitly ignored. FVID-0421 counts this one independent arithmetic problem. This is not a performance or whole-project/hardware qualification.

Broader root-unit qualification exposed four pre-existing failures. Re-running the original production parser reproduced the same four names (52 pass,4 fail). Three assertions assumed one raw timestamp representation although AudioPacket specifies a rational clock. They now compare exact cross products with positive numerator/denominator, retain byte-exact PCM/frame counts, and check every packet after seek. The implicit-base refusal was stale after earlier format support: its accepted complete index and all payload bytes now match the explicit-base control, while all invalid flags/run/span/init-only refusals remain. These test corrections are uncounted.
