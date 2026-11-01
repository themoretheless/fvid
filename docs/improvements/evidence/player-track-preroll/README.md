# Track replacement must inherit the video hold

Accepted FVID-0471 after all nine gates and twenty repetitions passed. FVID-0471 is one new-handle readiness problem;direction,seek/buffer/pause variants and controls add0IDs. The active old handle already honors the pause while its replacement starts independently.

An authored valid1.5s MP4 has five16x16 VP9 keyframes(300ms each)and two distinct600ms mono8kHz scalar PCM tracks. Exact independent YUV/F32 oracles are included. Generation is offline,separate from normal tests;no private media,codec parameter copying,encoder,FFmpeg or network.

The actual NativeReader is opened on a one-byte Read/BufRead/Seek input. After decoding the first picture,real reads of video sample2 block under a bounded Condvar gate. The player shows0..300ms and its lookahead queue is empty. A450ms seek has a new generation but cannot finish preroll until that real input is released. A separate case waits for the genuine500ms-grace buffering pause. Both forward/backward replacements must retain that hold. Existing user-pause,ready-video and sound-only controls distinguish the intended permission boundary.

Backend observations first await actual seek position,then use an acknowledged unique volume1001 marker followed by a separate restoring1000 marker. A marker handler can run before deferred Play in its command batch. The second marker is sent only after the first acknowledgment,so Worker::run must complete prior Play before processing it. This observes the actual backend running state without arbitrary sleep or synthetic event injection,and restores the original gain. Real resumes and native PCM bytes independently witness early output. After release,tests await the actual resume acknowledgment,compare display_rgb of the presented picture(including planar layouts)against native RGB,and require selected PCM prefixes,final-picture seek and EOF.

Preparation and caller release guards unblock the reader before Player joins its worker,including assertion failures. The final stand transfers guard ownership without a forgotten Arc;the initial baseline/stand expansion is retained separately and adds0IDs. No old assertion is removed or relaxed.

Checks use real native workers and headless egui with controlled output. No physical audio,new native-window duration,hardware-codec or performance claim. The existing separate viewport-close limitation remains in ../player-ab-clock/native-smoke/finding.json.

Final unchanged seven-case baseline:4pass3fail,candidate:7pass0fail. Twenty repetitions pass140case executions. All9gates pass1615physical-source-distinct checks and both builds;five old ignores unchanged. Initial guard ownership and asynchronous/planar stand observations are preserved in stand-audit.json and diagnostic snapshots,not counted as separate media defects. The existing-handle hold and470clock-origin behavior remain intact.
