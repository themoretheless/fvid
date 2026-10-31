# Relative jump clock origin

Accepted FVID-0461;registry461. Tiny authored10s MP4 has five16x16 VP9 colored pictures,each2s,with independent YUV planes. Five retained files regenerate without private media,encoder,FFmpeg or network. Long picture periods expose the actual supported3s transport action;shorter movie would hide the wrong target through endpoint clamping.

Qualified identical five-test baseline passes exact pixels/PTS/native seek/EOF control and real EOF/zero clamp/paused-state control. Three clock-origin acceptances fail:at0.30s,forward3s selects5s instead of3.30s;at4.30s,back3s selects3s instead of1.30s;paused forward repeats the error. Candidate changes only relative jump origin to the active presentation clock. All5focused tests then pass,including exact resulting native interval and RGB. Three variants are one cause;controls add no IDs. All nine qualification gates subsequently passed. No performance,physical audio,GPU or visible-window claim.

Final qualification:127 integration suites/591 passing checks,1542 physical-source-distinct checks,and core/player builds. Five old ignores unchanged. Five tests pass20repetitions/100 executions. Three transport variants are one cause. The next audit will verify requested sub-frame position after seek preroll;this item proves jump request origin and exact selected picture.
