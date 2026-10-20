#!/usr/bin/env python3
"""Own separate-colour-plane 8x8 transform, 12-list 8x8 scaling, ASO and deblocking fixtures."""
import json, struct, itertools
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/avc-separate-8x8-deblock"

NORMALIZATION = [
    [20, 18, 32, 19, 25, 24],
    [22, 19, 35, 21, 28, 26],
    [26, 23, 42, 24, 33, 31],
    [28, 25, 45, 26, 35, 33],
    [32, 28, 51, 30, 40, 38],
    [36, 32, 58, 34, 46, 43],
]
CATEGORIES = [[0, 3, 4, 3], [3, 1, 5, 1], [4, 5, 2, 5], [3, 1, 5, 1]]


def butterfly(d):
    e = [
        d[0] + d[4],
        -d[3] + d[5] - d[7] - (d[7] >> 1),
        d[0] - d[4],
        d[1] + d[7] - d[3] - (d[3] >> 1),
        (d[2] >> 1) - d[6],
        -d[1] + d[7] + d[5] + (d[5] >> 1),
        d[2] + (d[6] >> 1),
        d[3] + d[5] + d[1] + (d[1] >> 1),
    ]
    f = [
        e[0] + e[6],
        e[1] + (e[7] >> 2),
        e[2] + e[4],
        e[3] + (e[5] >> 2),
        e[2] - e[4],
        (e[3] >> 2) - e[5],
        e[0] - e[6],
        e[7] - (e[1] >> 2),
    ]
    return [
        f[0] + f[7],
        f[2] + f[5],
        f[4] + f[3],
        f[6] + f[1],
        f[6] - f[1],
        f[4] - f[3],
        f[2] - f[5],
        f[0] - f[7],
    ]


