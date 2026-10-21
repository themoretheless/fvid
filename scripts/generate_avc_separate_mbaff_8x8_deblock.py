#!/usr/bin/env python3
"""Own separate-colour-plane MBAFF 8x8 transform, 12-list 8x8 scaling, ASO and deblocking fixtures."""
import itertools, json, struct
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_separate_8x8_deblock import (
    residual_8x8_single,
    write_constant_list,
    weights_for_mode,
    filter_edge_sample,
)
from avc_fixture_mp4 import mux

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/avc-separate-mbaff-8x8-deblock"


def configuration(depth, cabac, scaling_mode):
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
    b.ue(1)  # pic_height_in_map_units_minus1 (2 map units -> 64 px MBAFF frame)
    b.u(0)  # frame_mbs_only_flag = 0
    b.u(1)  # mb_adaptive_frame_field_flag = 1
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


def write_mbaff_slice_header(b, first_pair, slice_type, plane, frame_num, poc_lsb, is_idr, deblock_idc, cabac):
    b.ue(first_pair)
    b.ue(slice_type)
    b.ue(0)  # pic_parameter_set_id
    b.u(plane, 2)
    b.u(frame_num, 4)
    b.u(0)  # field_pic_flag = 0
    if is_idr:
        b.ue(0)  # idr_pic_id
    b.u(poc_lsb, 4)
    if slice_type == 0:  # P slice
        b.u(0)  # num_ref_idx_active_override_flag
        b.u(0)  # ref_pic_list_modification_flag_l0
    if is_idr:
        b.u(0)  # no_output_of_prior_pics_flag
        b.u(0)  # long_term_reference_flag
    else:
        b.u(0)  # adaptive_ref_pic_marking_mode_flag
    if cabac and slice_type != 2:
        b.ue(0)  # cabac_init_idc
    b.se(0)  # slice_qp_delta
    b.ue(deblock_idc)
    if deblock_idc != 1:
        b.se(0)
        b.se(0)


