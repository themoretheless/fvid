# Seek refusal and event generation audit

Accepted FVID-0456 and FVID-0457; registry457. The qualified baseline passes two controls and fails three assertions: failed seek retains loading, fatal video refusal leaves actual audio worker running, and an old queued refusal terminates a newer valid seek. UI teardown alone passes four tests, leaving the stale refusal failure. Both fixes pass all five focused tests. All nine qualification gates subsequently passed.

Authored 16x16 VP9 MP4s contain five packets at 50Hz. The valid movie contains five independently authored colored keyframes. The malformed movie contains four valid coded packets and an explicit one-byte invalid VP9 marker; decoder lookahead emits three valid prefix frames. Independent YUV and 200ms mono 8kHz PCM oracles are retained. All eight files regenerate identically without FFmpeg or network. Initial three-packet scaffold results are diagnostic only and are excluded from defect confirmation.

Tests verify exact source pixels and specific refusal, successful ordinary EOF seek, actual audio worker resume and scalar PCM before fatal pause, stale-error recovery, and a subsequent current-generation refusal. The loading test consumes the initial refusal before requesting its own failing seek. No physical audio or visible-window claim. Ordinary tests read retained fixtures only.

Accepted FVID-0456 and FVID-0457 after all nine offline qualification gates passed: 127 integration suites/591 checks,1531 physical-source-distinct checks and core/player builds. Five existing ignores unchanged. Twenty repeated runs pass 100 case executions. Registry457,next0458. Source hashes, unchanged test hash, private-generation compatibility, reconstruction and native exact/error/recovery evidence are retained.
