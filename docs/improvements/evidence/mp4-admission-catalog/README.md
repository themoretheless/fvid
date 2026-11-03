# Compact PCM catalog planning audit

Accepted FVID-0541 / batch552:191 passing Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats. All five synthetic files are byte-identical on regeneration;5312 final source/input hashes remain stable. Base:e5acc7552f42fa62f1c66ef322955e00fe6ca5e8.

The original admitted-decoder, PS and routing loops test each description ID with a per-virtual-sample scan, despite an existing one-record compact index. Three authored341.334ms mono16-bit PCM movies have16384 frames, one audio chunk and1/32 identical descriptions selected first/last. Baseline release correctness passes exact PCM and sample budget4. Initial planning pilots show roughly0.73ms versus0.094ms for32 versus1 descriptions; those pilots are diagnostic, not comparative qualification.

The private description_used_before query uses compact chunk records, preserves description-number identity, and stops at a readable positive hit. Dense or expanded indexes keep the original per-sample traversal; gaps, overlaps or unreadable affine boundaries fall back for a negative query. This changes six membership checks across decoder admission, PS setup and normalization routing. It does not allocate a new index, change packet bytes/configuration/timestamps, drop catalog metadata, or alter a public signature. Memory budget factors and initial decoder admission remain unchanged.

The shared unit compares the optimized query with the independent original per-sample predicate at1008 combinations of prefixes/IDs across compact, zero-run, gapped, overlapping, offset-overflow and expanded index shapes. This is one equivalence test per compilation, not1008 media improvements. The committed short synthetic movies and separate generator verify exact PCM/compact record cost. Additional prefix gates exercise f32/f64 output, tight packet/index/memory limits and cancellation; actual native playback checks seek/rewind/EOF, three worker sessions/anchors and software VP9 pictures.

First-stage runtime/measurement hashes and all validation logs are archived separately. That stage passed its scoped timing gates, but review identified unnecessary traversal after a positive hit. The final candidate adds an early positive return, so its release binaries and the same seven-pair protocol are requalified. No first-stage timing is substituted for the final runtime, and the source-level optimization counts at most once.

Predeclared acceptance: seven balanced before/after process pairs, each with two warmup rounds and nine measured rounds of16 owned plan_decode_audio calls, fixed64MiB controlled-memory cap. Both32-description targets must win every paired median and improve aggregate median at least2x. Single-description control aggregate median must not regress over10%. Exact per-input plan fingerprints and source/input hashes must match; related correctness and core/player gates are required. These are warm-cache local-file planning measurements, not playback throughput, decoder speed or RSS claims. Reject/revert an inconclusive candidate. No timing assertion belongs in ordinary tests.

No old test or refusal expectation is edited. Ordinary tests contain no timing assertions and use committed synthetic files without FFmpeg, an encoder, generation subprocesses or the network.

## Final paired release measurements

Microseconds per owned plan call; each value is the median of seven process medians. All captured plan fingerprints agree.

| Catalog/selected ID | Original | Candidate | Ratio | Target wins |
| --- | ---: | ---: | ---: | ---: |
| catalog1-selected1.mp4 | 93.867 | 93.461 | 1.004x | 4/7 |
| catalog32-selected1.mp4 | 675.570 | 101.479 | 6.657x | 7/7 |
| catalog32-selected32.mp4 | 679.234 | 100.042 | 6.790x | 7/7 |

The single-description control is effectively unchanged (about0.4% lower median), within the predeclared10% bound. The32-description workloads win all seven pairs and exceed the2x target. These are planning measurements on this local341ms one-chunk workload; they do not establish whole-player or codec throughput. comparison.json, paired-measurements.json, timed-source.json and build-environment.json retain full observations, hashes and build details.
