# FVID-0454 and FVID-0455: verified terminal scheduling

Registry remains453;candidate0454 is unaccepted. Production behavior is unchanged except for inclusion of the new private regression module. Hypothesis:check_stop uses the end of the displayed picture,so a future end can spend the cutoff before an active media clock reaches it. This was explicitly outside0453terminal-state handling. No failure is confirmed before the baseline executable completes and the independent source controls pass.

Three authored CFR/VFR/fractional VP9 movies plus independent YUV and a200ms WAV/scalar PCM oracle are generated from committed synthetic donors only. All9files regenerate byte-identically. Ordinary tests need no generator,FFmpeg or network. Tests use actual Player/native decoder/egui presentation,an actual audio worker with explicitly controlled clock,and silent frame deadlines. The timed VFR path also checks actual picture holding through a remaining period. No physical audio/GPU/visible-window claim.

Before-source hashes and the exact test body are retained. Baseline session6167 uses isolated /private/tmp/fvid-player-stop-clock-target-20261031 and before.log. Observe it without restarting on timeout;compile/scaffold errors do not confirm the hypothesized media failure. Preserve valid source controls and strict before/at-cutoff assertions.

## Independent confirmed defects

Published baseline4case run has1exact-source control pass and3bound-clock failures:live20ms spends40ms cutoff,and both silent deadline/actual-holding checks fail. Expanded published5case baseline has1pass/4fail,adding real natural EOF without any stop bound:the actual native worker EOF ends the player at80ms while the final picture lasts until100ms. All baseline production behavior was unchanged apart from the regression module inclusion.

The first production stage fixes only bound scheduling. The unchanged5case suite becomes4pass/1fail,leaving the independent natural EOF failure. A sixth silent/rate0.3x actual-EOF witness also fails under that bound-only stage(4pass/2fail);its exact source/test/log are preserved. This separation supports two independent candidates,0454bound scheduling and0455natural EOF completion;fixture variants remain within those two causes.

Final candidate uses an active audio clock or exact interval/deadline clock for bounds,checks before publishing the next picture,and fences at/after-bound pictures even when A/V slack would make them ready. Explicit seek and paused/manual-step policies remain. The independent EOF change retains the actual native EOF and generation until the same clock completes the last picture;manual steps still finish without waiting for paused audio. Natural audio-tail handling is retained.

All6focused checks pass. Await final8gate qualification and repetitions. Registry remains453;neither candidate is accepted yet. No public API or performance claim.

Final6cases pass20complete repetitions(120executions),excluded from distinct-check and improvement counts. Source/binary/log digests are in repeat.json. independence.json maps each independently failing cause and its acceptance tests. Full frozen8gate process29814 is live;expected player group228passes. Registry remains453 until every gate completes successfully.

## Completed-audio review(amendment,count0)

The6case candidate passed all8gates;its exact source,results/logs and repetition are preserved under candidate-six-case. A7th witness starts normal unpaused playback,observes both actual worker EOF events,and completes the200ms audio covering the180ms video. The candidate then wrongly consults an unused future startup wall deadline,adding362ms in the captured failure. handoff-before.log records6pass/1fail;this was introduced by EOF deferral and is not a third improvement ID.

The amendment records the displayed picture's audio-clock context. If the completed device clock already covers that audio-paced picture,it remains authoritative instead of rewinding to the unused startup deadline. Silent pictures retain their scheduled-period handling;original6assertions remain. All7focused cases pass. Final7case repetition and complete8gate requalification are required;old6case all-green metadata is provisional.

Final amended7cases pass20complete repetitions(140executions);source/binary/log digests are in repeat.json. independence.json retains the2original causes and marks the completed-audio regression/amendment count0. Final frozen8gate run42944 is live;player229passes and final terminal exit0are required. Prior6case all-green metadata cannot establish final qualification. Registry remains453.

## Final acceptance

Final frozen8gate run completed exit0:127integration suites591passes,root MP469,WebM67,player229,media547,audio-thread39,and core/player checks. After root-filter and shared-physical-source exclusions,1521distinct checks pass. All7final cases pass20repetitions(140executions),excluded from distinct/ID counts. Nine fixture files regenerate byte-identically. Five existing ignores remain explicit;no new skip or weaker assertion.

Only the2independently reproduced causes are accepted as0454and0455(batch454). The completed-audio amendment,shared control,fixture variants,partial probes and repetitions add zero IDs. Historical unaccepted sections retain the actual sequence;candidate-six-case is provisional evidence,not final qualification.
