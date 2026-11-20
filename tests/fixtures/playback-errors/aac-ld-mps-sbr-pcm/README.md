# Downscaled LD-SBR PCM interface into LD-MPS

Generate explicitly with `python3 scripts/generate_aac_ld_mps_sbr_pcm_fixtures.py`.
Own 32-packet baseline programmes cover480/512 ELD, factors2/4/8, single/dual-rate SBR,
all8 trees, coupled stereo panning and protected/unprotected SBR. Twelve further
12-packet programmes reuse own wide-LF controls with changing coded crossover
and inverse filtering.152 MP4/MKV pairs and55 bare LD-SBR controls use an SSC explicitly matching adapted PCM.
Two additional own MP4 counterexamples flip an SBR CRC bit on source packet3.

The independent PCM control uses direct nominal/short core cosine/windows,
direct CLDFB convolution and independently authored SBR spectral reconstruction.
Spatial acceptance uses that saved PCM plus previously qualified spatial stages.
This is component composition/integration evidence,not an independent normative
whole-decoder or complete Annex B/profile conformance oracle.

Native decoding now uses the LD-SBR CLDFB PCM path followed by ordinary LD-MPS
PCM analysis/synthesis. This is distinct from the shared LD QMF handoff,which
remains selected for previously supported nominal streams. There is no external
codec, FFmpeg or generic PCM resampling fallback. The SSC clock must already
match actual adapted PCM;nominal SSC remapping is not implemented here.

Checks cover exports, untimed/timed native APIs, source-window navigation in
both transports, checkpoints, suffix/CRC rollback and valid retry, and full
AudioWorker queue/live-history seeks. Factor-three SBR remains unsupported.
All media/parameters/PCM are authored locally;no private material. Tests use saved
files without generation/network/FFmpeg/C reference compilation. Generation is
separate,offline Python with no external process.
