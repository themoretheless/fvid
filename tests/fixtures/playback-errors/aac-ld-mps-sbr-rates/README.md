# Nominal high-rate LD-SBR into LD-MPS through PCM

Generate explicitly with `python3 scripts/generate_aac_ld_mps_sbr_rates_fixtures.py`.
32 own MP4/MKV pairs cover a48kHz ELD core,480/512 samples,mono/stereo/5.1
inputs,all8 spatial trees and single/dual-rate SBR (48/96kHz PCM).12 bare
controls use independently calculated core PCM, literal CLDFB convolution and
spectral SBR reconstruction. Spatial composition uses qualified MPS stages.
This is not an independent whole-codec/profile conformance oracle.

The SSC explicitly describes the actual48/96kHz PCM clock. MPS uses64/128 bins;
480-sample cores require two source packets per spatial frame. The original
runtime passes all bare controls but refuses three spatial acceptance tests at
its shared-QMF geometry admission. Tests now qualify untimed/timed decode,
MP4/MKV export/playback,seek/rewind/EOF,checkpoint and malformed suffix retry.
The root AudioWorker checks queues and live-history navigation.

The first oracle prototype incorrectly reused the centered downscaled32-band
analysis for nominal CLDFB and failed its bare control. It was corrected to the
separately qualified nominal modulation and32-band output normalization before
the recorded pre-fix reproduction. That initial run is not admission evidence.

Existing matching shared-QMF streams retain their path. Other valid geometries
use owned LD-SBR PCM synthesis and owned MPS PCM analysis/synthesis;there is no
external decoder/resampler. Nominal SSC downscale remapping, direct downscaled
shared QMF, factor-three SBR and full profile conformance remain open.
All media and parameters are authored locally. Ordinary tests use frozen files
without generation,network or FFmpeg. Physical audio output is not qualified.
