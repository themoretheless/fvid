# Non-SBR AAC-ELD +64-band residual MPS, two-core spatial windows

Generate explicitly offline:
`python3 scripts/generate_aac_eld_residual64_multicore_fixtures.py`.
132 programmes use48kHz,480/512-sample ELD cores,two source packets per15/16-slot
spatial frame,all8 trees and both TTT variants. Modes are plain,repeated header,
delayed burst,fragmented message,STP and GES. Each has a residual MP4 and Matroska
video (264 acceptance movies) and a paired ordinary MP4 control. Six additional
24-frame bare-core MP4 controls have independent scalar PCM. All syntax,pictures
and parameters are authored locally; no private media or parameter sets.

These streams already decode with the existing runtime. This is extended
qualification,not a decoder bug fix. Production source is unchanged. The core
generator's optional frame_count=24 preserves its existing12-frame default.
The matrix-control helper's optional temporal bits preserve its empty default.

Whole residual PCM compares10,570,752 samples against analytical ELD core,
separately parsed1024-coefficient long/eight-short residual spectra,previously qualified64-band residual
MDCT/QMF conversion and separately signalled corrected matrix inputs. Zeroed
residual and uncorrected-ICC controls independently prove both signal effects.
STP/GES controls carry separately authored matching temporal flags/envelopes.
This composition is not independent whole-codec conformance.

Both containers require exact owned/frontend PCM,AudioStream playback,rewind,
forward/backward/middle/EOF seek and repeated ranges. AudioWorker capture adds
live-history navigation,three pairs of source-window boundary probes and exact
MP4 clocks/<1ns Matroska publication rounding. Matroska uses a1ns cluster clock.

Ordinary tests read frozen files only; own generation uses no FFmpeg,network or
external process. Optional independent C reference generation is separate.
Residual32/128,LFE residual,time alignment,protected spatial AAC,other profile
combinations,physical audio output and real-time throughput remain unqualified.
