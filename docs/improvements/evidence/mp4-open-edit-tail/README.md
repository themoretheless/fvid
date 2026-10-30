# MP4 declared packet-free audio tails

Accepted FVID-0419: 281 distinct checks and core/player builds pass. Publication is recorded separately after Git delivery.

## Reproducer

The standalone generator writes 12 open/finite pairs with existing authored VP9 and nonzero AAC, integer PCM or ALAC. Each contains three encoded audio packets followed by an explicit tfhd duration-is-empty interval with no trun or payload. V0/V1 elst and tfdt plus zero/one-packet media starts are covered. The audio oracle appends one packet duration of literal zero PCM to the authored source oracle; video packet counts are recorded from the authored original controls. Ordinary tests use only checked-in files.

The original implementation indexes the declared endpoint correctly, then resolves an open edit only against packet composition endpoints. For AAC, a valid 85 1/3 ms edit becomes 64 ms; PCM/ALAC similarly lose 20 ms. A finite equivalent is refused by owned decoding as extending outside available samples. Native scheduling also needs an explicit silence segment rather than attempting to fetch another packet.

Two one-byte invalid encoded final packets have valid MP4 framing and exact AAC/ALAC refusal assertions. Three finite edits extend past the declared tail and must still fail in direct decoding, file export and actual audio workers. These are refusal checks, separate from successful valid-tail playback.

## Baseline

The final nine tests were run against original commit 02ce5fd4703cdea5c0fc29e68bfe8c63d1e4ec3b by restoring only the five owned production files in the same workspace and recompiling them. The wrapper restored the exact fixed bytes in finally. This uses no source-snapshot/target-path switching. See full-baseline-source.json and full-baseline.log: 2 controls pass, 7 acceptance/refusal-contract tests fail.

An initial oracle incorrectly assumed that every imported video has three pictures; AAC has sixteen. The generator now records the exact authored source picture count. That rejected run is retained. Another expanded test assumed f32 WAVE for precise PCM export; it now checks the actual required f64 format and compares exact payload bytes between normalized finite/open controls. No production behavior or established assertion was relaxed.

## Repair

Track endpoint calculation distinguishes indexed DTS coverage from composition endpoints, so a genuine stated tail can extend time while an ordinary signed CTTS shift remains unchanged. Open edit resolution preserves the raw zero marker. Native edit scheduling splits only the declared packet-free source range into exact silence segments; finite AAC with such a tail uses that scheduler too. Owned f32/f64 integer and rational normalized paths append bounded zero chunks only after all packets drain and only inside the declared range. Missing data beyond that range and malformed codec packets remain errors.

## Uncounted original normalization follow-up

The expanded geometry suite already fails on the original commit for ssr-changes at 22.001..75.007 ms with a non-sample-aligned full-track duration. The selected prefix has one setup, so the old route constructs an integer full-track schedule although the declared duration is fractional at the initial rate. Qualify that unedited extent for rational normalization without reading future payload just to select the route. Existing test assertions remain; only the failure diagnostic gains the case and interval. This follow-up adds no ID.

Final evidence is recorded in status.json. This is a correctness change, with no performance claim or physical speaker/GPU qualification.

Saved logs normalize only trailing empty lines to one final newline. Registry metadata repairs are uncounted; historical failure reports remain unchanged.
