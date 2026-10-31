# Positive partial terminal MP4 audio window

Status: accepted FVID-0448; all six qualification gates terminal0. Registry448/1000.

Baseline6c44f835169e11f73f74866f9bd15f2ea8af900d accepts an authored three-packet ALAC video with160,160,40 source ticks at8kHz. It reports360 native frames but publishes480; seeking to the exact endpoint publishes120 extra frames. Coded audio packet bytes match the full-block control. The separate IMA4 witness already behaves correctly and is only a control.

The baseline test run has2 passing controls and1 failing PCM/rewind/seek/EOF acceptance. The initial log stopped at the first EOF assertion; the qualified before run aggregates all discrepancies without removing that requirement. before-test.rs.txt records that qualified test version. An actual AudioPlayback worker acceptance was subsequently added; it is not claimed as a separate baseline test or improvement.

The candidate clips decoded non-AAC/changing PCM to the exact source presentation endpoint before floor trimming, after codec validation. Ceiling native-frame coordinates preserve phase. Recovered clocks retain their existing earlier path, and declared empty tails remain represented. No performance claim.

Four authored short VP9 videos and four independent scalar PCM oracles are committed with their generator. No encoder, private media or network is used. Ordinary tests consume committed fixtures only. All files regenerate byte-identically. Full-block controls, raw audio timing and unchanged coded packets, decoded companion video, exact PCM, rewind, sample seek, EOF seek, and actual worker are checked. Focused after:21pass across5suites. Full125-suite integration, root MP4/WebM, media units, and core/player builds passed under the recorded locked/offline plan:125suites584passes;58rootMP4,67WebM,547media passes;1252 distinct after4duplicate events. Existing ignore policy remains unchanged.
