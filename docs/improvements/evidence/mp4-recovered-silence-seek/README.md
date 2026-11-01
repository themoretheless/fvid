# Recovered native-clock silence seek (FVID-0484 follow-up, zero new IDs)

Three authored short movies expose1ms source/edit windows with neutral VP9 and independent audio. AAC960 on a49Hz media grid requires native48kHz clock recovery: a source gap begins on a fractional presentation phase; a second control has an explicit initial silence edit. The PCM control changes8kHz to16kHz before a later gap. Its first64sample packet occupies8ms; the16kHz63sample packet occupies3.9375ms rather than the rounded4ms. The later gap therefore uses the16kHz lattice although rewind initially configures8kHz.

Expected seek frames/times are generated with exact Fraction arithmetic, independently of the production scheduler. AAC PCM uses existing owned stored references; PCM0.125 and silence are analytic. Full native video,complete PCM,duration and output PTS controls pass on the published eb59debc8a source. No private media,FFmpeg or network is used. Five fixture files regenerate exactly.

`before-final.log` runs the final identical four-test stand on original production:2 controls pass,2 acceptances fail. Eleven direct anchor and eleven actual-worker flush positions are wrong;two further worker EOF positions stay early with no output. The emitted PCM and first PTS already match. Earlier initial/expanded logs retain the two-AAC and added rate-transition audit stages;they add no IDs.

For Duration navigation,the fix aligns recovered source gaps using the preceding coded clock run's native rate,and explicit empty edits using the opening run's rate. The recovered raw tick API keeps its prior declared-grid landing through a private mode parameter. An existing owned900Hz PS-edit control proves raw1tick remains1/900s while its first emitted native sample is later;rewind/reseek keeps exact packets. Non-recovered exact AAC and coded-native landing remain covered by the existing regressions.

`after-raw-control.log` passes all4 new tests. All seven shared qualification gates pass866 executions;three repeated runs pass19 tests each,with production/test/generator hashes frozen. This is a zero-count extension of FVID-0484's native silence anchoring,not a separately counted problem. Counter485,next FVID-0486. No performance or physical audio-device claim is made.

Accepted in batch-484-validation.json with zero new IDs. The owned target and temporary restoration safety copy were removed after terminal verification;committed source,source hashes,fixtures and before/after proof remain.
