# Explicit short-core PCM clock into ordinary LD-MPS

Generate with `python3 scripts/generate_aac_ld_mps_downscaled_fixtures.py`.
112 MP4/MKV pairs cover AOT23/AOT39, 480/512 nominal cores, factors2/3/4/8
where integral, all8 trees, and32 source packets. The SSC explicitly describes
post-downscale PCM at24/16/12/6kHz. A spatial frame covers1/2/4/8 source packets;
512/factor8 uses two packets to retain at least four spatial slots.

42 bare controls retain independently computed short core PCM: nominal AAC-LD
prediction plus direct short cosine/windows; AAC-ELD direct cosine and independent
dense spline windows. Spatial PCM uses these controls plus previously qualified
spatial stages. This is composition/integration evidence,not an independent
whole-codec or complete Annex B conformance oracle.

Before the native fix,all bare controls pass while both spatial acceptance tests
fail specifically at downscaled spatial filter-bank admission. Current tests
cover public/owned exports, MP4/MKV reader playback, frame-boundary seek, EOF,
rewind, full native checkpoints and failed-suffix retry. Root worker coverage
checks queue/source stamps and live-history navigation.

Nominal-rate SSC adaptation remains unsupported. Downscaled LD-SBR is now
covered separately by `aac-ld-mps-sbr-pcm` through the owned PCM interface;
direct downscaled shared-QMF ingress remains unsupported.
All media, parameters and PCM are authored locally. Generation uses no external
process or network;ordinary tests use saved files without FFmpeg or generation.
