# FVID-0518: ordered raw Dolby decoded-window lookup

The raw AC3/EAC3 presentation adapter used a linear search through the complete
frame index for every decoded packet. Sequential presentation therefore incurred
quadratic index walking. This is a measured component performance opportunity,
not a claim that the original decoder produced incorrect audio.

`before.rs` preserves the original lookup expression from the baseline named in
`baseline-source.json`. `bench.rs` includes the actual replacement production
helper and an identical production Frame declaration (40 bytes on this host).
The replacement uses lower-bound lookup over the ordered unsigned native clock,
retains the first duplicate match, preserves signed-coordinate bit patterns and
the default window for missing keys. Checked cumulative addition maintains the
non-wrapping sorted-index invariant; it is supporting code, not a separately
counted overflow fix. No additional heap index, cache or public API is introduced.

## Reproduce the comparison

From the repository root:

```sh
rustc --edition=2024 -O docs/improvements/evidence/ac3-window-lookup/bench.rs -o /tmp/fvid-window-bench
/tmp/fvid-window-bench
```

Each size uses 8192 queries and five paired measurements with alternating order,
black-boxed inputs/results and equal-checksum assertions. `measurement.json`
records compiler, exact source/executable hashes and the first final measurement;
`recheck-benchmark.jsonl` records an independent second execution of that same
verified executable. Preliminary/extraction controls are retained separately.
Only helper lookup cost is measured, not whole-player throughput. Tiny indexes
of 1..16 frames can incur subnanosecond lookup overhead. The change targets long
indexes and does not claim a universal short-stream improvement.

## Correctness scope

The helper unit compares the old expression with the new lookup for empty,
variable-size, duplicate, missing and signed-edge keys. The new offline integration
suite reuses the existing authored short AC3/EAC3 tones under
`tests/fixtures/playback-errors/coded-audio-index-eof`, checking native stamps,
exact decoded-window acceptance and specific oversize refusal. Existing coded
worker and Dolby suites check actual decoding, selected programmes, stereo,
changing roles and presentation delay. No new parsing/decoding failure was found
or claimed, so no extra media fixture is needed for this performance change.
Fixture generation remains separate and ordinary tests do not invoke FFmpeg.

The exact commands, return codes and unchanged-source checks are recorded in
`qualification-results.json`; 2752 source/fixture hashes are frozen in
`candidate-source.json`. Test logs and benchmark sources/results are retained;
owned build outputs and the benchmark executable are removed after qualification.
Physical audio-device and visible-player qualification are outside this claim.

## Accepted measurements and gates

All ten final pairs at65536 records improve at least267.11x;median ratios are
444.46x and462.59x. At128 records the medians are3.858x and3.932x.
For1..16 records the median extra cost stays below1ns;individual noisy samples
are retained. Correctness gates pass87 executions,zero ignored,plus both
core/player build checks. Three separate repeats pass10 tests each.
