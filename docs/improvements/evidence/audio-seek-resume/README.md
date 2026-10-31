# FVID-0452: verified audio seek/resume device-clock ordering

Registry remains451;candidate ID0452 is not accepted. Base commit is recorded in base.txt. The production command handler is unchanged except for inclusion of the new regression module.

Hypothesis:drain_commands defers navigation but calls backend.resume immediately. A paused device receiving SeekTime then Play in one drain can resume at its stale clock before flush installs the selected source position. Rewind may have the same ordering. This is not a confirmed failure until the baseline executable completes and the controls prove the synthetic source is otherwise valid.

The generator uses only committed authored neutral VP9 with analytic IMA4/authored ALAC PCM. Each short MP4 has3video pictures and an audio track;scalar PCM companions are retained. Ordinary tests do not generate media or require FFmpeg/network. Controls decode all video pictures and exact audio,then check independent sequential seek/play. Acceptance tests exercise the actual private Worker drain with both commands queued deterministically,observe backend flush/resume clocks and compare the exact selected PCM suffix. No physical audio-device or GPU claim.

Before-source hashes and the exact test body are retained. Initial baseline process97694 uses isolated /private/tmp/fvid-audio-seek-resume-target-20261031 and /private/tmp/fvid-audio-seek-resume-before.log. Do not restart a live process on observation timeout;do not count a compile error as a media defect. Preserve this corpus,tests and proof;delete own disposable artifacts after qualification.

## Confirmed baseline and candidate

The initial executable was diagnostic:one video/PCM control passed,but the sequential ALAC control incorrectly expected a target inside its first block to be the returned source landing. The exact initial source/test/log are retained;this control error is excluded from confirmation. ALAC now targets its second block at20ms,IMA4 its second block at8ms;both retain exact scalar suffix checks.

Expanded qualified baseline completed:2controls pass and5device-clock ordering acceptances fail. Both source families retain exact video/PCM;sequential navigation is correct. Queued Seek+Play resumes at0 before flush8ms;Rewind+Play resumes8ms before flush0. A finalPause still starts then pauses a previously paused device. A deterministic real AudioPlayback worker factory holds startup until both commands are queued,and reproduces the same wrong resume order. No production behavior changed before this expanded baseline.

Candidate defers the batch's requested resume until navigation and device controls are installed;a laterPause cancels that pending start. Existing immediate pause of a running device remains. No public API or performance claim. Await focused and complete subsystem qualification;registry stays451.

Focused after-fix run completed7pass/0fail;the unchanged tests then passed20complete repetitions(140case executions),including the deterministic real-thread witness for both codecs. repeat.json records source/binary/log hashes;these repetitions never count as new improvements. Full frozen eight-gate runner56548 is live:127integration targets,root MP4/WebM,player,media,core/player builds,and all audio_thread units. The collector includes audio-thread root-filter overlap. Registry remains451 until all gates pass.

## Expanded subsystem review:pre-existing observer failures(count0)

Initial full qualification passed integration591,root MP469,WebM67,player216,media547,and both builds,but all-audio-thread found7old AAC/PS failures(32pass/7fail,including the7new acceptances). The complete initial run is preserved under candidate-full. Running the identical published HEAD audio_thread source independently reproduces all7failures(25pass/7fail);published-baseline.json records exact source/commit. Candidate was restored in finally.

Six tests assumed a packet numerator equals native sample count or required an unreduced denominator. Their observers now convert the entire rational stamp to native samples with positive num/den and exact divisibility assertions. Exact PCM,size,seek,and checkpoint speed assertions remain. The source-gap observer had dropped its public event receiver;reconfiguration sends the expected terminal error through that channel. It now inspects both returned and published events,keeps the same16step limit and3PCM-packet bound,and asserts the exact canonical error instead of the old AAC-only substring. The temporary diagnostic probe confirms3PCM packets before termination;its source/log are retained and the probe removed.

A second published-runtime probe applies only these test repairs(test-only-baseline.patch):all32original tests pass,without the candidate command-order fix. An initial repaired-observer probe still exposed the stale error spelling(31pass/1fail) and is retained. No production media handling was changed for these observer repairs;they add zero IDs. Final frozen-source eight-gate qualification must pass before452can be accepted.

## Final acceptance

All8frozen-source gates completed exit0:127integration suites591passes,root MP469,WebM67,player216,media547,core/player checks,and all39audio-thread tests. Exact root-filter union and shared-source exclusions yield1511distinct checks;five pre-existing ignores are explicit in deduplication.json. Seven new cases passed20repetitions on the final source,excluded from distinct counts. All5fixture files regenerate byte-identically. The registry counts only FVID-0452;all observer repairs,diagnostic scaffolding,fixture variants and repeated runs add zero IDs. Historical unaccepted sections above preserve the actual review sequence.
