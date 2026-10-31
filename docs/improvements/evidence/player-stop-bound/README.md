# FVID-0453: verified terminal stop-bound holding and navigation

Registry remains452,candidate0453 is not accepted. Published3bed435500 sets ended when --stop-time is reached but leaves producers running and present consuming queued pictures. Qualified baseline2controls pass/2acceptances fail:after the terminal flag CFR advances20..40ms to40..60ms,and the actual audio worker receives zero pauses. Queue continuation and exact native CFR/VFR/fractional/YUV/PCM controls pass.

Three authored16x16 five-picture VP9 movies and a200ms mono8kHz WAV with scalar float oracle are generated only from committed synthetic material. No private input,FFmpeg or network is used. Tests run actual Player/native decoder/egui presenter and an actual audio worker with a timed backend. They check picture/RGB hold during400ms of subsequent repaints,real device pause and stable audio clock,and transition to a queued item. No physical device/GPU or visible-window claim.

The initial scaffold mistakenly treated encoded WAV packets as already decoded PCM. Its source/log are retained separately and excluded. The corrected native control decodes packets and validates all scalar PCM;production behavior was unchanged before the qualified baseline.

Candidate parks active video/audio producers when an explicit bound is spent,and prevents repaint from consuming queued pictures after terminal state. Explicit navigation/restart already clears ended. Natural EOF still permits the existing audio-tail behavior;queue continuation is retained. Four focused tests pass. Await final eight-gate qualification;all variants count at most once.

The new collector also excludes all three compiled contexts of the same three shared edit-clock tests. Earlier batches excluded only one duplicate context;this reporting refinement subtracts six shared-source executions rather than three and never changes accepted improvement IDs or weakens tests. Final unique-check totals use the stricter physical-source method.

## Candidate seek review(count0)

The initial4case candidate passed every8gate check;that provisional run and its source/repetition hashes are preserved under candidate-full. Additional actual video/audio seek-after-stop tests reproduce an introduced mismatch:producers were parked while the chosen UI mode stayed unpaused. Seeking cleared ended but did not resume those producers. The new6case run has4passes and2stalls;seek-before.log and exact source/test snapshots preserve the failures.

The amendment restores an unpaused transport when seeking out of terminal state. Video resumes immediately after scheduling the new target;its audio is still held until presentation preroll. Sound-only seek explicitly resumes after its new clock is installed,using the previously verified command-order handling. Paused navigation remains paused. This review adds no ID and does not weaken the original4cases. Final6case and complete8gate qualification are required before acceptance.

Final amendment focused run passes all6cases. These same6cases passed20complete repetitions(120case executions) on frozen final source;repeat.json stores source/binary/log hashes. Final8gate process41445 is still live. Its expected player group is222passes;the older220pass provisional metadata cannot establish final qualification. Registry remains452 until final completion.

## Final acceptance

Final frozen8gate run completed exit0:127integration suites591passes,root MP469,WebM67,player222,media547,audio-thread39,and core/player builds. Root-filter duplicates plus all3compiled shared-clock contexts are excluded:1514physical-source-distinct checks. Five existing ignores are explicit in deduplication.json. All6final cases passed20repetitions(120executions),excluded from distinct counts. Nine fixture files regenerate byte-identically.

OnlyFVID-0453 is accepted. The scaffold correction,candidate seek reentry amendment,fixture variants,repetitions and stricter reporting method add zero IDs. This item retains the existing frame-end threshold trigger;it qualifies terminal hold,actual producer pause,and explicit seek reentry rather than sample-accurate cutoff scheduling. Historical unaccepted sections above retain the actual review sequence.
