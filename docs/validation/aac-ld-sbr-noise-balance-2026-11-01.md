# Coupled SBR noise orientation, 2026-11-01

## Finding and correction

The three remaining ordinary LD-SBR stereo S2 waveform differences came from
reversed noise balance. `coupled_noise_with_q31` used the opposite orientation
to envelope energy. ISO/IEC 14496-3:2009,4.6.18.3.5,printed236 (PDF712),defines
left Q=2^(7-Q0)/(1+2^(12-Q1)) and right Q=2^(7-Q0)/(1+2^(Q1-12)). Both have
the same orientation. The correction changes the sign supplied to the stable
fraction evaluator; it preserves strict normative bounds and explicit Q31 policy.

The full source PDF was visually checked at the formula page. Source:
https://ossrs.io/lts/zh-cn/assets/files/ISO_IEC_14496-3-AAC-2009-90cda427e845cc176293bffee3f92f99.pdf
SHA256 `913b72c340ba90bc9f4fc3bbc2edad8005369076c78a51f147bbbbbe6b5cf20d`.
No ISO media or codec parameter sets are added to the repository.

## Specific reproduction and acceptance

`aac-ld-sbr-noise-balance` contains four authored eight-AU stereo programmes:
480/512,centered controls and asymmetric coupling. Each has MP4 synthetic video
and a Matroska audio control. At own Q0=7,Q1=8,left noise ratio is1/17 and right
is16/17. The previous implementation reverses these.

Before correction,the centered PCM test passes; asymmetric decoding succeeds
but the acceptance test fails with max PCM error0.000047067264. After correction,
complete independent PCM,both transport paths,player rewind and repeated sample
seek pass. Golden output is generated with independent cosine/window and CLDFB
convolution plus directly evaluated gain/noise fractions. Generators run only
explicitly; ordinary tests require neither FFmpeg nor network. Two regenerated
copies are identical with process/network creation disabled.

The optional syntax inspector now handles stereo CPE and reports core codebooks,
MS masks,TNS and SBR coupling. All three original S2 streams have no PNS bands;
SBR coupling is present in1402/1403 AUs. Diagnostic strict Q0..30 policy remains.

## Original references

Complete signed PCM24 original WAV comparisons use zero lag,no gain fit,no
search or truncation. All26 original streams now have per-channel RMS at most
2.0159868404994132e-7 and max absolute error at most4.015862941741943e-6.
Only the three S2 outputs change; the other23 are byte-identical to the previous
limiter-fixed decoder. S2 RMS is now around9e-8,from around8e-5.

These are measured waveform comparisons,not a claim of all AAC/LD-MPS profiles
or normative conformance certification. Original WAVs/compressed inputs remain
in the external reference cache. See the paired ISO report for full lengths,
reference/native hashes and per-channel metrics.

## Validation

- [Original26 comparisons](aac-ld-sbr-noise-balance-iso-2026-11-01.json).
- [Specific pre-fix reproduction](aac-ld-sbr-noise-balance-reproducers-2026-11-01.json).
- [Offline deterministic generation](aac-ld-sbr-noise-balance-generation-2026-11-01.json).
- [Fresh tests and device results](aac-ld-sbr-noise-balance-tests-2026-11-01.json):
  661 pass,0 fail,1 pre-existing ignored RGB acceptance. Media590,integration70
  across13 targets,and new audio worker1. FFmpeg/ffprobe are blocked,with zero
  invocations. Integration includes ordinary/EP3/downscaled/eighth/spatial-bank
  LD-SBR,Q31,LD-MPS PCM/rates and legacy HE-AAC/PS. Both asymmetric programmes
  drain the real macOS audio device through full/seek25%/seek60%/rewind. This is
  device/navigation evidence,not a subjective listening-quality verdict.
- [Regenerated dependency preservation](aac-ld-sbr-noise-balance-preservation-2026-11-01.json):
  all100 generators run with process/network disabled;257093 previous compressed
  inputs stay identical. Only20 PCM golds,one QMF gold and two scalar oracle
  JSON files change. All530 dequant cases remain,with372 asymmetric noise
  fractions corrected;syntax and geometry are unchanged.

The full ISO/IEC23003-2 AnnexB/TableA.21 is still unavailable locally. Independent
search checked official ISO/MPEG listings,previews and indexed full-PDF links.
The indexed links returned404,403 or DNS failure; preview text does not contain
the required annex. LD-MPS qualification remains open. This ordinary LD-SBR fix
uses the separately available full AAC2009 specification.
