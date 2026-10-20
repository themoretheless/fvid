#!/usr/bin/env python3
"""Owned PAFF field FMO (field_pic_flag=1, num_slice_groups_minus1>0) I/P/B CAVLC fixtures."""
import itertools
import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from avc_fixture_mp4 import mux
from generate_avc_field_fmo_samples import configuration
from generate_avc_mbaff_direct_samples import Writer

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/avc-field-fmo-inter"


def map_for(kind, direction):
    return {
        0: [0, 1, 0, 1],
        1: [0, 1, 1, 0],
        2: [0, 1, 1, 1],
        3: [0, 1, 0, 1] if direction else [1, 1, 0, 0],
        4: [1, 1, 0, 0] if direction else [0, 0, 1, 1],
        5: [1, 0, 1, 0] if direction else [0, 1, 0, 1],
        6: [0, 1, 0, 1],
    }[kind]


def source_field_planes(depth, bottom):
    maximum = (1 << depth) - 1
    y_plane = [
        (x * 173 + (2 * y + int(bottom)) * 97 + x * y * 11 + 37) & maximum
        for y in range(32)
        for x in range(32)
    ]
    cb_plane = [
        (x * 131 + (2 * y + int(bottom)) * 61 + 53) & maximum
        for y in range(16)
        for x in range(16)
    ]
    cr_plane = [
        (x * 149 + (2 * y + int(bottom)) * 79 + 89) & maximum
        for y in range(16)
        for x in range(16)
    ]
    return y_plane, cb_plane, cr_plane


def write_pcm_mb(b, depth, y_plane, cb_plane, cr_plane, address):
    b.ue(25)
    b.align()
    mx = address % 2
    my = address // 2
    for row in range(16):
        base = (my * 16 + row) * 32 + mx * 16
        for col in range(16):
            b.u(y_plane[base + col], depth)
    for plane in (cb_plane, cr_plane):
        for row in range(8):
            base = (my * 8 + row) * 16 + mx * 8
            for col in range(8):
                b.u(plane[base + col], depth)


def i_field_slice(addresses, kind, bottom, index, depth, y_plane, cb_plane, cr_plane):
    b = Writer()
    b.ue(addresses[0])
    b.ue(2)  # I slice
    b.ue(0)  # pps_id = 0
    b.u(0, 4)  # frame_num = 0
    b.u(1)  # field_pic_flag = 1
    b.u(int(bottom))
    if index == 0:
        b.ue(0)  # idr_pic_id
    b.u(index, 4)  # pic_order_cnt_lsb = 0 or 1
    if index == 0:
        b.u(0)  # no_output_of_prior_pics_flag
        b.u(0)  # long_term_reference_flag
    else:
        b.u(0)  # adaptive_ref_pic_marking_mode_flag
    b.se(0)  # slice_qp_delta
    b.ue(1)  # disable_deblocking_filter_idc = 1 (all I_PCM)
    if kind in [3, 4, 5]:
        b.u(2, 3)  # slice_group_change_cycle = 2
    for address in addresses:
        write_pcm_mb(b, depth, y_plane, cb_plane, cr_plane, address)
    return b.nal(0x65 if index == 0 else 0x41)


