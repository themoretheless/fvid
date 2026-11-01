# Original LD-MPS waveform re-audit, 2026-11-01

Historical audit before the shared single-rate synthesis correction. The later
[source-qualified fix](aac-ld-mps-single-rate-2026-11-01.md) resolves the24kHz
LD-QMF2 discrepancy below;the measurements here retain their original values.


The October31 observations predate the causal ELD origin correction. This audit
uses current owned probe SHA256
`f694c2c6658aa8eb4c22edc8f3ee153773d1e58335196975529d31d7e10c5477`
from production commit11cf2ddf6b. Both implemented decorrelators are selected
explicitly; the production default remains Complex. All comparisons retain the
complete stream,with zero lag,no delay search,no gain fit and exact rate/channel/
frame geometry. These are waveform observations,not complete-codec conformance.

## Current evidence

Maximum per-channel RMS and maximum absolute error over the complete stream:

| Original ISO input | Renderer | RMS | Max absolute error |
| --- | --- | ---: | ---: |
| AAC-LD tree5151 | Real | 1.087456455e-5 | 5.794526078e-5 |
| AAC-ELD tree5151 | Real | 8.170271756e-6 | 1.545098803e-5 |
| LD-QMF1,48kHz | Real/Complex | 8.993463440e-6 | 1.530721784e-5 |
| LD-QMF2,24kHz | Real/Complex | 0.09650983604 | 0.2980642626 |

The ELD5151 maximum is0.5063 PCM16 quantization steps,approximately half a step;
it is not literally bounded by exactly half a step. The previous frozen owned
probe produces maximum-channel RMS0.05361778437 and max error0.431043461 on
the identical original input. The current ELD timing discrepancy is therefore
resolved by the causal AU-origin fix,not by fitting a delay or changing the
reference. Its authored synthetic causality regressions are documented in
[aac-eld-au-origin-2026-11-01.md](aac-eld-au-origin-2026-11-01.md).

The AAC-LD tree5151 numerical residual and24kHz LD-QMF2 discrepancy remain open.
Complex and Real decorrelators are different supported renderers; matching the
original Real-renderer waveforms does not justify changing the playback default.
The complete eight comparisons are retained,including the larger Complex
renderer differences for the two5151 streams.

## Independent control and source investigation

The pinned external FDK reference,explicitly requesting complex QMF,also differs
from the24kHz ISO WAV: full RMS0.09657538915,max error0.29800415039. Current owned
float vs FDK PCM16 RMS is0.001770324761 over the whole stream. Startup differences
are retained. At48kHz,the owned decoder matches the ISO WAV much more closely
than FDK does. Neither agreement with FDK nor its outputDelay metadata is a
normative permission to shift,trim or replace FVid output.

The official SAOC reference archive has been re-fetched and SHA256 checked:
`b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83`.
Its unmodified `sac_polyphase.c` default LD analysis/synthesis phases match the
current owned formulas. `sac_dec.c` can instead select externally supplied
fi/beta synthesis twiddles,with a separate pair for output below27713Hz. This
is a source-level lead,not proof that those twiddles explain LD-QMF2 or that
production math should change. The full23003-2 AnnexB/TableA.21 procedure is
still needed to qualify that convention independently.

## Reproduce separately from ordinary tests

Fetch original vectors with `scripts/fetch_iso_ld_mps_conformance.py` into an
external reference directory. Keep the pinned original ISO checksum inventory
separately. Then run:

```sh
python3 scripts/validate_aac_ld_mps_original_pcm.py \
  --vectors-dir /path/to/originals \
  --inventory /path/to/mpeg4audio-conformance2ndEd_20230125.md5 \
  --native-probe /path/to/target/debug/examples/aac_spatial_reference \
  --output-dir /path/to/research-output \
  --report /path/to/report.json
```

The validator verifies all8 MP4/WAV files against the pinned inventory before
decoding,checks full output geometry,and verifies originals stay unchanged.
Research output cannot overwrite the originals or enter synthetic fixtures.
It uses neither FFmpeg nor network. It is an explicit reference workflow;normal
unit/integration tests do not execute it or download original media.

- [Eight full comparisons and input pins](aac-ld-mps-original-pcm-2026-11-01.json).
- [Frozen before/current ELD timing evidence](aac-ld-mps-original-timing-2026-11-01.json).
- [Current external-reference controls](aac-ld-mps-reference-control-2026-11-01.json).

No new decoder refusal or media failure was introduced or newly discovered here;
this remeasures previously documented discrepancies after the implemented fix.
The previously passing661 scoped checks belong to the unchanged production
commit11cf2ddf6b;this audit does not present them as a new broad codec test run.
