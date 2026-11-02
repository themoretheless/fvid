# Seek into a concealable Dolby packet before first playback

Zero-count correction of batch522/FVID-0484. This is an implementation
regression, not an additional independent improvement toward1000.

The selected future E-AC-3 description starts with a good frame, then one
explicit bad-CRC frame, then another good frame. Complete playback and a seek
after complete playback pass: the middle frame is successfully concealed with
silence. On source0265b5d33174c317aaa89ef5f5c963b3d9e07461, a first seek into
that middle packet fails because its CRC-checked header is used as the only
source of native rate before the description has negotiated its decoder.

`baseline.log`: two passing acceptance controls, two failing fresh-seek tests
(reader and real audio worker), specifically `missing selected E-AC-3 native
seek rate`. `baseline-source.json` binds the source/test/fixtures to the run.
The tests are unchanged after the fix. `candidate.log`: all four new tests plus
all ten batch522 tests pass. Those focused results alone are not full
qualification; inspect the complete gate results and source stability before
acceptance/publication.

Six authored classic/fragmented/hybrid movies cover programme0 at48k stereo
and programme1 at44.1k with three native channels and placeholder48k metadata.
Standalone selected-programme controls prove the middle1536 samples are zero,
neighboring frames are nonzero and whole mixed PCM/timestamps are exact. Short
neutral VP9 video, warm/fresh seek, raw reset and actual worker flush/packet
anchors are checked without normalizing rational timestamps to nanoseconds.

The fix shares existing description setup probing/cache with native-rate
selection without activating the future decoder. Checked setup remains the
source of native rate/layout; corrupt setup and missing programmes retain the
same refusal path. The selected playback packet can still use the decoder's
existing CRC concealment. Full-packet limits and setup bounds are retained.

Generation is standalone, authored and deterministic;`regeneration.json`
proves nine byte-identical files including manifest. Only the selected second
frame's CRC trailer bit is invalid. This is a specific concealment regression,
not a mutation/fuzz audit. Normal tests read committed synthetic fixtures and
require neither FFmpeg nor network. No private media/codec parameters are used.
No performance,physical audio device or visual-player qualification is claimed.

Final acceptance: batch523,zero new IDs. Seven gates pass1082 test executions,
two prior ignores,both builds pass. Three additional four-test repeats pass.
All frozen source/fixture hashes remain stable;counter512,next FVID-0513.
