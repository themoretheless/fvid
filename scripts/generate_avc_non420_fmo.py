#!/usr/bin/env python3
"""Owned non-4:2:0 (monochrome=0, 4:2:2=2) FMO (num_slice_groups_minus1>0) CAVLC fixtures across progressive, PAFF frame, PAFF field, and MBAFF."""
import itertools
import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from avc_fixture_mp4 import mux
from generate_avc_mbaff_direct_samples import Writer

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/avc-non420-fmo"


def map_units_2x2(kind, direction):
    return {
        0: [0, 1, 0, 1],
        1: [0, 1, 1, 0],
        2: [0, 1, 1, 1],
        3: [0, 1, 0, 1] if direction else [1, 1, 0, 0],
        4: [1, 1, 0, 0] if direction else [0, 0, 1, 1],
        5: [1, 0, 1, 0] if direction else [0, 1, 0, 1],
        6: [0, 1, 0, 1],
    }[kind]


def macroblocks_from_map(unit_map, width_mbs, frame_mbs_only, field_pic, mbaff):
    count = len(unit_map) * (1 if (frame_mbs_only or field_pic) else 2)
    out = []
    for i in range(count):
        if frame_mbs_only or field_pic:
            u = i
        elif mbaff:
            u = i // 2
        else:
            u = (i // (2 * width_mbs)) * width_mbs + (i % width_mbs)
        out.append(unit_map[u])
    return out


def configuration(fmt, yd, cd, kind, direction, frame_mbs_only, mbaff, height_map_units=2):
    profile = {0: 100, 2: 122, 3: 244}[fmt]
    b = Writer()
    for v in [profile, 0, 51]:
        b.u(v, 8)
    b.ue(0)  # sps_id = 0
    b.ue(fmt)  # chroma_format_idc
    if fmt == 3:
        b.u(0)  # separate_colour_plane_flag = 0
    b.ue(yd - 8)
    b.ue(cd - 8)
    b.u(0)  # qpprime_y_zero_transform_bypass_flag
    b.u(0)  # seq_scaling_matrix_present_flag
    b.ue(0)  # log2_max_frame_num_minus4 -> 4 bits
    b.ue(0)  # pic_order_cnt_type = 0
    b.ue(0)  # log2_max_pic_order_cnt_lsb_minus4 -> 4 bits
    b.ue(2)  # max_num_ref_frames = 2
    b.u(0)  # gaps_in_frame_num_value_allowed_flag
    b.ue(1)  # pic_width_in_mbs_minus1 = 1 (width = 32)
    b.ue(height_map_units - 1)  # pic_height_in_map_units_minus1
    b.u(int(frame_mbs_only))
    if not frame_mbs_only:
        b.u(int(mbaff))
    b.u(1)  # direct_8x8_inference_flag
    b.u(0)  # frame_cropping_flag
    b.u(0)  # vui_parameters_present_flag
    sps = b.nal(0x67)

    p = Writer()
    p.ue(0)  # pps_id = 0
    p.ue(0)  # sps_id = 0
    p.u(0)  # entropy_coding_mode_flag = 0 (CAVLC)
    p.u(0)  # bottom_field_pic_order_in_frame_present_flag = 0
    p.ue(1)  # num_slice_groups_minus1 = 1 (2 groups)
    p.ue(kind)  # slice_group_map_type
    if kind == 0:
        p.ue(0)  # run_length_minus1[0] = 0 -> 1
        p.ue(0)  # run_length_minus1[1] = 0 -> 1
    elif kind == 2:
        p.ue(0)  # top_left[0] = 0
        p.ue(0)  # bottom_right[0] = 0
    elif kind in [3, 4, 5]:
        p.u(int(direction))
        p.ue(0)  # slice_group_change_rate_minus1 = 0 -> rate = 1
    elif kind == 6:
        unit_map = map_units_2x2(6, False)
        p.ue(len(unit_map) - 1)
        for g in unit_map:
            p.u(g, 1)
    p.ue(0)  # num_ref_idx_l0_default_active_minus1 = 0
    p.ue(0)  # num_ref_idx_l1_default_active_minus1 = 0
    p.u(0)  # weighted_pred_flag = 0
    p.u(0, 2)  # weighted_bipred_idc = 0
    p.se(0)  # pic_init_qp_minus26 = 0
    p.se(0)  # pic_init_qs_minus26 = 0
    p.se(0)  # chroma_qp_index_offset = 0
    p.u(1)  # deblocking_filter_control_present_flag = 1
    p.u(0)  # constrained_intra_pred_flag = 0
    p.u(0)  # redundant_pic_cnt_present_flag = 0
    pps = p.nal(0x68)

    ext = bytes([0xFC | fmt, 0xF8 | (yd - 8), 0xF8 | (cd - 8), 0])
    return (
        bytes([1, profile, 0, 51, 255, 225])
        + len(sps).to_bytes(2, "big")
        + sps
        + bytes([1])
        + len(pps).to_bytes(2, "big")
        + pps
        + ext
    )


def source_planes(fmt, yd, cd, width, height, tag):
    sx = 2 if fmt == 2 else 1
    planes = []
    for p, depth in enumerate([yd, cd, cd]):
        if fmt == 0 and p > 0:
            planes.append([])
            continue
        w = width if p == 0 else width // sx
        maximum = (1 << depth) - 1
        vals = [
            (x * 37 + y * 59 + p * 71 + tag * 23 + (x ^ y) * 7) & maximum
            for y in range(height)
            for x in range(w)
        ]
        vals[:4] = [0, maximum, 1, maximum - 1]
        planes.append(vals)
    return planes


def write_slice_header(b, first_mb, slice_type, frame_num, poc_lsb, kind, deblock, field_pic=None, ref_idc=True):
    b.ue(first_mb)
    b.ue(slice_type)  # 2=I, 0=P, 1=B
    b.ue(0)  # pps_id = 0
    b.u(frame_num, 4)
    if field_pic is not None:
        if field_pic == "frame":
            b.u(0)
        else:
            b.u(1)
            b.u(int(field_pic == "bottom"))
    if frame_num == 0 and slice_type == 2 and field_pic != "bottom":
        b.ue(0)  # idr_pic_id
    b.u(poc_lsb, 4)
    if slice_type == 1:
        b.u(1)  # direct_spatial_mv_pred_flag = 1
    if slice_type in [0, 1]:
        b.u(0)  # num_ref_idx_active_override_flag = 0
        b.u(0)  # ref_pic_list_modification_flag_l0 = 0
        if slice_type == 1:
            b.u(0)  # ref_pic_list_modification_flag_l1 = 0
    if ref_idc:
        if frame_num == 0 and slice_type == 2 and field_pic != "bottom":
            b.u(0)
            b.u(0)
        else:
            b.u(0)
    b.se(0)  # slice_qp_delta = 0
    b.ue(deblock)
    if deblock != 1:
        b.se(0)
        b.se(0)
    if kind in [3, 4, 5]:
        b.u(2, 3)  # slice_group_change_cycle = 2


def encode_frame_or_field_i_packet(
    fmt, yd, cd, kind, direction, deblock, aso, compressed, frame_mbs_only, field_pic, mbaff, frame_num, poc_lsb, width, height, tag
):
    sx = 2 if fmt == 2 else 1
    unit_map = map_units_2x2(kind, direction)
    mb_map = macroblocks_from_map(unit_map, width // 16, frame_mbs_only, field_pic in ("top", "bottom"), mbaff)
    if compressed:
        y_val = 1 << (yd - 1)
        c_val = 1 << (cd - 1)
        planes = [
            [y_val] * (width * height),
            [] if fmt == 0 else [c_val] * ((width // sx) * height),
            [] if fmt == 0 else [c_val] * ((width // sx) * height),
        ]
    else:
        planes = source_planes(fmt, yd, cd, width, height, tag)

    groups = [1, 0] if aso else [0, 1]
    nals = []
    idr = frame_num == 0 and field_pic != "bottom"
    for group in groups:
        addresses = [a for a, g in enumerate(mb_map) if g == group]
        first_mb = addresses[0] // 2 if mbaff else addresses[0]
        b = Writer()
        write_slice_header(
            b,
            first_mb,
            2,
            frame_num,
            poc_lsb,
            kind,
            deblock,
            field_pic=field_pic,
            ref_idc=True,
        )
        for addr in addresses:
            if mbaff and addr % 2 == 0:
                b.u(0)  # mb_field_decoding_flag = 0 (frame pair)
            if compressed:
                b.ue(3)  # I_16x16_2_0_0 (DC, cbp_chroma=0, cbp_luma=0)
                if fmt != 0:
                    b.ue(0)  # intra_chroma_pred_mode = 0 (DC)
                b.se(0)  # mb_qp_delta = 0
                b.u(1, 1)  # coeff_token for DC (TotalCoeffs=0, nC=0 -> '1')
            else:
                b.ue(25)  # I_PCM
                b.align()
                if mbaff:
                    pair = addr // 2
                    parity = addr % 2
                    mx = pair % (width // 16)
                    my = (pair // (width // 16)) * 2 + parity
                else:
                    mx = addr % (width // 16)
                    my = addr // (width // 16)
                for p, depth in enumerate([yd, cd, cd]):
                    if fmt == 0 and p > 0:
                        continue
                    bw = 16 if p == 0 else 16 // sx
                    stride = width if p == 0 else width // sx
                    for row in range(16):
                        for col in range(bw):
                            b.u(planes[p][(my * 16 + row) * stride + mx * bw + col], depth)
        nals.append(b.nal(0x65 if idr else 0x41))
    return b"".join(len(n).to_bytes(4, "big") + n for n in nals), planes


def encode_inter_skip_packet(
    kind, direction, deblock, aso, slice_type, frame_mbs_only, field_pic, mbaff, frame_num, poc_lsb, width
):
    unit_map = map_units_2x2(kind, direction)
    mb_map = macroblocks_from_map(unit_map, width // 16, frame_mbs_only, field_pic in ("top", "bottom"), mbaff)
    groups = [1, 0] if aso else [0, 1]
    nals = []
    ref_idc = slice_type == 0
    for group in groups:
        addresses = [a for a, g in enumerate(mb_map) if g == group]
        first_mb = addresses[0] // 2 if mbaff else addresses[0]
        b = Writer()
        write_slice_header(
            b,
            first_mb,
            slice_type,
            frame_num,
            poc_lsb,
            kind,
            deblock,
            field_pic=field_pic,
            ref_idc=ref_idc,
        )
        b.ue(len(addresses))  # mb_skip_run = all macroblocks in this slice group
        nals.append(b.nal(0x41 if ref_idc else 0x01))
    return b"".join(len(n).to_bytes(4, "big") + n for n in nals)


def weave_fields(top_planes, bottom_planes, fmt):
    sx = 2 if fmt == 2 else 1
    out = []
    for p in range(3):
        if fmt == 0 and p > 0:
            out.append([])
            continue
        w = 32 if p == 0 else 32 // sx
        plane = []
        for row in range(32):
            plane.extend(top_planes[p][row * w : (row + 1) * w])
            plane.extend(bottom_planes[p][row * w : (row + 1) * w])
        out.append(plane)
    return out


def generate():
    OUT.mkdir(parents=True, exist_ok=True)
    cases = []
    golden = bytearray()

    def record_samples(planes):
        at = len(golden) // 2
        flat = sum(planes, [])
        golden.extend(struct.pack("<" + str(len(flat)) + "H", *flat))
        return at

    # 1. Progressive FMO I/P/B across fmt in [0, 2], depths in [(8,8), (10,12)], all 10 FMO map types/directions,
    #    compressed in [False, True], deblock in [1, 0, 2], aso in [False, True]
    for fmt, (yd, cd), kind in itertools.product([0, 2], [(8, 8), (10, 12)], range(7)):
        for direction in [False, True] if kind in [3, 4, 5] else [False]:
            config = configuration(fmt, yd, cd, kind, direction, True, False, 2)
            for compressed, deblock, aso in itertools.product([False, True], [1, 0, 2], [False, True]):
                if kind in [3, 4, 5] and (compressed or deblock == 2):
                    continue
                name = f"prog-fmo-f{fmt}-y{yd}-c{cd}-t{kind}-d{int(direction)}-comp{int(compressed)}-deb{deblock}-aso{int(aso)}"
                i_pkt, planes = encode_frame_or_field_i_packet(
                    fmt, yd, cd, kind, direction, deblock, aso, compressed, True, None, False, 0, 0, 32, 32, kind + deblock
                )
                p_pkt = encode_inter_skip_packet(kind, direction, deblock, aso, 0, True, None, False, 1, 4, 32)
                b_pkt = encode_inter_skip_packet(kind, direction, deblock, aso, 1, True, None, False, 2, 2, 32)
                at = record_samples(planes)
                (OUT / f"{name}.avcc").write_bytes(config)
                (OUT / f"{name}.mp4").write_bytes(mux(config, [(0, True, i_pkt), (2, False, p_pkt), (1, False, b_pkt)], 32, 32, 25))
                cases.append(dict(
                    name=name,
                    mode="progressive",
                    format=fmt,
                    luma_depth=yd,
                    chroma_depth=cd,
                    width=32,
                    height=32,
                    offset=at,
                ))

    # 2. PAFF Frame FMO I/P/B across fmt in [0, 2], depths in [(8,8), (10,12)], kind in [0, 1, 2, 3, 6],
    #    compressed in [False, True], deblock in [1, 0], aso in [False, True]
    for fmt, (yd, cd), kind, compressed, deblock, aso in itertools.product(
        [0, 2], [(8, 8), (10, 12)], [0, 1, 2, 3, 6], [False, True], [1, 0], [False, True]
    ):
        config = configuration(fmt, yd, cd, kind, False, False, False, 2)
        name = f"paff-frame-fmo-f{fmt}-y{yd}-c{cd}-t{kind}-comp{int(compressed)}-deb{deblock}-aso{int(aso)}"
        i_pkt, planes = encode_frame_or_field_i_packet(
            fmt, yd, cd, kind, False, deblock, aso, compressed, False, "frame", False, 0, 0, 32, 64, kind + 10
        )
        p_pkt = encode_inter_skip_packet(kind, False, deblock, aso, 0, False, "frame", False, 1, 4, 32)
        b_pkt = encode_inter_skip_packet(kind, False, deblock, aso, 1, False, "frame", False, 2, 2, 32)
        at = record_samples(planes)
        (OUT / f"{name}.avcc").write_bytes(config)
        (OUT / f"{name}.mp4").write_bytes(mux(config, [(0, True, i_pkt), (2, False, p_pkt), (1, False, b_pkt)], 32, 64, 25))
        cases.append(dict(
            name=name,
            mode="paff_frame",
            format=fmt,
            luma_depth=yd,
            chroma_depth=cd,
            width=32,
            height=64,
            offset=at,
        ))

    # 3. PAFF Field FMO I/P/B across fmt in [0, 2], depths in [(8,8), (10,12)], kind in [0, 1, 2, 3, 6],
    #    compressed in [False, True], deblock in [1, 0], aso in [False, True]
    for fmt, (yd, cd), kind, compressed, deblock, aso in itertools.product(
        [0, 2], [(8, 8), (10, 12)], [0, 1, 2, 3, 6], [False, True], [1, 0], [False, True]
    ):
        config = configuration(fmt, yd, cd, kind, False, False, False, 2)
        name = f"paff-field-fmo-f{fmt}-y{yd}-c{cd}-t{kind}-comp{int(compressed)}-deb{deblock}-aso{int(aso)}"
        i_top, top_planes = encode_frame_or_field_i_packet(
            fmt, yd, cd, kind, False, deblock, aso, compressed, False, "top", False, 0, 0, 32, 32, kind + 20
        )
        i_bot, bot_planes = encode_frame_or_field_i_packet(
            fmt, yd, cd, kind, False, deblock, aso, compressed, False, "bottom", False, 0, 1, 32, 32, kind + 21
        )
        p_top = encode_inter_skip_packet(kind, False, deblock, aso, 0, False, "top", False, 1, 4, 32)
        p_bot = encode_inter_skip_packet(kind, False, deblock, aso, 0, False, "bottom", False, 1, 5, 32)
        b_top = encode_inter_skip_packet(kind, False, deblock, aso, 1, False, "top", False, 2, 2, 32)
        b_bot = encode_inter_skip_packet(kind, False, deblock, aso, 1, False, "bottom", False, 2, 3, 32)
        woven = weave_fields(top_planes, bot_planes, fmt)
        at = record_samples(woven)
        (OUT / f"{name}.avcc").write_bytes(config)
        (OUT / f"{name}.mp4").write_bytes(mux(
            config,
            [(0, True, i_top), (1, False, i_bot), (4, False, p_top), (5, False, p_bot), (2, False, b_top), (3, False, b_bot)],
            32,
            64,
            50,
        ))
        cases.append(dict(
            name=name,
            mode="paff_field",
            format=fmt,
            luma_depth=yd,
            chroma_depth=cd,
            width=32,
            height=64,
            offset=at,
        ))

    # 4. MBAFF FMO I/P/B across fmt in [0, 2], depths in [(8,8), (10,12)], kind in [0, 1, 2, 3, 6],
    #    compressed in [False, True], deblock in [1, 0], aso in [False, True]
    for fmt, (yd, cd), kind, compressed, deblock, aso in itertools.product(
        [0, 2], [(8, 8), (10, 12)], [0, 1, 2, 3, 6], [False, True], [1, 0], [False, True]
    ):
        config = configuration(fmt, yd, cd, kind, False, False, True, 2)
        name = f"mbaff-fmo-f{fmt}-y{yd}-c{cd}-t{kind}-comp{int(compressed)}-deb{deblock}-aso{int(aso)}"
        i_pkt, planes = encode_frame_or_field_i_packet(
            fmt, yd, cd, kind, False, deblock, aso, compressed, False, "frame", True, 0, 0, 32, 64, kind + 30
        )
        p_pkt = encode_inter_skip_packet(kind, False, deblock, aso, 0, False, "frame", True, 1, 4, 32)
        b_pkt = encode_inter_skip_packet(kind, False, deblock, aso, 1, False, "frame", True, 2, 2, 32)
        at = record_samples(planes)
        (OUT / f"{name}.avcc").write_bytes(config)
        (OUT / f"{name}.mp4").write_bytes(mux(config, [(0, True, i_pkt), (2, False, p_pkt), (1, False, b_pkt)], 32, 64, 25))
        cases.append(dict(
            name=name,
            mode="mbaff",
            format=fmt,
            luma_depth=yd,
            chroma_depth=cd,
            width=32,
            height=64,
            offset=at,
        ))

    (OUT / "samples.y16").write_bytes(golden)
    (OUT / "manifest.json").write_text(json.dumps(dict(
        provenance="Owned non-4:2:0 (monochrome=0, 4:2:2=2) FMO CAVLC I/P/B fixtures across progressive, PAFF frame, PAFF field, and MBAFF. No external codec or private media.",
        cases=cases,
    ), indent=2) + "\n")
    print(f"Generated {len(cases)} cases")


if __name__ == "__main__":
    generate()
