# Stereo arbitrary-downmix gains before TTT inversion

Generate movies explicitly with
`python3 scripts/generate_aac_ld_mps_arbdmx_ttt_fixtures.py`.
Generate the separate component oracle explicitly with
`python3 scripts/generate_aac_ld_mps_arbdmx_ttt_reference.py <saoc-reference.zip>`.
Ordinary Rust tests consume frozen files without generation,network,FFmpeg or
external C reference compilation.

270 own MP4/MKV pairs cover TTT trees2/3/4,all six TTT mode combinations,
signalled inversion/bypass,23/9/4 parameter bands and unequal L/R gains varying
in time/frequency. The first frame uses unity gains. Five independent saved
stereo PCM controls cover plain short ELD,downscaled LD-SBR and nominal48kHz
LD-SBR,with32/64/128-band spatial PCM at12/24/48/96kHz. Each movie has12 source
packets. No private media or codec parameters are copied.

Before the fix all five bare controls pass while three spatial tests refuse
at the exact old arbitrary-downmix matrix constructor. Gains now scale the TTT
input basis before the inverse. Scaling final M1 columns instead fails a
noncommutative negative control. M2 and rendered PCM are not gain substitutes.

The separate oracle uses unmodified original525/7271/7272 routines and80-digit
Decimal temporal smoothing.108 unique matrix geometries (tree,resolution,mode
combination,slot count) cover4,769,280 endpoints;identical PCM-clock-independent
matrices are not stored repeatedly for all270 transports. This is component
integration evidence,not independent whole-codec/profile conformance.

Checks cover timed/untimed APIs,MP4/MKV export/playback,source/spatial edge seek,
rewind/EOF,checkpoint and malformed-suffix retry,truncated-gain rollback with
unchanged cursor,AudioWorker queue stamps and live-history navigation. A direct
builder test checks all six modes with inversion on/off,unity identity,unchanged
M2,invalid linear gains and the incorrect post-inversion ordering.

Mono downmix support is retained. Trees5/6/7,HRTF,arbitrary-tree/downmix-residual
extensions remain explicitly refused. Dynamic SSC gain-mode enable/disable,
shared-QMF LD-SBR ingress,physical output,realtime performance and complete
Annex B conformance are not newly qualified here.
