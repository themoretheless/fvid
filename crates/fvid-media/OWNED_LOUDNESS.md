# Owned PCM loudness analysis

`fvid-media` exposes `owned_loudness::LoudnessMeter` and
`owned_k_weight::KWeighting` without the `legacy-ffmpeg` feature. They operate
on interleaved f64 PCM with explicit channel weights. Container reading,
codec decoding, cancellation and file export remain caller responsibilities.
The FVid file adapters use this same implementation for WAV, AAC and other
supported owned audio sources.

```rust
use fvid_media::owned_loudness::LoudnessMeter;

let mut meter = LoudnessMeter::new(48_000, &[1.0, 1.0])?;
meter.push(&interleaved_stereo_pcm)?;
let report = meter.report();
```

The meter reports integrated LUFS with absolute and relative gating, loudness
range and unweighted sample peak. It does not calculate true peak. Stream
chunk boundaries do not reset filter history. LFE energy is excluded by an
explicit zero weight; surround channels use their explicit energy weights.

Build and test independently:

```
cargo test --manifest-path crates/fvid-media/Cargo.toml --no-default-features
```

Normal tests require neither FFmpeg nor network access. The two ignored
reference tests may be run explicitly with `FVID_REFERENCE_FFMPEG` pointing
to an external executable. Legacy libav loudness APIs still exist behind the
optional legacy feature; this migration does not claim their replacement.

## Constant gain normalization

`owned_normalize::GainPlan::new(&report, target)` selects the lesser of the
loudness correction and sample-peak headroom. Its `linear_gain` can be passed
to streaming export, or `apply(&mut pcm)` can process interleaved f64 chunks
in place. The gain is constant across chunks, preserves channel ratios and
dynamics, and never guesses a layout. Silence, nonfinite measurements and gain
outside `(0, 64]` are rejected. Invalid PCM is rejected before modifying samples.
This is sample-peak constrained gain, not dynamic loudnorm or true-peak limiting.

`NormalizationProgressHook::for_phase` adapts a normal copy progress hook for
measurement or export. `phase_complete` marks each phase; `done` only marks a
completed export. The caller must emit export completion after publishing output.