def p_field_slice(addresses, kind, bottom, index, deblock, group, residual, depth):
    b = Writer()
    b.ue(addresses[0])
    b.ue(0)  # P slice
    b.ue(0)  # pps_id = 0
    b.u(1, 4)  # frame_num = 1
    b.u(1)  # field_pic_flag = 1
    b.u(int(bottom))
    b.u(4 + index, 4)  # pic_order_cnt_lsb = 4 or 5
    b.u(0)  # num_ref_idx_active_override_flag = 0
    b.u(0)  # ref_pic_list_modification_flag_l0 = 0
    b.u(0)  # adaptive_ref_pic_marking_mode_flag = 0
    b.se(-26 - 6 * (depth - 8))  # QP_Y = 0
    b.ue(deblock)
    if deblock != 1:
        b.se(0)
        b.se(0)
    if kind in [3, 4, 5]:
        b.u(2, 3)
    mv = (16, 0) if group == 0 else (-16, 0)
    for i, _ in enumerate(addresses):
        b.ue(0)  # mb_skip_run = 0
        b.ue(0)  # P_L0_16x16
        if i == 0:
            b.se(mv[0])
            b.se(mv[1])
        else:
            b.se(0)
            b.se(0)
        if i == 0 and residual:
            b.ue(11)  # Inter CBP code 11 -> pattern 15 (luma AC, no chroma)
            b.se(0)  # mb_qp_delta = 0
            for block in range(16):
                b.u(5, 6)  # coeff_token for total=1, trailing=0 (nC=0 -> '000101')
                if block % 2 == 0:
                    b.u(1, 5)  # level=+4 -> prefix=4 ('00001')
                else:
                    b.u(1, 6)  # level=-4 -> prefix=5 ('000001')
                b.u(1, 1)  # total_zeros = 0 ('1')
        else:
            b.ue(0)  # CBP = 0
    return b.nal(0x41)


def b_field_slice(addresses, kind, bottom, index, deblock, skip_mode, spatial):
    b = Writer()
    b.ue(addresses[0])
    b.ue(1)  # B slice
    b.ue(0)  # pps_id = 0
    b.u(2, 4)  # frame_num = 2
    b.u(1)  # field_pic_flag = 1
    b.u(int(bottom))
    b.u(2 + index, 4)  # pic_order_cnt_lsb = 2 or 3
    b.u(int(spatial))  # direct_spatial_mv_pred_flag
    b.u(0)  # num_ref_idx_active_override_flag = 0
    b.u(0)  # ref_pic_list_modification_flag_l0 = 0
    b.u(0)  # ref_pic_list_modification_flag_l1 = 0
    b.se(-26)  # slice_qp_delta = -26 -> slice_qp = 0
    b.ue(deblock)
    if deblock != 1:
        b.se(0)
        b.se(0)
    if kind in [3, 4, 5]:
        b.u(2, 3)
    if skip_mode:
        b.ue(len(addresses))
    else:
        for _ in addresses:
            b.ue(0)  # mb_skip_run = 0
            b.ue(0)  # B_Direct_16x16
            b.ue(0)  # CBP = 0
    return b.nal(0x01)


