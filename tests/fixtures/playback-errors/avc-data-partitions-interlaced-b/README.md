# Owned two-reference interlaced B partitions

Generate separately with `python3 scripts/generate_avc_partitioned_interlaced_b.py`.
The generator writes 2,179 artifacts (2,777,754 bytes before this README), with
PATH empty and commands/network denied. It uses original synthetic SPS/PPS and
nonzero PCM seed/future references. Ordinary tests read stored artifacts and
require neither generation, FFmpeg nor network access.

There are 128 five-frame 32x32 videos, equally divided between MBAFF and PAFF.
Cases cover L0/L1/Bi, temporal/spatial direct, B-skip, mixed PCM/L1, all adaptive
pair layouts, both field orders, single/split PAFF slices and ASO/interleaved
A/B/C. Two references differ by four in every component. Thus L0/L1/Bi have
separate scalar expected offsets0/4/2. Signed C residuals add a literal horizontal
frame or vertical field transform basis; PCM regions use original seed samples.

Decode order is 0/3/1/2/4 with non-reference B partitions (nal_ref_idc=0) and two
B pictures sharing frame_num2. The scalar oracle is in presentation order. PAFF
list modifications independently bind L0 to the past same-parity field and L1
to the future same-parity field. A successor P checks reference selection after
B: default lists are used in MBAFF and on the first PAFF field, so erroneously
retaining B can change output. The second PAFF field explicitly selects the
future reference to avoid accidentally selecting the newly decoded opposite
field of the P pair.

Native decoding checks all samples, controls and exact field-pair completion
through reset. Every packet has an independently serialized combined control.
Software MP4 must reorder five complete frames with exact 40ms PTS/duration
through EOF, rewind and sync seek. Metadata checks 640 actual B headers, both
reference-list modifications, field/POC/frame_num, direct mode, non-reference
NAL classes, B/C presence and exact byte caps. Literal future/successor goldens
are additionally checked to be exactly seed+4.

The existing native decoder passed this finite matrix without a production
change. Reference-coded B, nonzero motion, weight/filter/FMO combinations,
arbitrary dimensions and missing-primary recovery still need partitioned-stream
qualification. This is not proof of complete interlaced B or all codec coverage.
