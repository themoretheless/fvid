# Sound-only native EOF transport ownership

Accepted FVID-0468 after the identical final eight-case baseline/candidate and all subsystem gates qualified. One independent healthy EOF ownership problem;endpoint variants and controls add0IDs. The issue under audit is healthy native EOF closing an item before pending A-B or explicit stop can be resolved.

An authored five-frame16x16 VP9 MP4 and200ms mono8kHz PCM source have exact checked-in YUV and PCM oracles. Generation is offline and separate from tests;no private media,FFmpeg,network or encoder. Ordinary tests use only checked-in sources.

The real AudioPlayback worker decodes exact PCM. Its controlled backend records successful drain,then an acknowledged Pause/Play pair fences event delivery:Worker::run sends its drain event before processing a further command. Tests leave the actual Ended event in the player's receiver;they neither inject an event nor depend on an arbitrary wall-clock sleep. An A25ms seek must replay the exact PCM suffix from byte800 and later reach natural EOF200ms. Controls distinguish healthy source EOF,unreachable B beyond source duration,and a named backend drain error that must be refused without replay.

Headless egui drives the player and real native workers. No physical sound device,new native-window duration,hardware-codec or performance claim. The separate pre-existing viewport-close limitation remains at ../player-ab-clock/native-smoke/finding.json.

Final baseline:3pass5fail;unchanged candidate:8pass0fail. Twenty repetitions pass160case executions. Nine gates pass1589physical-source-distinct checks and both builds;the same five old ignores remain. Initial seven-case3pass4fail and final expansion history are preserved separately(test-expansion.json);no existing assertion was removed or weakened. The old completion branch is byte-identical before0463(classification.json);no historical full-suite success is claimed.
