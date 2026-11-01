# Sound-only explicit stop versus A-B rewind

Accepted FVID-0467 after the identical baseline/candidate and all subsystem gates were qualified. One ordering problem. Endpoint variants and source controls add no IDs.

Authored five-frame 16x16 VP9 MP4 (300ms frames), three independent YUV oracles and 200ms mono 8kHz scalar PCM accompany an offline generator. No private media, codec parameters, FFmpeg or network. Ordinary tests read checked-in files, not the generator. The exact source control decodes every video frame and checks every YUV sample and interval through native EOF; actual WAV decoding sends exact reference PCM to a controlled output backend and supports seek to125ms and natural EOF200ms.

Tests use the real AudioPlayback worker and Player::present via headless egui. The backend publishes a controlled 150ms output phase below source EOF, while loop A=25ms/B=100ms and explicit stop=75ms or100ms. The producer must pause without issuing an A seek; terminal phase is exactly the explicit bound. Removing the loop and seeking after the terminal bound must resume to actual EOF. Separate controls ensure an unreached175ms stop still permits looping and paused transport spends neither bound. No hardware output or new native-window duration claim; the prior viewport-close limitation remains at ../player-ab-clock/native-smoke/finding.json.

Baseline source remains unchanged apart from test-module registration. Do not count fixture admission or stand defects as media fixes. No existing assertions or tests are relaxed.

Baseline:3pass2fail,both failures prove an actual unwanted audio seek(generation1 instead of0),not a source-admission error. Candidate:5pass0fail on the unchanged test hash. Twenty repeats pass100case executions. Nine gates and both builds pass1581distinct tests;the same five old ignores remain. The old pre0463 tick_sound body is byte-identical to the baseline(classification.json). No other improvement IDs counted.
