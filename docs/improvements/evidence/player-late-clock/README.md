# Continuous silent clock across missed refreshes

Accepted FVID-0463;registry463,next0464. Five authored16x16 VP9 pictures span1.5s,each300ms,with independent YUV oracles. Native prefetch is confirmed and media phase reaches75ms before an actual800ms gap between presentation polls,excluding startup-delay confounding. All five files regenerate without private media,FFmpeg,encoder or network.

Qualified identical5test baseline passes exact native pixel/PTS/seek/EOF and zero-bound manual-picture control,but fails all3late acceptances. The stale picture ends300ms;actual phase is about880ms. Clock clips300ms and a450ms stop bound stalls behind a queued600..900ms picture. Pause also loses overdue phase.

Candidate extrapolates both sides of the deadline with rate-scaled integer media time,restores overdue held phase with a signed deadline offset,and parks explicit terminal position. Natural silent EOF remains clamped;healthy audio-tail ownership is retained. Navigation/source replacement releases parked state. Five focused tests pass,including exact catch-up RGB,stop-before-future-picture,paused rate change,seek recovery,stable EOF clock,and manual back/forward after zero stop before the first picture.

Initial four-case clock-only stage2pass2fail exposed terminal/resume boundary regressions;combined4cases passed. Adding zero-bound navigation caught a candidate parked-clock leak(4pass1fail),fixed without counting it. Historical test copies,source hashes,byte-verified reconstruction,and candidate-boundaries.json distinguish these stages. Qualified baseline/final five-test hashes are identical. All are variants or candidate regressions of one cause;controls/repeats add0IDs. All nine qualification gates subsequently passed. No performance,physical audio,GPU or visible-window claim. Next unconfirmed audit:A-B loop endpoint timing.

Final qualification:127 integration suites/591 passing checks,1552 physical-source-distinct checks,and core/player builds. Five old ignores unchanged. Five tests pass20repetitions/100 executions. All candidate boundary repairs remain within this one cause;count delta1.

Count0 native-window follow-up:../native-player-clock-smoke/report.json records two complete10s silent native FVid runs(exit0) and three visually inspected screenshots with advancing authored video. Temporary executable/app packaging was byte-identical to the regular build and has been removed;selected-window screenshots and metadata are retained. Detailed controls remain covered by the actual-worker regression cases. No physical audio or hardware-codec qualification.