def shift_field_mb(plane, width, height, mb, dx_pixels, depth, residual=False):
    maximum = (1 << depth) - 1
    bw = width // 2
    bh = height // 2
    mx = mb % 2
    my = mb // 2
    out = [0] * (bw * bh)
    for row in range(bh):
        sy = my * bh + row
        for col in range(bw):
            sx = max(0, min(width - 1, mx * bw + col + dx_pixels))
            val = plane[sy * width + sx]
            if residual:
                block = (row // 4) * 4 + (col // 4)
                sign = 1 if block % 2 == 0 else -1
                delta = (sign * 40 + 32) >> 6
                val = max(0, min(maximum, val + delta))
            out[row * bw + col] = val
    return out


def copy_mb_into(dst, width, height, mb, block_pixels):
    bw = width // 2
    bh = height // 2
    mx = mb % 2
    my = mb // 2
    for row in range(bh):
        base = (my * bh + row) * width + mx * bw
        dst[base : base + bw] = block_pixels[row * bw : (row + 1) * bw]


def weave_pair_planes(top, bottom):
    out = []
    for p, (w, h) in enumerate([(32, 32), (16, 16), (16, 16)]):
        for y in range(h):
            out.extend(top[p][y * w : (y + 1) * w])
            out.extend(bottom[p][y * w : (y + 1) * w])
    return out


def generate():
    OUT.mkdir(parents=True, exist_ok=True)
    cases = []
    temporal_witnesses = 0
    residual_witnesses = 0
    for depth, kind in itertools.product([8, 10], range(7)):
        for direction in [False, True] if kind in [3, 4, 5] else [False]:
            mbmap = map_for(kind, direction)
            config = configuration(depth, kind, direction)
            for reverse, spatial, skip_mode, residual, deblock, aso in itertools.product(
                [False, True],
                [False, True],
                [False, True],
                [False, True],
                [1, 0, 2],
                [False, True],
            ):
                # Orthogonal matrix:
                # 1. Core sweep: all 10 (kind, direction) * 2 depth * 2 reverse * 2 spatial * 2 skip_mode * 2 aso at deblock=1, residual=False (320 cases)
                # 2. Residual & filter sweep: kind in (0, 1, 2, 6), direction=False, reverse=False, skip_mode in (False, True), aso in (False, True), spatial in (False, True), deblock in (0, 2) or residual=True (160 cases)
                if deblock != 1 or residual:
                    if kind in [3, 4, 5] or reverse:
                        continue
                order = [True, False] if reverse else [False, True]
                groups = [1, 0] if aso else [0, 1]

                i_fields = {}
                p_fields = {}
                b_fields = {}
                i_packets = []
                p_packets = []
                b_packets = []

                for idx, bottom in enumerate(order):
                    y0, cb0, cr0 = source_field_planes(depth, bottom)
                    i_fields[bottom] = (y0, cb0, cr0)
                    i_nals = [
                        i_field_slice(
                            [a for a, g in enumerate(mbmap) if g == group],
                            kind,
                            bottom,
                            idx,
                            depth,
                            y0,
                            cb0,
                            cr0,
                        )
                        for group in groups
                    ]
                    i_packets.append(b"".join(len(n).to_bytes(4, "big") + n for n in i_nals))

                    # P field reconstruction: group 0 -> mv=(+16, 0) [+4 luma, +2 chroma], group 1 -> mv=(-16, 0) [-4 luma, -2 chroma]
                    yp = [0] * (32 * 32)
                    cbp = [0] * (16 * 16)
                    crp = [0] * (16 * 16)
                    for group in [0, 1]:
                        addrs = [a for a, g in enumerate(mbmap) if g == group]
                        dx_luma = 4 if group == 0 else -4
                        dx_chroma = 2 if group == 0 else -2
                        for local_i, mb in enumerate(addrs):
                            with_res = shift_field_mb(
                                y0, 32, 32, mb, dx_luma, depth, residual=(local_i == 0 and residual)
                            )
                            if local_i == 0 and residual:
                                no_res = shift_field_mb(y0, 32, 32, mb, dx_luma, depth, residual=False)
                                residual_witnesses += sum(a != b for a, b in zip(with_res, no_res))
                            copy_mb_into(yp, 32, 32, mb, with_res)
                            copy_mb_into(
                                cbp,
                                16,
                                16,
                                mb,
                                shift_field_mb(cb0, 16, 16, mb, dx_chroma, depth, residual=False),
                            )
                            copy_mb_into(
                                crp,
                                16,
                                16,
                                mb,
                                shift_field_mb(cr0, 16, 16, mb, dx_chroma, depth, residual=False),
                            )
                    p_fields[bottom] = (yp, cbp, crp)
                    p_nals = [
                        p_field_slice(
                            [a for a, g in enumerate(mbmap) if g == group],
                            kind,
                            bottom,
                            idx,
                            deblock,
                            group,
                            residual,
                            depth,
                        )
                        for group in groups
                    ]
                    p_packets.append(b"".join(len(n).to_bytes(4, "big") + n for n in p_nals))

                    # B field reconstruction
                    yb = [0] * (32 * 32)
                    cbb = [0] * (16 * 16)
                    crb = [0] * (16 * 16)
                    for group in [0, 1]:
                        addrs = [a for a, g in enumerate(mbmap) if g == group]
                        for mb in addrs:
                            if spatial:
                                l0_dx_y, l1_dx_y = 0, 0
                                l0_dx_c, l1_dx_c = 0, 0
                            else:
                                # Temporal direct scales co-located P field mv (+/-16, 0) by 1/2:
                                # L0 mv = (+/-8, 0) -> +/-2 luma px, +/-1 chroma px
                                # L1 mv = (-/+8, 0) -> -/+2 luma px, -/+1 chroma px
                                l0_dx_y = 2 if group == 0 else -2
                                l1_dx_y = -2 if group == 0 else 2
                                l0_dx_c = 1 if group == 0 else -1
                                l1_dx_c = -1 if group == 0 else 1
                            l0_y = shift_field_mb(y0, 32, 32, mb, l0_dx_y, depth)
                            l1_y = shift_field_mb(yp, 32, 32, mb, l1_dx_y, depth)
                            l0_cb = shift_field_mb(cb0, 16, 16, mb, l0_dx_c, depth)
                            l1_cb = shift_field_mb(cbp, 16, 16, mb, l1_dx_c, depth)
                            l0_cr = shift_field_mb(cr0, 16, 16, mb, l0_dx_c, depth)
                            l1_cr = shift_field_mb(crp, 16, 16, mb, l1_dx_c, depth)
                            by_mb = [(a + b + 1) // 2 for a, b in zip(l0_y, l1_y)]
                            bcb_mb = [(a + b + 1) // 2 for a, b in zip(l0_cb, l1_cb)]
                            bcr_mb = [(a + b + 1) // 2 for a, b in zip(l0_cr, l1_cr)]
                            if not spatial:
                                zero_y = [
                                    (a + b + 1) // 2
                                    for a, b in zip(
                                        shift_field_mb(y0, 32, 32, mb, 0, depth),
                                        shift_field_mb(yp, 32, 32, mb, 0, depth),
                                    )
                                ]
                                temporal_witnesses += sum(a != b for a, b in zip(by_mb, zero_y))
                            copy_mb_into(yb, 32, 32, mb, by_mb)
                            copy_mb_into(cbb, 16, 16, mb, bcb_mb)
                            copy_mb_into(crb, 16, 16, mb, bcr_mb)
                    b_fields[bottom] = (yb, cbb, crb)
                    b_nals = [
                        b_field_slice(
                            [a for a, g in enumerate(mbmap) if g == group],
                            kind,
                            bottom,
                            idx,
                            deblock,
                            skip_mode,
                            spatial,
                        )
                        for group in groups
                    ]
                    b_packets.append(b"".join(len(n).to_bytes(4, "big") + n for n in b_nals))

                name = (
                    f"avc-field-fmo-inter-{depth}bit-type{kind}-dir{int(direction)}-"
                    + ("bottom-first-" if reverse else "top-first-")
                    + ("spatial-" if spatial else "temporal-")
                    + ("skip-" if skip_mode else "direct-")
                    + f"res{int(residual)}-filter{deblock}"
                    + ("-aso" if aso else "")
                )
                packets = i_packets + p_packets + b_packets
                frames = [
                    (pts, i == 0, pkt)
                    for i, (pts, pkt) in enumerate(zip([0, 1, 4, 5, 2, 3], packets))
                ]
                i_woven = weave_pair_planes(i_fields[False], i_fields[True])
                b_woven = weave_pair_planes(b_fields[False], b_fields[True])
                p_woven = weave_pair_planes(p_fields[False], p_fields[True])
                # Presentation order: I0 (pts 0), B2 (pts 2), P4 (pts 4)
                golden = i_woven + b_woven + p_woven
                if depth == 8:
                    oracle_bytes = bytes(golden)
                else:
                    oracle_bytes = struct.pack(f"<{len(golden)}H", *golden)
                mp4_bytes = mux(config, frames, 32, 64, 50)
                (OUT / f"{name}.mp4").write_bytes(mp4_bytes)
                (OUT / f"{name}.yuv").write_bytes(oracle_bytes)
                cases.append(
                    dict(
                        name=name,
                        depth=depth,
                        kind=kind,
                        direction=direction,
                        bottom_first=reverse,
                        spatial=spatial,
                        skip_mode=skip_mode,
                        residual=residual,
                        deblock=deblock,
                        aso=aso,
                    )
                )
    assert len(cases) == 640, len(cases)
    assert temporal_witnesses > 0 and residual_witnesses > 0
    (OUT / "manifest.json").write_text(json.dumps(cases, indent=2) + "\n")


if __name__ == "__main__":
    generate()
