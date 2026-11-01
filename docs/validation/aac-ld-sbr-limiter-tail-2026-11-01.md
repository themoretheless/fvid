# Limiter partition after a discarded short HF patch

## Localization and cause

After the causal ELD origin correction,the original GH1_44 stream differs at
only four primary AUs624/789/885/916 and the following synthesis tails. The
corresponding inverse filters are Off and no harmonics are signalled. Header
changes expose a retained final patch end that also occurs in the low-resolution
frequency table while one or two terminal HF bands have no retained patch.

The old limiter treats every retained patch end as protected,including this
coincident end. That creates an extra protected limiter interval at the
unpatched tail and changes noise attenuation/boost. The correction protects
only internal boundaries between retained patches and the complete SBR range
endpoints. No frequency range is omitted and no patch is invented.

The historical ISO/IEC14496-3:2009 Figure4.41 (printed231,physical707) sums patch
widths into patchBorders,which is ambiguous at the outer edge after discarding
a short final patch. The old endpoint-preservation adaptation covered k(m) but
also retained this spurious internal boundary. This correction keeps the full
Cor.1 gain domain and uses the canonical complete-range boundary convention.
The unmodified reference [createLimiterBandTable](https://android.googlesource.com/platform/external/aac/+/ee76460efbdb147e26d804c798949c23f174460b/libSBRdec/src/env_calc.cpp)
sets the final boundary to highSubband and includes only retained patch starts
as internal boundaries. Original ISO PCM independently corroborates this choice.
This is not a claim that the literal old flowchart already contains that fix.

## Independent authored reproduction

Eight own short programmes cover480/512,all four limiter densities and a strong
own spectral bin64. Their headers and coefficients are independently chosen;
no original media/parameter sets are copied. Direct full cosine/window
convolution and CLDFB superposition produce frozen PCM,with Decimal spacing and
explicit per-interval gain/noise/boost. Both mode0 controls pass before/after.
The six mode1–3 videos decode successfully before,but fail PCM acceptance;the
first error is0.014393883. After correction all eight programmes match independent
PCM in MP4 and Matroska. Two offline generations reproduce all26 generated files
exactly with network/processes disabled.

The1680 existing geometry input cases are unchanged.466 expected partitions
change under the corrected boundary convention;the high-precision oracle still
uses Decimal ratio thresholds rather than native floating logarithms.

## Original ISO waveform evidence

All26 original compressed inputs,ASC/AU records and reference FDK PCM hashes are
unchanged. Only the six native GH outputs change;the other20 outputs remain
byte-identical. Against complete original PCM24 WAVs at zero lag,the six GH RMS
errors fall from3.15e-5–4.08e-5 to1.23e-7–1.46e-7. Their largest absolute error
falls from0.002742425 to0.000001439825. Thus23/26 original streams now have RMS
at most2.016e-7. The three stereo S2 differences remain unqualified and open.
No delay/gain fitting,cropping,padding or relaxed acceptance threshold is used.

## Qualification

606 scoped checks pass,0 fail,1 pre-existing ignored:590 media library tests,
15 integration tests across four related LD/legacy SBR targets,and one actual
worker regression. The worker covers both transports through full playback,
seeks,EOF and rewind. Real macOS device drains for n480-lim1 and n512-lim3
complete full playback,seek25%,seek60% and rewind. Device drain is not a listening
assessment or a whole-codec conformance verdict. All previous compressed fixture
files remain untouched;only numeric geometry expectations change.
