# Causal AAC-ELD AU origin and LD-SBR reference comparison

## Cause and correction

The four-block ELD window has an identically zero leading quarter. The previous
renderer emitted accumulated[0..n], retaining that prefix. It now emits
accumulated[n/4..n+n/4], while advancing history by exactly n as before. This
interval is already known: the next AU's leading quarter contributes zero.
It emits a complete frame immediately and includes the known final quarter at
EOF. No whole-file trimming, zero padding, resampling or fitted delay is used.

The direct transform/window/overlap equations are ISO/IEC 14496-3:2009 section
4.6.20.2 (printed page277, physical PDF page753). The complete source PDF SHA256
is 913b72c340ba90bc9f4fc3bbc2edad8005369076c78a51f147bbbbbe6b5cf20d,
from https://ossrs.io/lts/zh-cn/assets/files/ISO_IEC_14496-3-AAC-2009-90cda427e845cc176293bffee3f92f99.pdf.
The page was rendered and visually inspected. Nominal and adapted windows retain
their existing coefficients; all admitted windows have the zero leading quarter.

## Independent original reference

Two original ISO core MP4/WAV pairs, er_eld1000np_24 and er_eld1000np_48, are
retained outside the repository. Every compressed/reference file is verified
against the pinned official conformance inventory. References are original
24-bit extensible integer WAVE, not converted PCM16. Only the enclosing audio
edit box tag is hidden in a research copy; exported ASC/AUs must remain identical.

The optional `scripts/validate_aac_eld_core_origin.py` compares complete raw
streams at zero lag, including startup and EOF. Before correction, maximum
errors are 0.6362793743610382 and 0.6416769027709961. Afterwards, both maxima
are 8.940696716308594e-8, below one PCM24 LSB. RMS errors are respectively
3.635729751021325e-8 and 3.648397604757005e-8 over 1573376 and 3146752 samples.
This qualifies these two complete streams; it is not formal whole-codec
conformance. The old executable is frozen by SHA256 in the before report.

The reference WAVE utility now strictly reads signed16/24/32-bit PCM and retains
the old PCM16-only wrapper contract. Eight stdlib tests check signed extrema,
classic/extensible headers and malformed geometry/duplicate/truncated chunks.

## LD-SBR consequence and remaining work

All26 original LD-SBR streams were decoded again with the same original FDK
complex-QMF renderer. Every original input, ASC/AU and FDK PCM hash is unchanged.
The24 nonzero-core native outputs change; full-stream zero-lag RMS errors improve
by137 to16259 times. The two zero-core controls remain byte-identical. Largest
remaining full-stream channel RMS versus FDK is0.0005117433;no tolerance was
introduced to call those differences conformant. Such cross-decoder differences
must be separated from native errors using original PCM references.

Four separately fetched original PCM24 WAVs have pinned ISO inventory checksums
and exactly matching full decoded lengths. At zero lag,e1_32 has native maximum
error0.00000401586 versus the original WAV,whereas FDK complex reaches0.025855422.
The large EOF discrepancy there belongs to the fixed-point comparison. The
zero-core twi1_48 native maximum is0.00000101328 versus FDK0.0000457764.
Conversely,gh1_44 retains native maximum0.00237958 versus FDK0.0000497103,and
stereo s2_48 retains0.000535391 versus FDK0.0000395775. Those two native waveform
differences remain open. Automatic/Low Power FDK is substantially farther from
all four original references,independently corroborating complex renderer
selection. The separate four-case report preserves original WAV and decoded
PCM hashes. No delay/gain fitting,cropping,padding or acceptance relaxation is used.

The subsequent complete26-WAV audit verifies every original reference MD5/SHA256
and both cached native PCM versions against their earlier immutable audit hashes.
All26 WAVs are PCM24 and match the entire native channel/rate/sample geometry.
Seventeen cases have full-stream maximum-channel RMS at most2.016e-7. The nine
larger residuals are exactly the six GH cases and three stereo S2 cases,with RMS
from3.146e-5 to8.264e-5 and maximum absolute error up to0.002742425. For the24
nonzero-core streams,the correction improves RMS against original WAVs by819
to1.4million times. Both zero-core outputs remain unchanged. These are measured
differences,not a newly invented conformance threshold or proof that every
remaining difference is a codec defect. The complete per-channel report is
`aac-eld-au-origin-original-sbr-all-2026-11-01.json`;the committed
`compare_aac_reference_pcm.py` computes each comparison without a delay search.

## Authored regression

Twelve own eight-AU programmes (12 MP4 videos and12 audio-only Matroska controls)
isolate first/last/both activation,
480/512 and mono/stereo. A separate full impulse-superposition cosine/window
oracle supplies PCM. Both new PCM tests fail on the previous origin and pass
with the correction. Immediate frame size, reset, checkpoint replay and nonzero
final-quarter output are checked. Two explicit generations with processes and
network disabled reproduce all38 generated files byte-for-byte.

Every one of the12 authored MP4s was also decoded by the frozen before/after
executables. All24 decoder invocations succeed with full sample counts. Each
before output fails the specific PCM bound;each after output passes it. This
separately verifies first/last/both markers,without an unrelated parsing refusal.

The real macOS device drains n480-c2-both,n512-c1-last and nonzero stereo
dual-rate LD-SBR n512-c2-x2 through full playback,seek25%,seek60% and rewind. This is a device/transport check, not a listening assessment.
Broader qualification and compressed-input preservation are recorded separately.

## Completed regression qualification

1206 checks pass,0 fail,1 pre-existing ignored:589 media library tests,565
integration tests across105 AAC modules,35 actual-worker tests,and17 Python checks.
The first coordinator hit its1800-second wall-clock limit. Completed individual
harness results were retained;every remaining named test was subsequently run
successfully without that arbitrary deadline. The final coverage is checked
against complete test-name inventories and unchanged executable hashes. The
qualification report distinguishes this orchestration interruption from decoder
or assertion failures. No test is exempted as an expected failure.

FFmpeg/ffprobe invocations are blocked throughout the integration/worker resume;
there are zero attempted invocations. Ordinary tests use frozen own media and
require no original ISO assets or reference decoder. All257093 prior compressed
input files retain their hashes;only independently derived numeric oracles and
one previously stale preparation-manifest provenance checksum are refreshed.

## Reproduction

Ordinary acceptance uses frozen own assets:

```sh
python3 scripts/test_aac_reference_pcm.py
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline --no-default-features --features player --test aac_eld_au_origin
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline --no-default-features --features player --lib eld_au_origin_keeps_worker_pcm_timestamps_and_navigation
```

The optional original reference audit needs the four named original files and
the pinned inventory from https://standards.iso.org/iso-iec/14496/-26/ed-2/en/.
Keep them outside authored fixtures. Build `aac_spatial_reference` with the same
player feature,then run:

```sh
python3 scripts/validate_aac_eld_core_origin.py --inputs /external/iso-core --inventory /external/mpeg4audio-conformance2ndEd_20230125.md5 --native-probe /external/bin/aac_spatial_reference --work /external/core-origin-audit --report /external/core-origin.json
```

The dependent-generation receipt lists the100 standalone generators in execution
order with their source hashes. Each can be run explicitly with Python. The
sixteen optional C-reference generators are excluded:their assembly/filterbank
stimuli do not depend on ELD core PCM and remain unchanged.
