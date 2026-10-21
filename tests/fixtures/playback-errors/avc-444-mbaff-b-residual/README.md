# Owned retained 4:4:4 MBAFF B residual/scaling/deblocking interactions

Generate separately with `python3 scripts/generate_avc_444_mbaff_b_residual.py`.
Ordinary tests use saved short synthetic movies offline. No FFmpeg, external
encoder/decoder, network, private samples or copied private parameters are used.

8640 I/I/B/P movies combine three precision pairs (8/8, 10/12, 14/8), CAVLC
and CABAC init 0/1/2, four MBAFF pair topologies, four residual-activity patterns
(all/Cb-only/Cr-only/per-block checker), and filter IDC 0/1/2. Eight B partition
patterns carry AC4; seven legal patterns carry AC8. They include uni-L0,
mixed L0/L1, uni/bi mixtures, direct-only and small mixed subpartitions. AC8
is never emitted where the B subpartition syntax prohibits an 8x8 transform.
QP offsets, explicit prediction weights and custom scaling are selected by a
documented cycling schedule in the generator rather than a full Cartesian
product of those selectors. Both flat and explicit twelve-list PPS scaling
occur, with distinct inter Y/Cb/Cr weights for 4x4 and 8x8 transforms.

Independent scalar list/ref/motion grids generate qpel predictions. Component
precision, prediction clipping, mapped QPs, integer inverse transforms and
frame/field residual scans produce raw B samples. Physical edge ownership
then applies component-specific nonzero bS=2, mixed-field bS=1, and two-list
reference/vector strength with correct frame/field vertical thresholds and
swapped-list equivalence. Sparse checker activity varies within each component
and macroblock; coefficient/CABAC histories are independently maintained.
Witnesses distinguish residual omission, filter omission, false shared component
flags, missing motion strengths, ignored slice boundaries and flat scaling.

B is explicitly marked as a reference picture with the required reference
marking syntax. The final P (frame_num=3) predicts that retained B with no
residual/filtering, checking publication of the filtered reference. Source
and future PCM references are unfiltered. Tests assert actual PPS scaling/QP/
weight flags, deblock IDC, slice/reference/frame numbering and ASO, compare all
coded native planes and cropped normalized software-MP4 bytes, and repeat
reset/rewind with exact presentation ordering and 40 ms timing.

This matrix qualifies its saved combinations; it is not complete H.264/profile
conformance, all selector interactions, arbitrary DPB history or a 60-fps gate.
