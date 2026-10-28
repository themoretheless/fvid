# Selected video concat clock

A single owned PCM-coded AVC picture lasts 40 ms. An owned mono AAC pulse track lasts 512 packets (10.923 s). The generator preserves the original video payload and offsets and appends a separately indexed AAC mdat. No private media or codec parameters, FFmpeg or network are involved.

Selecting track 0 and concatenating this file twice must produce video frames at 0 and 40 ms, with an 80 ms total presentation. Before the fix the second frame started at 10,922,666,667 ns because duration was derived from all tracks, including discarded audio.

The packet timing regression is separate from player acceptance. Player verification checks exact RGB equality against the original picture, two frames, duration, repeated EOF, rewind and seek at/beyond the final timestamp. Generation is separate from test execution.
