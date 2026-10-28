# MP4 metadata admission before allocation

`small.mp4` is an owned 40 ms, 32x32 PCM-coded AVC picture. `padded.mp4` adds a legal 1 MiB `free` box inside `moov`, preserving every media byte and media offset. `unsupported-entry.mp4` changes the sample-entry type to the intentionally unknown `fvdx`, to preserve unsupported-format dispatch. No private media, external encoder or network is involved.

Before the fix, the direct parser's 64 KiB metadata limit rejected padding, but the owned remux plan admitted it under a 64 KiB controlled-memory option because it used the default 32 MiB parser limit. Small input at 64 KiB and padded input at 2 MiB are positive controls.

Separate tests cover resource refusal, preservation of an existing destination, absence of temporary outputs, public routing and concat, and ample-budget remux with exact RGB, 40 ms timestamps/duration, repeated EOF, rewind and seek. Unknown sample-entry refusal is a dispatch control, not playback acceptance. Generation remains separate from ordinary offline tests. This bounds parser metadata admission and does not claim a physical RSS bound or complete accounting of every parser allocation.
