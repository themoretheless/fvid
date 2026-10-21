# Owned full-resolution MBAFF component residual/QP deblocking

Generate separately with `python3 scripts/generate_avc_444_mbaff_component_deblock.py`.
Ordinary tests use the saved synthetic files offline, without external codecs,
FFmpeg, network access or private parameters/samples.

1728 I/P/P movies combine CAVLC and CABAC init 0/1/2; 8/8, 10/12 and 14/8
depths; frame/field/columns/checker pairs; AC4/AC8 scans; all/Cb-only/Cr-only
activity; PPS offsets 0/0 and -6/+6; and filter IDC 0/1/2. All components retain
32x64 samples and are cropped to 30x60 at presentation. Source planes are
synthetic smooth block patterns; the middle picture has zero motion and
independent component residual syntax. The echo uses the filtered DPB picture.

The scalar oracle applies component-specific residual QPs, integer inverse
transforms, physical edge ownership, nonzero bS=2, frame/field bS=1 without
residuals, full-resolution filtering and slice-edge suppression. Zero residual
alone does not imply zero filtering across mixed frame/field edges. Quantizer
mapping follows ITU-T H.264 (06/2019), 8.5.8/table 8-15; 4:4:4 residual and
filter substitution follows 8.5.5 and 8.7. Normative source:
https://www.itu.int/rec/T-REC-H.264-201906-S/en

Witnesses require changed samples for filtering, falsely sharing component
nonzero flags, ignoring slice boundaries, using luma QP in chroma filtering,
and using luma QP in chroma residual reconstruction. Tests assert actual PPS
QP offsets and filter IDC, compare every native coded sample and normalized
cropped MP4 byte, and repeat reset/rewind with exact 40 ms presentation timing.
This qualifies the stated combinations, not every H.264 scaling/tool/profile.