def residual_8x8_single(raster_pos, level, qp, depth, weight):
    q = qp + 6 * (depth - 8)
    factor = NORMALIZATION[q % 6][CATEGORIES[(raster_pos // 8) % 4][raster_pos % 4]]
    value = level * weight * factor
    shift = q // 6
    scaled = value << (shift - 6) if shift >= 6 else (value + (1 << (5 - shift))) >> (6 - shift)
    block = [0] * 64
    block[raster_pos] = scaled
    for y in range(8):
        block[y * 8 : (y + 1) * 8] = butterfly(block[y * 8 : (y + 1) * 8])
    out = [0] * 64
    for x in range(8):
        col = butterfly([block[y * 8 + x] for y in range(8)])
        for y in range(8):
            out[y * 8 + x] = (col[y] + 32) >> 6
    return out


def write_constant_list(b, size, weight):
    b.u(1)
    b.se(weight - 8)
    for _ in range(size - 1):
        b.se(0)


def configuration(depth, cabac, scaling_mode, paff=False):
    b = Writer()
    for v in [244, 0, 41]:
        b.u(v, 8)
    b.ue(0)  # seq_parameter_set_id
    b.ue(3)  # chroma_format_idc = 3
    b.u(1)  # separate_colour_plane_flag = 1
    b.ue(depth - 8)
    b.ue(0)  # bit_depth_chroma_minus8
    b.u(0)  # qpprime_y_zero_transform_bypass_flag
    has_sps_scaling = scaling_mode in ("sps", "fallback")
    b.u(int(has_sps_scaling))
    if has_sps_scaling:
        for w in [16] * 6:
            write_constant_list(b, 16, w)
        if scaling_mode == "sps":
            for w in [8, 12, 16, 20, 24, 28]:
                write_constant_list(b, 64, w)
        else:
            # Rule A/B fallback: specify Intra Y (8) and Inter Y (12), inherit Cb/Cr (lists 8..11 = 0).
            write_constant_list(b, 64, 8)
            write_constant_list(b, 64, 12)
            for _ in range(4):
                b.u(0)
    b.ue(0)  # log2_max_frame_num_minus4
    b.ue(0)  # pic_order_cnt_type
    b.ue(0)  # log2_max_pic_order_cnt_lsb_minus4
    b.ue(2)  # max_num_ref_frames
    b.u(0)  # gaps_in_frame_num_value_allowed_flag
    b.ue(1)  # pic_width_in_mbs_minus1 (32 px)
    b.ue(0)  # pic_height_in_map_units_minus1 (16 px frame, 32 px PAFF frame)
    b.u(int(not paff))  # frame_mbs_only_flag
    if paff:
        b.u(0)  # mb_adaptive_frame_field_flag
    b.u(1)  # direct_8x8_inference_flag
    b.u(1)  # frame_cropping_flag
    for _ in range(4):
        b.ue(1)
    b.u(0)  # vui_parameters_present_flag
    sps = b.nal(0x67)

    b = Writer()
    b.ue(0)  # pic_parameter_set_id
    b.ue(0)  # seq_parameter_set_id
    b.u(int(cabac))
    b.u(0)  # bottom_field_pic_order_in_frame_present_flag
    b.ue(0)  # num_slice_groups_minus1
    b.ue(0)  # num_ref_idx_l0_default_active_minus1
    b.ue(0)  # num_ref_idx_l1_default_active_minus1
    b.u(0)  # weighted_pred_flag
    b.u(0, 2)  # weighted_bipred_idc
    b.se(0)  # pic_init_qp_minus26
    b.se(0)  # pic_init_qs_minus26
    b.se(0)  # chroma_qp_index_offset
    b.u(1)  # deblocking_filter_control_present_flag
    b.u(0)  # constrained_intra_pred_flag
    b.u(0)  # redundant_pic_cnt_present_flag
    b.u(1)  # transform_8x8_mode_flag = 1
    has_pps_scaling = scaling_mode in ("pps", "fallback")
    b.u(int(has_pps_scaling))
    if has_pps_scaling:
        for _ in range(6):
            b.u(0)
        if scaling_mode == "pps":
            for w in [24, 28, 16, 20, 8, 12]:
                write_constant_list(b, 64, w)
        else:
            # Override Intra Cb (list 8 = 20) and Inter Cb (list 9 = 28); Cr inherits Cb (lists 10, 11 = 0); Y inherits SPS (lists 6, 7 = 0).
            b.u(0)
            b.u(0)
            write_constant_list(b, 64, 20)
            write_constant_list(b, 64, 28)
            b.u(0)
            b.u(0)
    b.se(0)  # second_chroma_qp_index_offset
    pps = b.nal(0x68)
    return (
        bytes([1, 244, 0, 41, 255, 225])
        + len(sps).to_bytes(2, "big")
        + sps
        + bytes([1])
        + len(pps).to_bytes(2, "big")
        + pps
    )


def weights_for_mode(scaling_mode):
    if scaling_mode == "sps":
        return ([8, 16, 24], [12, 20, 28])
    if scaling_mode == "pps":
        return ([24, 16, 8], [28, 20, 12])
    return ([8, 20, 20], [12, 28, 28])


def write_slice_header(b, first_mb, slice_type, plane, frame_num, poc_lsb, is_idr, deblock_idc, paff=False, bottom=False, ref_pic=True, cabac=False):
    b.ue(first_mb)
    b.ue(slice_type)
    b.ue(0)  # pic_parameter_set_id
    b.u(plane, 2)
    b.u(frame_num, 4)
    if paff:
        b.u(1)  # field_pic_flag
        b.u(int(bottom))
    if is_idr:
        b.ue(0)  # idr_pic_id
    b.u(poc_lsb, 4)
    if slice_type == 0:  # P slice
        b.u(0)  # num_ref_idx_active_override_flag
        b.u(0)  # ref_pic_list_modification_flag_l0
    if ref_pic:
        if is_idr:
            b.u(0)  # no_output_of_prior_pics_flag
            b.u(0)  # long_term_reference_flag
        else:
            b.u(0)  # adaptive_ref_pic_marking_mode_flag
    if cabac and slice_type != 2:
        b.ue(0)  # cabac_init_idc
    b.se(0)  # slice_qp_delta (slice_qp = 26)
    b.ue(deblock_idc)
    if deblock_idc != 1:
        b.se(0)  # slice_alpha_c0_offset_div2
        b.se(0)  # slice_beta_offset_div2


def encode_i8x8_slice(address, plane, depth, cabac, deblock_idc, sign, intra_weight, paff=False, bottom=False, frame_num=0, poc_lsb=0, is_idr=True):
    b = Writer()
    write_slice_header(b, address, 2, plane, frame_num, poc_lsb, is_idr, deblock_idc, paff=paff, bottom=bottom, ref_pic=True, cabac=cabac)
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(-1, 26)
        c.decision(3, 0)  # I_NxN (first MB of slice: left/top unavailable -> ctx 3)
        c.decision(399, 1)  # transform_size_8x8_flag = 1 (left/top unavailable -> ctx 399)
        for _ in range(4):
            c.decision(68, 1)  # prev_intra8x8_pred_mode_flag = 1 -> DC
        for _ in range(4):
            c.decision(73, 1)  # coded_block_pattern luma = 15 (unavailable or 1 -> inc = 0 -> ctx 73)
        c.decision(60, 0)  # mb_qp_delta = 0
        sig_ctx = 436 if paff else 402
        last_ctx = 451 if paff else 417
        for block in range(4):
            c.decision(sig_ctx, 1)  # pos 0 significant
            c.decision(last_ctx, 1)  # pos 0 last
            # coeff_abs_level_minus1 = 3 (|level| = 4): ctx 427 -> 1, ctx 431 -> 1, 1, 0
            c.decision(427, 1)
            c.decision(431, 1)
            c.decision(431, 1)
            c.decision(431, 0)
            c.bypass(int(sign < 0))
        b.bits.extend(c.finish())
    else:
        b.ue(0)  # mb_type = 0 (I_NxN)
        b.u(1)  # transform_size_8x8_flag = 1
        for _ in range(4):
            b.u(1)  # prev_intra8x8_pred_mode_flag = 1 -> DC
        b.ue(0)  # coded_block_pattern code 0 -> luma 15 in monochrome/separate-plane
        b.se(0)  # mb_qp_delta = 0
        for sub in range(16):
            if sub % 4 == 0:
                b.u(5, 6)  # TotalCoeff=1, TrailingOnes=0 at nC=0
                b.u(1, 6 if sign < 0 else 5)  # level -4 or +4
                b.u(1)  # total_zeros = 0 (max_coeff=16)
            else:
                b.u(1)  # TotalCoeff=0 at nC=1 (from left/top subblock with coeff=1)
    nal = b.nal(0x65 if is_idr else 0x41, trailing=not cabac)
    res = residual_8x8_single(0, 4 * sign, 26, depth, intra_weight)
    max_v = (1 << depth) - 1
    mb_samples = [0] * 256
    for block in range(4):
        bx = (block % 2) * 8
        by = (block // 2) * 8
        if block == 0:
            pred = 1 << (depth - 1)
        elif block == 1:
            # Block 1 has left (block 0) of constant value b0_val; top is unavailable
            pred = b0_val
        elif block == 2:
            # Block 2 has top (block 0 = b0_val) and top-right (block 1 = b1_val); left is unavailable.
            # H.264 8.3.2.2.2 filters t[7] with t[8] (from block 1): t'[7] = (3 * b0_val + b1_val + 2) >> 2.
            t7 = (3 * b0_val + b1_val + 2) >> 2
            pred = (7 * b0_val + t7 + 4) // 8
        else:
            # Block 3 has top (block 1 = b1_val), left (block 2 = b2_val), and corner (block 0 = b0_val).
            t0 = (b0_val + 3 * b1_val + 2) >> 2
            l0 = (b0_val + 3 * b2_val + 2) >> 2
            pred = (t0 + 7 * b1_val + l0 + 7 * b2_val + 8) // 16
        val = max(0, min(max_v, pred + res[0]))
        if block == 0:
            b0_val = val
        elif block == 1:
            b1_val = val
        elif block == 2:
            b2_val = val
        for y in range(8):
            for x in range(8):
                mb_samples[(by + y) * 16 + bx + x] = val
    return nal, mb_samples


def encode_p8x8_slice(address, plane, depth, cabac, deblock_idc, sign, inter_weight, base_samples, paff=False, bottom=False, frame_num=1, poc_lsb=2):
    b = Writer()
    write_slice_header(b, address, 0, plane, frame_num, poc_lsb, False, deblock_idc, paff=paff, bottom=bottom, ref_pic=True, cabac=cabac)
    # In P-slice:
    # For Frame mode, use scan_idx=1 -> raster pos 1 (row 0, col 1).
    # For PAFF field mode, use scan_idx=1 -> FIELD_SCAN[1] = raster pos 8 (row 1, col 0)!
    # Notice that if field_scan were NOT used in PAFF 8x8 inverse scan, scan_idx=1 would land at raster pos 1 (horizontal) instead of raster pos 8 (vertical)!
    raster_pos = 8 if paff else 1
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(0, 26)
        c.decision(11, 0)  # mb_skip_flag = 0
        c.decision(14, 0)  # mb_type = P_L0_16x16 (bin0=0, bin1=0, bin2=0)
        c.decision(15, 0)
        c.decision(16, 0)
        c.mvd(0, 0, 0)
        c.mvd(1, 0, 0)
        for _ in range(4):
            c.decision(73, 1)  # coded_block_pattern luma = 15 (unavailable or 1 -> inc = 0 -> ctx 73)
        c.decision(399, 1)  # transform_size_8x8_flag = 1
        c.decision(60, 0)  # mb_qp_delta = 0
        sig_base = 436 if paff else 402
        last_base = 451 if paff else 417
        for block in range(4):
            c.decision(sig_base + 0, 0)  # pos 0 not significant
            c.decision(sig_base + 1, 1)  # pos 1 significant
            c.decision(last_base + 1, 1)  # pos 1 last
            # coeff_abs_level_minus1 = 1 (|level| = 2): ctx 427 -> 1, ctx 431 -> 0
            c.decision(427, 1)
            c.decision(431, 0)
            c.bypass(int(sign < 0))
        b.bits.extend(c.finish())
    else:
        b.ue(0)  # mb_skip_run = 0
        b.ue(0)  # mb_type = 0 (P_L0_16x16)
        b.se(0)  # mvd_l0 x = 0
        b.se(0)  # mvd_l0 y = 0
        b.ue(9)  # coded_block_pattern code 9 -> luma 15 in inter monochrome/separate-plane
        b.u(1)  # transform_size_8x8_flag = 1
        b.se(0)  # mb_qp_delta = 0
        # scan_idx = 1 belongs to subblock stream 1 (since 4*0 + 1 = 1), at coefficient index 0 of stream 1!
        for group in range(4):
            for stream in range(4):
                if stream == 1:
                    b.u(5, 6)  # TotalCoeff=1, TrailingOnes=0 at nC=0
                    b.u(1, 2 if sign < 0 else 1)  # level -2 or +2
                    b.u(1)  # total_zeros = 0 (at index 0, so 0 zeros before it!)
                else:
                    # Note: for stream 0, nC=0; for stream 1, nC=0; for stream 2, left is stream 1 (TotalCoeff=1) so nC=1 -> still VLC0!
                    b.u(1)  # TotalCoeff=0 at nC<2
    nal = b.nal(0x41, trailing=not cabac)
    res8 = residual_8x8_single(raster_pos, 2 * sign, 26, depth, inter_weight)
    max_v = (1 << depth) - 1
    out = [0] * 256
    for by in range(2):
        for bx in range(2):
            for y in range(8):
                for x in range(8):
                    idx = (by * 8 + y) * 16 + bx * 8 + x
                    out[idx] = max(0, min(max_v, base_samples[idx] + res8[y * 8 + x]))
    return nal, out


def filter_edge_sample(p, q, qp, depth, strength):
    if strength == 0:
        return list(p), list(q)
    ALPHA = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 17, 20,
        22, 25, 28, 32, 36, 40, 45, 50, 56, 63, 71, 80, 90, 101, 113, 127, 144, 162, 182, 203, 226,
        255, 255,
    ]
    BETA = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 6, 6, 7, 7, 8, 8,
        9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18,
    ]
    TC1 = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13,
    ]
    TC2 = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2,
        2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 7, 8, 8, 10, 11, 12, 13, 15, 17,
    ]
    TC3 = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3,
        3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 23, 25,
    ]
    a = list(p)
    b = list(q)
    idx = max(0, min(51, qp))
    alpha = ALPHA[idx] << (depth - 8)
    beta = BETA[idx] << (depth - 8)
    if abs(p[0] - q[0]) >= alpha or abs(p[1] - p[0]) >= beta or abs(q[1] - q[0]) >= beta:
        return a, b
    ap = abs(p[2] - p[0]) < beta
    aq = abs(q[2] - q[0]) < beta
    if strength == 4:
        strong = abs(p[0] - q[0]) < (alpha >> 2) + 2
        if strong and ap:
            a[0] = (p[2] + 2 * p[1] + 2 * p[0] + 2 * q[0] + q[1] + 4) >> 3
            a[1] = (p[2] + p[1] + p[0] + q[0] + 2) >> 2
            a[2] = (2 * p[3] + 3 * p[2] + p[1] + p[0] + q[0] + 4) >> 3
        else:
            a[0] = (2 * p[1] + p[0] + q[1] + 2) >> 2
        if strong and aq:
            b[0] = (q[2] + 2 * q[1] + 2 * q[0] + 2 * p[0] + p[1] + 4) >> 3
            b[1] = (q[2] + q[1] + q[0] + p[0] + 2) >> 2
            b[2] = (2 * q[3] + 3 * q[2] + q[1] + q[0] + p[0] + 4) >> 3
        else:
            b[0] = (2 * q[1] + q[0] + p[1] + 2) >> 2
    else:
        tc0 = (TC1[idx] if strength == 1 else TC2[idx] if strength == 2 else TC3[idx]) << (depth - 8)
        tc = tc0 + int(ap) + int(aq)
        delta = max(-tc, min(tc, ((q[0] - p[0]) * 4 + p[1] - q[1] + 4) >> 3))
        max_v = (1 << depth) - 1
        a[0] = max(0, min(max_v, p[0] + delta))
        b[0] = max(0, min(max_v, q[0] - delta))
        avg = (p[0] + q[0] + 1) >> 1
        if ap:
            a[1] += max(-tc0, min(tc0, (p[2] + avg - 2 * p[1]) >> 1))
        if aq:
            b[1] += max(-tc0, min(tc0, (q[2] + avg - 2 * q[1]) >> 1))
    return a, b


