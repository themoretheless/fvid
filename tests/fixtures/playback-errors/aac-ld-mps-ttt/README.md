# TTT transform and complex inversion controls

Owned synthetic component inputs only. Saved little-endian f64 results come
from an independent 80-digit Decimal oracle. No private media or codec
parameter sets are copied. These are not spatial movie/playback/seek acceptance.

40937 transforms cover every CPC pair on the 51-level grid with all ICC levels
and residual/nonresidual regions, excluding the singular nonresidual ICC=0
combinations. Energy and center-subtraction each cover every 31x31 CLD pair.
Mode aliases 0/1, 2/3 and 4/5 are checked by ordinary tests. All 2601 singular
CPC pairs are separately checked for a specific refusal, with residual-region
acceptance; that refusal is not intended media playback acceptance.

7688 inversions cover every pair of OTT CLDs, with four deterministic TTT
parameter trials for predictive and energy weights. Decimal uses a complex
adjugate; owned Rust uses pivoted elimination. Both inverse multiplication
orders are checked. An optional separate harness compares unmodified ISO C
functions to this same saved Decimal oracle.

Explicit offline generation:

```sh
python3 scripts/generate_aac_ld_mps_ttt_fixtures.py
```

Ordinary `aac_ld_mps_ttt` tests read saved data only, requiring neither FFmpeg,
network, generation, a C compiler nor reference software. Full TTT tree
composition, optional real-only alias-lock DSP, residual decoding and spatial
PCM/playback/seek integration remain open. Native spatial refusal stays enabled.
