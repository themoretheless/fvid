# Requested sub-frame position after seek preroll

Accepted FVID-0462;registry462. Five authored16x16 VP9 pictures span10s,each2s,with independent YUV oracles. Target3300ms lies inside2000..4000ms;remaining media duration is700ms. Five retained files regenerate without private media,FFmpeg,encoder or network.

Qualified identical5test baseline passes exact native pixels/PTS/seek/EOF control but fails all four phase acceptances:1x,2x,paused/resume,and a stop-time3500ms after seek3300ms. The selected picture is correct while player clock rewinds to about2000ms. Candidate retains the covered requested phase when preroll finishes,anchors only remaining silent picture duration at selected rate,and preserves the held paused phase. Manual steps still use picture starts;active audio keeps its own clock. Targets outside the returned interval retain existing scheduling.

Five focused tests pass. They verify post-preroll clock,exact native RGB,actual remaining wall hold,next interval,natural EOF,and correctly timed stop-bound teardown. All nine qualification gates subsequently passed;all four phase variants are one cause and source/control/repeats add no IDs. No performance,physical audio,GPU or visible-window claim.

Final qualification:127 integration suites/591 passing checks,1547 physical-source-distinct checks,and core/player builds. Five old ignores unchanged. Five tests pass20repetitions/100 executions. Baseline and candidate have identical test SHA;baseline differs from the published base only by its test hook,with byte-verified reconstruction retained.