def deblock_plane_32x16(plane, depth, deblock_idc, is_intra):
    if deblock_idc == 1:
        return plane
    out = list(plane)
    # 2 macroblocks: MB 0 (x=0..15) in slice 0, MB 1 (x=16..31) in slice 1.
    # Both have transform8 = True, so only edge 0 and edge 2 (multiples of 8) are filtered!
    for mb in range(2):
        mx = mb * 16
        # Vertical edges (edge 0 at mx, edge 2 at mx + 8)
        for edge in [0, 2]:
            if edge == 0 and mx == 0:
                continue
            if edge == 0 and deblock_idc == 2:
                # Cross-slice boundary between slice 0 and slice 1 disabled when disable_deblocking_filter_idc == 2
                continue
            strength = (4 if edge == 0 else 3) if is_intra else 2
            x = mx + edge * 4
            for y in range(16):
                at = y * 32 + x
                p = [out[at - 1 - i] for i in range(4)]
                q = [out[at + i] for i in range(4)]
                np, nq = filter_edge_sample(p, q, 26, depth, strength)
                for i in range(3):
                    out[at - 1 - i] = np[i]
                    out[at + i] = nq[i]
        # Horizontal edges (edge 0 at y=0 skipped since 1 MB row; edge 2 at y=8)
        strength = 3 if is_intra else 2
        y = 8
        for x in range(mx, mx + 16):
            at = y * 32 + x
            p = [out[at - (i + 1) * 32] for i in range(4)]
            q = [out[at + i * 32] for i in range(4)]
            np, nq = filter_edge_sample(p, q, 26, depth, strength)
            for i in range(3):
                out[at - (i + 1) * 32] = np[i]
                out[at + i * 32] = nq[i]
    return out


