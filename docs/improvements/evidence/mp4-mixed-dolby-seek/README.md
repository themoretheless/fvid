# Aligned-clock mixed Dolby native seek

This is a zero-count extension of FVID-0484, not a new independent improvement.
The new movie timescale (14,112,000 Hz) is divisible by every native rate, so
packet-clock recovery correctly leaves container packet points unchanged.
Previously, mixed AC-3/E-AC-3 descriptions excluded the entire movie from the
precise native Duration seek route. An integer container-tick floor therefore
lost the remaining fractional request before native sample rounding.

The original source passes complete playback, neutral VP9 output, exact reset
controls, and raw tick reseek. It fails precise suffix/device anchor tests.
`baseline.log`: 3 pass, 2 fail. `baseline-expanded-fixed.log`: 4 pass, 3 fail,
including a first seek before playback. `baseline-expanded.log` is a test-stand
u64/u128 multiplication compile error; it is not a product defect.

`candidate.log`: 7 pass. `candidate-expanded.log`: 10 pass, including fresh
worker seeks, exact selected-programme clocks, missing-programme refusal,
full-packet budget refusal, and successful AC-3 CRC concealment. These focused
results alone do not qualify the full change. Qualification logs and the
source-stability report must also pass before acceptance/publication.

Independent constant-description controls have exact native cursor timestamps
and provide reset decoder PCM for each unedited mixed group. Raw rational
comparison never converts packet timestamps to floored nanoseconds. The worker
capture retains the actual backend packets/timebases and flush anchors.

Generation is standalone Python and deterministic: `regeneration.json` proves
31 files identical, including manifest. Normal Rust tests only read fixtures;
they do not generate media, invoke FFmpeg, or access a network. Authored spectra,
synthetic tone, PCM and neutral short VP9 video contain no private media/params.
CRC-bad AC-3 is explicitly an acceptance test for supported concealment;
missing E-AC-3 programme and full packet budget are refusal controls.

No benchmark, speedup, hardware audio-device, or visual-player claim is made.
The actual playback worker is exercised with an in-process capture backend.

Final qualification: all seven gates pass,1078 test executions,two old ignores.
Final ten-test stand passes in focused,integrated and repeat runs. Source hashes
remain unchanged through qualification and repetition. Accepted as batch522,
zero-count extension of FVID-0484;counter512,next FVID-0513.
