# Expanded AHT in MP4 with a silent base 5.1

Regenerate separately: `python3 scripts/generate_eac3_mp4_aht.py`.
Eighteen owned 48 kHz cases use six-block programmes: an independently coded
silent base 5.1 plus a nonzero dependent rear stereo pair. The pair exercises
AHT masks 1/2/3, stationary/mixed spectra and unity/changing/boosting final gain.
Each MP4 has neutral VP9 video and three E-AC-3 samples, each one six-block
programme. dec3 describes base acmod=7/lfeon=1, one dependent and chan_loc=2;
its bitrate counts the complete programme. Sample-entry channelcount=2 remains
a placeholder, while decoded output is eight channels with mask 0x63f.

Independent scalar controls and analytical rounding bounds check assembled PCM;
header probes assert actual active AHT. Tests additionally require the base
control PCM to be zero and the final dependent signal to be nonzero. They accept
direct assembly/reset, ordinary decoder dispatch, MP4 PCM/PTS/duration and worker
layout, rewind and partial-sample seeks at 5/40/80 ms. This is not a physical
device/GUI test or an independent external-codec oracle. No production fix was
needed for these cases; they qualify an existing path.

163 owned artifacts (3,459,437 bytes excluding README), no private media or codec
parameter sets. Generator/test execution are separate. Regeneration forbids
subprocess/socket creation; ordinary tests use neither FFmpeg nor network.

This specifically qualifies dependent AHT with a silent base 5.1 at 48 kHz.
Sounding base 5.1, other rates, AHT quantizers, coupling/SPX combinations and
changing maps still require separate acceptance. It does not prove all E-AC-3.
