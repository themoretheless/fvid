# In-band arbitrary-downmix mode transitions

Generate movies explicitly with
`python3 scripts/generate_aac_ld_mps_arbdmx_switch_fixtures.py`.
Generate the separate matrix oracle explicitly with
`python3 scripts/generate_aac_ld_mps_arbdmx_switch_reference.py <saoc-reference.zip>`.
Ordinary tests consume frozen files without generation,network,FFmpeg or C.

330 own MP4/MKV programmes cover trees0..4,23/9/4 bands and all six TTT modes
through three paired-mode configurations at quantization mode0. Ten independently
saved mono/stereo PCM inputs cover512-sample ELD,short ELD and LD-SBR PCM at
12/24/48/96kHz with32/64/128 spatial banks. Each programme has12 source packets.
No private media or codec parameters are used.

Both initial gain-mode states alternate at packets3/6/9;identical headers repeat
at2/5/8/11. Dependent gain sets hold prior values across identical SSCs. Re-enabled
parameter contexts must hold the default0dB rather than a stale earlier gain.
OTT/TTT main parameters are explicitly coded. A separate continuous always-on
MP4 codes the effective gains absolutely,using0dB when the changing mode is off.
The resulting raw M1 endpoints and uninterrupted filters must produce identical
PCM. No reconfiguration is needed in that control path.

132 unique matrix geometries have5,253,120 original C/80-digit Decimal endpoints.
This is component integration evidence,not independent whole-decoder/profile
conformance. A negative control resets the entire MPS renderer at each header;
its PCM must measurably differ,so state-reset behavior cannot pass unnoticed.

Tests check both transports,exports,timed/untimed APIs,source/spatial-edge seeks,
rewind/EOF,checkpoints and suffix retry. Eight own MP4 counterexamples truncate
frame data after a valid changed SSC;valid retry must preserve core,spatial,
configuration and timing state and the published PCM prefix. Root AudioWorker
checks live-history navigation and source/queue stamps for all660 transports.

This qualifies existing runtime behavior;no decoder implementation was changed.
Shared-QMF ingress,480-sample cores,energy-quantization combinations,STP/GES gain-
mode transitions and full Annex B/profile conformance are not newly qualified.
Physical output and realtime performance remain separate measurements. Trees5/6/7
arbitrary-downmix interpretation remains unqualified and explicitly refused.