def generate():
    OUT.mkdir(parents=True, exist_ok=True)
    cases = []
    for depth, cabac, scaling_mode, paff, deblock_idc, aso in itertools.product(
        [8, 10, 14], [False, True], ["sps", "pps", "fallback"], [False, True], [0, 1, 2], [False, True]
    ):
        name = f"sep-8x8-{depth}-e{int(cabac)}-{scaling_mode}-{'paff' if paff else 'prog'}-d{deblock_idc}-a{int(aso)}"
        cfg = configuration(depth, cabac, scaling_mode, paff=paff)
        intra_w, inter_w = weights_for_mode(scaling_mode)
        packets = []
        if not paff:
            # Frame 0: IDR I_8x8 (2 slices per plane: MB 0 has sign -1, MB 1 has sign +1)
            f0_nals = []
            f0_planes = []
            for plane in range(3):
                mb0_nal, mb0_samples = encode_i8x8_slice(0, plane, depth, cabac, deblock_idc, -1, intra_w[plane])
                mb1_nal, mb1_samples = encode_i8x8_slice(1, plane, depth, cabac, deblock_idc, 1, intra_w[plane])
                raw_plane = [
                    v
                    for y in range(16)
                    for v in (mb0_samples[y * 16 : (y + 1) * 16] + mb1_samples[y * 16 : (y + 1) * 16])
                ]
                filtered_plane = deblock_plane_32x16(raw_plane, depth, deblock_idc, True)
                f0_planes.append(filtered_plane)
                f0_nals.extend([mb0_nal, mb1_nal])
            if aso:
                f0_nals.reverse()
            f0_pkt = b"".join(len(n).to_bytes(4, "big") + n for n in f0_nals)
            packets.append(f0_pkt)

            # Frame 1: P_8x8 (2 slices per plane: MB 0 has sign +1, MB 1 has sign -1)
            f1_nals = []
            f1_planes = []
            for plane in range(3):
                ref_plane = f0_planes[plane]
                mb0_ref = [ref_plane[y * 32 + x] for y in range(16) for x in range(16)]
                mb1_ref = [ref_plane[y * 32 + 16 + x] for y in range(16) for x in range(16)]
                mb0_nal, mb0_samples = encode_p8x8_slice(0, plane, depth, cabac, deblock_idc, 1, inter_w[plane], mb0_ref)
                mb1_nal, mb1_samples = encode_p8x8_slice(1, plane, depth, cabac, deblock_idc, -1, inter_w[plane], mb1_ref)
                raw_plane = [
                    v
                    for y in range(16)
                    for v in (mb0_samples[y * 16 : (y + 1) * 16] + mb1_samples[y * 16 : (y + 1) * 16])
                ]
                filtered_plane = deblock_plane_32x16(raw_plane, depth, deblock_idc, False)
                f1_planes.append(filtered_plane)
                f1_nals.extend([mb0_nal, mb1_nal])
            if aso:
                f1_nals.reverse()
            f1_pkt = b"".join(len(n).to_bytes(4, "big") + n for n in f1_nals)
            packets.append(f1_pkt)

            golden = sum(f0_planes, []) + sum(f1_planes, [])
            records = [(0, True, f0_pkt), (1, False, f1_pkt)]
            width, height, fps = 32, 16, 25
        else:
            # PAFF mode: 4 field access units -> 2 woven output frames (32x32 each)
            field_planes = [{}, {}]  # [pair_idx][bottom] -> [plane0, plane1, plane2]
            for pair_idx in range(2):
                for field_step, bottom in enumerate([False, True]):
                    au_idx = pair_idx * 2 + field_step
                    nals = []
                    planes = []
                    for plane in range(3):
                        if pair_idx == 0:
                            is_idr = field_step == 0
                            s0 = -1 if not bottom else 1
                            s1 = 1 if not bottom else -1
                            mb0_nal, mb0_samples = encode_i8x8_slice(
                                0, plane, depth, cabac, deblock_idc, s0, intra_w[plane],
                                paff=True, bottom=bottom, frame_num=0, poc_lsb=field_step, is_idr=is_idr,
                            )
                            mb1_nal, mb1_samples = encode_i8x8_slice(
                                1, plane, depth, cabac, deblock_idc, s1, intra_w[plane],
                                paff=True, bottom=bottom, frame_num=0, poc_lsb=field_step, is_idr=is_idr,
                            )
                            raw_plane = [
                                v
                                for y in range(16)
                                for v in (mb0_samples[y * 16 : (y + 1) * 16] + mb1_samples[y * 16 : (y + 1) * 16])
                            ]
                            filtered_plane = deblock_plane_32x16(raw_plane, depth, deblock_idc, True)
                        else:
                            # Reference for P field with num_ref_idx_l0=1 and default list order is the same-parity field from frame 0!
                            ref_plane = field_planes[0][bottom][plane]
                            mb0_ref = [ref_plane[y * 32 + x] for y in range(16) for x in range(16)]
                            mb1_ref = [ref_plane[y * 32 + 16 + x] for y in range(16) for x in range(16)]
                            s0 = 1 if not bottom else -1
                            s1 = -1 if not bottom else 1
                            mb0_nal, mb0_samples = encode_p8x8_slice(
                                0, plane, depth, cabac, deblock_idc, s0, inter_w[plane], mb0_ref,
                                paff=True, bottom=bottom, frame_num=1, poc_lsb=2 + field_step,
                            )
                            mb1_nal, mb1_samples = encode_p8x8_slice(
                                1, plane, depth, cabac, deblock_idc, s1, inter_w[plane], mb1_ref,
                                paff=True, bottom=bottom, frame_num=1, poc_lsb=2 + field_step,
                            )
                            raw_plane = [
                                v
                                for y in range(16)
                                for v in (mb0_samples[y * 16 : (y + 1) * 16] + mb1_samples[y * 16 : (y + 1) * 16])
                            ]
                            filtered_plane = deblock_plane_32x16(raw_plane, depth, deblock_idc, False)
                        planes.append(filtered_plane)
                        nals.extend([mb0_nal, mb1_nal])
                    if aso:
                        nals.reverse()
                    pkt = b"".join(len(n).to_bytes(4, "big") + n for n in nals)
                    packets.append(pkt)
                    field_planes[pair_idx][bottom] = planes
            golden = []
            for pair_idx in range(2):
                for c in range(3):
                    for y in range(16):
                        golden.extend(field_planes[pair_idx][False][c][y * 32 : (y + 1) * 32])
                        golden.extend(field_planes[pair_idx][True][c][y * 32 : (y + 1) * 32])
            records = [(i, i == 0, pkt) for i, pkt in enumerate(packets)]
            width, height, fps = 32, 32, 50

        for idx, pkt in enumerate(packets):
            (OUT / f"{name}-{idx}.packet").write_bytes(pkt)
        (OUT / f"{name}.avcc").write_bytes(cfg)
        (OUT / f"{name}.mp4").write_bytes(mux(cfg, records, width, height, fps))
        (OUT / f"{name}.y16").write_bytes(struct.pack(f"<{len(golden)}H", *golden))
        cases.append(
            dict(
                name=name,
                depth=depth,
                cabac=cabac,
                scaling=scaling_mode,
                paff=paff,
                deblock=deblock_idc,
                aso=aso,
            )
        )
    assert len(cases) == 216
    (OUT / "manifest.json").write_text(json.dumps(cases, indent=2) + "\n")


if __name__ == "__main__":
    generate()