def reconstruct_i8x8_mb(depth, sign, intra_weight, top_avail_val=None):
    res = residual_8x8_single(0, 4 * sign, 26, depth, intra_weight)
    max_v = (1 << depth) - 1
    mb_samples = [0] * 256
    for block in range(4):
        bx = (block % 2) * 8
        by = (block // 2) * 8
        if block == 0:
            if top_avail_val is not None:
                # Top is available (constant top_avail_val across all 16 columns, so t[0..15] = top_avail_val -> filtered t'[0..7] = top_avail_val)
                pred = top_avail_val
            else:
                pred = 1 << (depth - 1)
        elif block == 1:
            if top_avail_val is not None:
                # Top (t[0..7] = top_avail_val) and corner (t[-1] = top_avail_val) and left (l[0..7] = b0_val) are available!
                # t'[0..7] = top_avail_val; l'[0] = (top_avail_val + 3 * b0_val + 2) >> 2, l'[1..7] = b0_val
                l0 = (top_avail_val + 3 * b0_val + 2) >> 2
                pred = (8 * top_avail_val + l0 + 7 * b0_val + 8) // 16
            else:
                pred = b0_val
        elif block == 2:
            t7 = (3 * b0_val + b1_val + 2) >> 2
            pred = (7 * b0_val + t7 + 4) // 8
        else:
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
    return mb_samples, val


def encode_mbaff_i8x8_pair(pair, field, plane, depth, cabac, deblock_idc, intra_weight):
    b = Writer()
    write_mbaff_slice_header(b, pair, 2, plane, 0, 0, True, deblock_idc, cabac)
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(-1, 26)
        c.decision(70, int(field))  # mb_field_decoding_flag (left/top pair not in slice -> ctx 70)
    else:
        b.u(int(field))

    pair_mbs = []
    top_bottom_row_val = None
    for parity in range(2):
        sign = 1 if (pair + parity + plane) % 2 == 0 else -1
        # In a frame-coded MBAFF pair, the bottom MB (parity=1) has top neighbour = top MB (parity=0)!
        # In a field-coded MBAFF pair, parity=1 is the bottom field MB, whose top neighbour is in pair-2 (another slice -> unavailable)!
        has_top = (parity == 1) and (not field)
        if cabac:
            if parity == 1:
                c.range -= 2
                c.renormalize()
            c.decision(3, 0)  # mb_type = I_NxN (top MB is also I_NxN=0 so condTermFlagB=0 -> ctx 3)
            c.decision(399 + int(has_top), 1)  # transform_size_8x8_flag = 1 (top MB has eight=1 when has_top!)
            for _ in range(4):
                c.decision(68, 1)  # prev_intra8x8_pred_mode_flag = 1 -> DC
            for _ in range(4):
                c.decision(73, 1)  # coded_block_pattern luma = 15 (top MB has pattern=15 so bit=1 -> inc=0 -> ctx 73)
            c.decision(60, 0)  # mb_qp_delta = 0
            sig_ctx = 436 if field else 402
            last_ctx = 451 if field else 417
            for _ in range(4):
                c.decision(sig_ctx, 1)
                c.decision(last_ctx, 1)
                c.decision(427, 1)
                c.decision(431, 1)
                c.decision(431, 1)
                c.decision(431, 0)
                c.bypass(int(sign < 0))
        else:
            b.ue(0)  # mb_type = I_NxN
            b.u(1)  # transform_size_8x8_flag = 1
            for _ in range(4):
                b.u(1)
            b.ue(0)  # coded_block_pattern = 15
            b.se(0)  # mb_qp_delta = 0
            # Note on CAVLC nC in 8x8:
            # For sub % 4 == 0 (the first 4x4 subblock of each 8x8 block: sub 0, 4, 8, 12 -> (bx, by) = (0,0), (2,0), (0,2), (2,2)):
            # When has_top=False: all have nC=0 (since their left/top 4x4 blocks are subblocks 1,2,3 which have count=0!).
            # When has_top=True: for sub 0 (bx=0, by=0), its top neighbour is (bx=0, by=3) of parity=0, which is subblock 10 (by=2+1=3, bx=0) -> count=0!
            # For sub 4 (bx=2, by=0), its top neighbour is (bx=2, by=3) of parity=0, which is subblock 14 -> count=0!
            # So nC is ALWAYS 0 for sub % 4 == 0, and <= 1 (VLC0) for all other subblocks!
            for sub in range(16):
                if sub % 4 == 0:
                    b.u(5, 6)  # TotalCoeff=1, TrailingOnes=0 at nC=0
                    b.u(1, 6 if sign < 0 else 5)  # level -4 or +4
                    b.u(1)  # total_zeros = 0
                else:
                    b.u(1)  # TotalCoeff=0 at VLC0
        mb_samples, bottom_val = reconstruct_i8x8_mb(
            depth, sign, intra_weight, top_avail_val=top_bottom_row_val if has_top else None
        )
        # Note: in parity=0, blocks 2 and 3 both have the same reconstructed constant value when top_avail_val is None?
        # Wait: let's check if block 2 and block 3 have the SAME value in parity=0!
        # Wait: in parity=0, block 2 has pred = (7*b0 + t7 + 4)//8, whereas block 3 has pred = (t0 + 7*b1 + l0 + 7*b2 + 8)//16.
        # Could block 2 and block 3 differ slightly? Let's pass the exact 16-sample bottom row of parity=0!
        pair_mbs.append(mb_samples)
        top_bottom_row_val = mb_samples[15 * 16 : 16 * 16]
    if cabac:
        b.bits.extend(c.finish())
    nal = b.nal(0x65, trailing=not cabac)
    return nal, pair_mbs


def reconstruct_i8x8_mb_exact(depth, sign, intra_weight, top_row=None):
    res = residual_8x8_single(0, 4 * sign, 26, depth, intra_weight)
    max_v = (1 << depth) - 1
    mb_samples = [0] * 256
    for block in range(4):
        bx = (block % 2) * 8
        by = (block // 2) * 8
        if block == 0:
            if top_row is not None:
                # Top is top_row[0..8], top-right is top_row[8..16], corner is unavailable.
                # H.264 8.3.2.2.2 reference filtering when top is available, top-right is available, left/corner unavailable:
                # t'[0] = (3 * t[0] + t[1] + 2) >> 2
                # t'[x] = (t[x-1] + 2*t[x] + t[x+1] + 2) >> 2 for x=1..7 (using t[8] = top_row[8] for x=7)
                t_filt = [0] * 8
                t_filt[0] = (3 * top_row[0] + top_row[1] + 2) >> 2
                for x in range(1, 8):
                    t_filt[x] = (top_row[x - 1] + 2 * top_row[x] + top_row[x + 1] + 2) >> 2
                pred = (sum(t_filt) + 4) >> 3
            else:
                pred = 1 << (depth - 1)
        elif block == 1:
            if top_row is not None:
                # Top is top_row[8..16], corner is top_row[7], left is block 0 col 7 (all = b0_val), top-right unavailable.
                # t'[0] = (top_row[7] + 2*top_row[8] + top_row[9] + 2) >> 2
                # t'[x] = (top_row[8+x-1] + 2*top_row[8+x] + top_row[8+x+1] + 2) >> 2 for x=1..6
                # t'[7] = (top_row[14] + 3*top_row[15] + 2) >> 2
                # l'[0] = (top_row[7] + 3*b0_val + 2) >> 2, l'[1..7] = b0_val
                t_filt = [0] * 8
                t_filt[0] = (top_row[7] + 2 * top_row[8] + top_row[9] + 2) >> 2
                for x in range(1, 7):
                    t_filt[x] = (top_row[8 + x - 1] + 2 * top_row[8 + x] + top_row[8 + x + 1] + 2) >> 2
                t_filt[7] = (top_row[14] + 3 * top_row[15] + 2) >> 2
                l0 = (top_row[7] + 3 * b0_val + 2) >> 2
                pred = (sum(t_filt) + l0 + 7 * b0_val + 8) >> 4
            else:
                pred = b0_val
        elif block == 2:
            t7 = (3 * b0_val + b1_val + 2) >> 2
            pred = (7 * b0_val + t7 + 4) >> 3
        else:
            t0 = (b0_val + 3 * b1_val + 2) >> 2
            l0 = (b0_val + 3 * b2_val + 2) >> 2
            pred = (t0 + 7 * b1_val + l0 + 7 * b2_val + 8) >> 4
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
    return mb_samples


def encode_mbaff_i8x8_pair_exact(pair, field, plane, depth, cabac, deblock_idc, intra_weight):
    b = Writer()
    write_mbaff_slice_header(b, pair, 2, plane, 0, 0, True, deblock_idc, cabac)
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(-1, 26)
        c.decision(70, int(field))
    else:
        b.u(int(field))

    pair_mbs = []
    top_row = None
    for parity in range(2):
        sign = 1 if (pair + parity + plane) % 2 == 0 else -1
        has_top = (parity == 1) and (not field)
        if cabac:
            c.decision(3, 0)
            c.decision(399 + int(has_top), 1)
            for _ in range(4):
                c.decision(68, 1)
            for _ in range(4):
                c.decision(73, 1)
            c.decision(60, 0)
            sig_ctx = 436 if field else 402
            last_ctx = 451 if field else 417
            for _ in range(4):
                c.decision(sig_ctx, 1)
                c.decision(last_ctx, 1)
                c.decision(427, 1)
                c.decision(431, 1)
                c.decision(431, 1)
                c.decision(431, 0)
                c.bypass(int(sign < 0))
        else:
            b.ue(0)
            b.u(1)
            for _ in range(4):
                b.u(1)
            b.ue(0)
            b.se(0)
            for sub in range(16):
                if sub % 4 == 0:
                    b.u(5, 6)
                    b.u(1, 6 if sign < 0 else 5)
                    b.u(1)
                else:
                    b.u(1)
        mb_samples = reconstruct_i8x8_mb_exact(depth, sign, intra_weight, top_row=top_row if has_top else None)
        pair_mbs.append(mb_samples)
        top_row = mb_samples[15 * 16 : 16 * 16]
    if cabac:
        b.bits.extend(c.finish())
    nal = b.nal(0x65, trailing=not cabac)
    return nal, pair_mbs


def encode_mbaff_p8x8_pair(pair, field, plane, depth, cabac, deblock_idc, inter_weight, ref_plane):
    b = Writer()
    write_mbaff_slice_header(b, pair, 0, plane, 1, 2, False, deblock_idc, cabac)
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(0, 26)
        c.decision(11, 0)  # mb_skip_flag = 0 for parity 0
        c.decision(70, int(field))  # mb_field_decoding_flag
    else:
        b.ue(0)  # mb_skip_run = 0 before parity 0
        b.u(int(field))

    pair_mbs = []
    raster_pos = 8 if field else 1
    for parity in range(2):
        sign = -1 if (pair + parity + plane) % 2 == 0 else 1
        has_top = (parity == 1) and (not field)
        if cabac:
            if parity == 1:
                c.decision(11 + int(has_top), 0)  # mb_skip_flag = 0 (top MB was not skipped when has_top!)
            c.decision(14, 0)  # P_L0_16x16
            c.decision(15, 0)
            c.decision(16, 0)
            if field:
                c.decision(54, 0)  # ref_idx_l0 = 0 (active = 2 * 1 = 2 in field-coded MBAFF macroblock)
            c.mvd(0, 0, 0)
            c.mvd(1, 0, 0)
            for _ in range(4):
                c.decision(73, 1)  # coded_block_pattern luma = 15
            c.decision(399 + int(has_top), 1)  # transform_size_8x8_flag = 1
            c.decision(60, 0)  # mb_qp_delta = 0
            sig_base = 436 if field else 402
            last_base = 451 if field else 417
            for _ in range(4):
                c.decision(sig_base + 0, 0)
                c.decision(sig_base + 1, 1)
                c.decision(last_base + 1, 1)
                c.decision(427, 1)
                c.decision(431, 0)
                c.bypass(int(sign < 0))
        else:
            if parity == 1:
                b.ue(0)  # mb_skip_run = 0 before parity 1
            b.ue(0)  # mb_type = P_L0_16x16
            if field:
                b.u(1)  # te(1) for ref_idx_l0 = 0 (active = 2 -> !bit -> bit = 1 for index 0)
            b.se(0)  # mvd_l0 x = 0
            b.se(0)  # mvd_l0 y = 0
            b.ue(9)  # coded_block_pattern = 15 (inter monochrome)
            b.u(1)  # transform_size_8x8_flag = 1
            b.se(0)  # mb_qp_delta = 0
            for _ in range(4):
                for stream in range(4):
                    if stream == 1:
                        b.u(5, 6)
                        b.u(1, 2 if sign < 0 else 1)
                        b.u(1)
                    else:
                        b.u(1)
        res8 = residual_8x8_single(raster_pos, 2 * sign, 26, depth, inter_weight)
        max_v = (1 << depth) - 1
        ox = (pair % 2) * 16
        oy = (pair // 2) * 32 + (parity if field else parity * 16)
        step = 2 if field else 1
        mb_samples = [0] * 256
        for by in range(2):
            for bx in range(2):
                for y in range(8):
                    for x in range(8):
                        r = by * 8 + y
                        col = bx * 8 + x
                        base = ref_plane[(oy + r * step) * 32 + ox + col]
                        mb_samples[r * 16 + col] = max(0, min(max_v, base + res8[y * 8 + x]))
        pair_mbs.append(mb_samples)
    if cabac:
        b.bits.extend(c.finish())
    nal = b.nal(0x41, trailing=not cabac)
    return nal, pair_mbs


def scatter_mbaff_plane(pair_mbs_list, topology):
    plane = [0] * 2048
    for pair, field in enumerate(MODES[topology]):
        for parity in range(2):
            ox = (pair % 2) * 16
            oy = (pair // 2) * 32 + (parity if field else parity * 16)
            step = 2 if field else 1
            mb = pair_mbs_list[pair][parity]
            for r in range(16):
                for c in range(16):
                    plane[(oy + r * step) * 32 + ox + c] = mb[r * 16 + c]
    return plane


def deblock_mbaff_plane(source, depth, topology, deblock_idc, is_intra, transform8=True, qp=26, active=True, edge_strength=None):
    out = list(source)
    if deblock_idc == 1:
        return out
    owners = [None] * 2048
    locations = []
    for pair, field in enumerate(MODES[topology]):
        for parity in range(2):
            ox = (pair % 2) * 16
            oy = (pair // 2) * 32 + (parity if field else parity * 16)
            step = 2 if field else 1
            rows = [oy + r * step for r in range(16)]
            locations.append((ox, rows, field, pair))
            for r in rows:
                for c in range(ox, ox + 16):
                    owners[r * 32 + c] = len(locations) - 1
    for address, (ox, rows, field, pair) in enumerate(locations):
        for vertical in [True, False]:
            # An 8x8 transform skips odd 4x4 edges; the shared author also covers 4x4 grids.
            for edge in ([0, 8] if transform8 else [0, 4, 8, 12]):
                if edge == 0 and (ox == 0 if vertical else rows[0] < (2 if field else 1)):
                    continue
                mixed_top = not vertical and edge == 0 and not field and locations[owners[(rows[0] - 1) * 32 + ox]][2]
                for extra in range(2 if mixed_top else 1):
                    for line in range(16):
                        x = ox + (edge if vertical else line)
                        y = rows[line] if vertical else rows[edge] + extra
                        step = 1 if vertical else 32 * (2 if field or mixed_top else 1)
                        at = y * 32 + x
                        neighbour = owners[at - step]
                        if deblock_idc == 2 and locations[neighbour][3] != pair:
                            continue
                        if edge_strength is not None:
                            py,px=divmod(at-step,32);nx,nrows,nfield,_=locations[neighbour]
                            strength=edge_strength(neighbour,(px-nx,(py-nrows[0])//(2 if nfield else 1)),address,(x-ox,(y-rows[0])//(2 if field else 1)))
                        elif is_intra:
                            strength = (
                                4
                                if edge == 0 and (vertical or (not field and not locations[neighbour][2]))
                                else 3
                            )
                        else:
                            # Active planes use bS=2; zero planes retain mixed frame/field bS=1.
                            strength = 2 if active else int(field != locations[neighbour][2])
                        if strength == 0:
                            continue
                        p = [out[at - (i + 1) * step] for i in range(4)]
                        q = [out[at + i * step] for i in range(4)]
                        np, nq = filter_edge_sample(p, q, qp, depth, strength)
                        for i in range(3):
                            out[at - (i + 1) * step] = np[i]
                            out[at + i * step] = nq[i]
    return out


def generate():
    OUT.mkdir(parents=True, exist_ok=True)
    cases = []
    layouts = list(MODES)
    for depth, cabac, scaling_mode, base, deblock_idc, aso in itertools.product(
        [8, 10, 14], [False, True], ["sps", "pps", "fallback"], range(4), [0, 1, 2], [False, True]
    ):
        name = f"sep-mbaff-8x8-{depth}-e{int(cabac)}-{scaling_mode}-t{base}-d{deblock_idc}-a{int(aso)}"
        cfg = configuration(depth, cabac, scaling_mode)
        intra_weights, inter_weights = weights_for_mode(scaling_mode)
        chosen = [layouts[(base + c) % 4] for c in range(3)]

        i_nals = []
        p_nals = []
        i_planes = []
        p_planes = []
        for c, topology in enumerate(chosen):
            iw = intra_weights[c]
            pw = inter_weights[c]
            plane_i_nals = []
            plane_i_mbs = []
            for pair, field in enumerate(MODES[topology]):
                nal, pair_mbs = encode_mbaff_i8x8_pair_exact(pair, field, c, depth, cabac, deblock_idc, iw)
                plane_i_nals.append(nal)
                plane_i_mbs.append(pair_mbs)
            if aso:
                plane_i_nals.reverse()
            i_nals.extend(plane_i_nals)
            i_plane = deblock_mbaff_plane(scatter_mbaff_plane(plane_i_mbs, topology), depth, topology, deblock_idc, True)
            i_planes.append(i_plane)

            plane_p_nals = []
            plane_p_mbs = []
            for pair, field in enumerate(MODES[topology]):
                nal, pair_mbs = encode_mbaff_p8x8_pair(pair, field, c, depth, cabac, deblock_idc, pw, i_plane)
                plane_p_nals.append(nal)
                plane_p_mbs.append(pair_mbs)
            if aso:
                plane_p_nals.reverse()
            p_nals.extend(plane_p_nals)
            p_plane = deblock_mbaff_plane(scatter_mbaff_plane(plane_p_mbs, topology), depth, topology, deblock_idc, False)
            p_planes.append(p_plane)

        pkt0 = b"".join(len(n).to_bytes(4, "big") + n for n in i_nals)
        pkt1 = b"".join(len(n).to_bytes(4, "big") + n for n in p_nals)
        (OUT / f"{name}-0.packet").write_bytes(pkt0)
        (OUT / f"{name}-1.packet").write_bytes(pkt1)
        golden = sum(i_planes, []) + sum(p_planes, [])
        assert len(golden) == 12288
        for ext, data in [
            ("avcc", cfg),
            ("mp4", mux(cfg, [(0, True, pkt0), (1, False, pkt1)], 32, 64, 25)),
            ("y16", struct.pack("<12288H", *golden)),
        ]:
            (OUT / f"{name}.{ext}").write_bytes(data)
        cases.append(
            dict(
                name=name,
                depth=depth,
                cabac=cabac,
                scaling=scaling_mode,
                layouts=chosen,
                deblock=deblock_idc,
                aso=aso,
            )
        )
    assert len(cases) == 432
    (OUT / "manifest.json").write_text(json.dumps(cases, indent=2) + "\n")
    print(f"Generated {len(cases)} fixtures in {OUT}")


if __name__ == "__main__":
    generate()
