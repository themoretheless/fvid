"""Owned non-4:2:0 PAFF frame and filtered I_PCM fixtures across monochrome/4:2:2/4:4:4."""
import itertools
import json
import struct
from pathlib import Path
from generate_avc_component_fields import configuration as interlaced_configuration
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_pcm_formats import Writer, configuration as prog_configuration, mux

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/avc-non420-paff-filtered-pcm'
MODES = {
    'frame': [False] * 4,
    'field': [True] * 4,
    'columns': [False, True, False, True],
    'checker': [False, True, True, False],
}


def write_slice_header(b, first_mb, index, field_pic=None, deblock_idc=0, offsets=(-2, 1)):
    b.ue(first_mb)
    b.ue(2)
    b.ue(0)
    b.u(0 if field_pic in ('top', 'bottom') else index, 4)
    if field_pic is not None:
        if field_pic == 'frame':
            b.u(0)
        else:
            b.u(1)
            b.u(int(field_pic == 'bottom'))
    if index == 0:
        b.ue(0)
    b.u(index if field_pic in ('top', 'bottom') else index * 2, 4)
    if index == 0:
        b.u(0)
        b.u(0)
    else:
        b.u(0)
    b.se(0)
    b.ue(deblock_idc)
    if deblock_idc != 1:
        b.se(offsets[0])
        b.se(offsets[1])


def sample_planes(fmt, yd, cd, width, height, index):
    sx = 2 if fmt == 2 else 1
    planes = []
    for p, depth in enumerate([yd, cd, cd]):
        if fmt == 0 and p > 0:
            planes.append([])
            continue
        w = width if p == 0 else width // sx
        h = height
        maximum = (1 << depth) - 1
        values = [(x * 31 + y * 53 + p * 43 + index * 17) & maximum for y in range(h) for x in range(w)]
        values[:4] = [0, maximum, 1, maximum - 1]
        planes.append(values)
    return planes


def progressive_or_paff_frame_pcm_packet(fmt, yd, cd, cabac, height, index, paff_frame, deblock_idc, aso):
    sx = 2 if fmt == 2 else 1
    planes = sample_planes(fmt, yd, cd, 32, height, index)
    mb_count = (32 // 16) * (height // 16)
    slices = [[mb] for mb in range(mb_count)] if aso else [list(range(mb_count))]
    nals = []
    for addresses in slices:
        b = Writer()
        write_slice_header(
            b,
            addresses[0],
            index,
            field_pic='frame' if paff_frame else None,
            deblock_idc=deblock_idc,
        )
        if cabac:
            while len(b.bits) % 8:
                b.u(1)
            c = CabacWriter(-1, 26)
        start = addresses[0]
        for pos, mb in enumerate(addresses):
            if cabac:
                if pos > 0:
                    c.range -= 2
                    c.renormalize()
                left = (mb % 2 != 0) and (mb > start)
                top = (mb >= 2) and (mb - 2 >= start)
                c.decision(3 + int(left) + int(top), 1)
                b.bits.extend(c.finish())
                b.align()
            else:
                b.ue(25)
                b.align()
            mx = mb % 2
            my = mb // 2
            for p, depth in enumerate([yd, cd, cd]):
                if fmt == 0 and p > 0:
                    continue
                bw = 16 if p == 0 else 16 // sx
                stride = 32 if p == 0 else 32 // sx
                for row in range(16):
                    for col in range(bw):
                        b.u(planes[p][(my * 16 + row) * stride + mx * bw + col], depth)
            if cabac:
                c.low = 0
                c.range = 510
                c.width = 9
        if cabac:
            b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index == 0 else 0x41, trailing=not cabac))
    if aso:
        nals.reverse()
    return b''.join(len(n).to_bytes(4, 'big') + n for n in nals), sum(planes, [])


def paff_frame_compressed_packet(fmt, yd, cd, cabac, index, deblock_idc):
    sx = 2 if fmt == 2 else 1
    b = Writer()
    write_slice_header(b, 0, index, field_pic='frame', deblock_idc=deblock_idc)
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(-1, 26)
    for mb in range(4):
        if cabac:
            if mb > 0:
                c.range -= 2
                c.renormalize()
            left = mb % 2 != 0
            top = mb >= 2
            c.decision(3 + int(left) + int(top), 1)
            c.range -= 2
            c.renormalize()
            for ctx, v in [(6, 0), (7, 0), (9, 1), (10, 0)]:
                c.decision(ctx, v)
            if fmt != 0:
                c.decision(64, 0)
            c.decision(60, 0)
            c.decision(85 + (0 if left else 1) + (0 if top else 2), 0)
        else:
            b.ue(3)
            if fmt != 0:
                b.ue(0)
            b.se(0)
            b.u(1)
    if cabac:
        b.bits.extend(c.finish())
    nal = b.nal(0x65 if index == 0 else 0x41, trailing=not cabac)
    y_val = 1 << (yd - 1)
    c_val = 1 << (cd - 1)
    planes = [[y_val] * (32 * 32)]
    if fmt == 0:
        planes.extend([[], []])
    else:
        planes.extend([[c_val] * ((32 // sx) * 32), [c_val] * ((32 // sx) * 32)])
    return len(nal).to_bytes(4, 'big') + nal, sum(planes, [])


def paff_field_filtered_pcm_packets(fmt, yd, cd, cabac, deblock_idc, aso):
    sx = 2 if fmt == 2 else 1
    planes = sample_planes(fmt, yd, cd, 32, 32, 0)
    packets = []
    for index, bottom in enumerate([False, True]):
        slices = [[0], [1]] if aso else [[0, 1]]
        nals = []
        for addresses in slices:
            b = Writer()
            write_slice_header(
                b,
                addresses[0],
                index,
                field_pic='bottom' if bottom else 'top',
                deblock_idc=deblock_idc,
            )
            if cabac:
                while len(b.bits) % 8:
                    b.u(1)
                c = CabacWriter(-1, 26)
            for pos, mb in enumerate(addresses):
                if cabac:
                    if pos > 0:
                        c.range -= 2
                        c.renormalize()
                    c.decision(3 + int(pos > 0), 1)
                    b.bits.extend(c.finish())
                    b.align()
                else:
                    b.ue(25)
                    b.align()
                for p, depth in enumerate([yd, cd, cd]):
                    if fmt == 0 and p > 0:
                        continue
                    bw = 16 if p == 0 else 16 // sx
                    stride = 32 if p == 0 else 32 // sx
                    for row in range(16):
                        for col in range(bw):
                            b.u(planes[p][(2 * row + int(bottom)) * stride + mb * bw + col], depth)
                if cabac:
                    c.low = 0
                    c.range = 510
                    c.width = 9
            if cabac:
                b.bits.extend(c.finish())
            nals.append(b.nal(0x65 if index == 0 else 0x41, trailing=not cabac))
        if aso:
            nals.reverse()
        packets.append(b''.join(len(n).to_bytes(4, 'big') + n for n in nals))
    return packets, sum(planes, [])


def mbaff_filtered_pcm_packet(fmt, yd, cd, cabac, topology, deblock_idc, aso, index):
    sx = 2 if fmt == 2 else 1
    planes = sample_planes(fmt, yd, cd, 32, 64, index)
    nals = []
    for pair, field in enumerate(MODES[topology]):
        b = Writer()
        write_slice_header(b, pair, index, field_pic='frame', deblock_idc=deblock_idc)
        if cabac:
            while len(b.bits) % 8:
                b.u(1)
            c = CabacWriter(-1, 26)
        for parity in range(2):
            if parity == 0:
                if cabac:
                    c.decision(70, int(field))
                else:
                    b.u(int(field))
            if cabac:
                c.decision(3 + int(parity == 1 and not field), 1)
                b.bits.extend(c.finish())
                b.align()
            else:
                b.ue(25)
                b.align()
            for p, depth in enumerate([yd, cd, cd]):
                if fmt == 0 and p > 0:
                    continue
                bw = 16 if p == 0 else 16 // sx
                stride = 32 if p == 0 else 32 // sx
                for row in range(16):
                    y = pair // 2 * 32 + (row * 2 + parity if field else row + parity * 16)
                    for col in range(bw):
                        b.u(planes[p][y * stride + (pair % 2) * bw + col], depth)
            if cabac:
                c.low = 0
                c.range = 510
                c.width = 9
        if cabac:
            b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index == 0 else 0x41, trailing=not cabac))
    if aso:
        nals.reverse()
    return b''.join(len(n).to_bytes(4, 'big') + n for n in nals), sum(planes, [])


def paff_frame_422_p_packets(yd, cd, cabac, weighted, deblock_idc):
    planes = sample_planes(2, yd, cd, 32, 32, 0)
    first, first_samples = progressive_or_paff_frame_pcm_packet(2, yd, cd, cabac, 32, 0, True, deblock_idc, False)
    b = Writer()
    b.ue(0)  # first_mb_in_slice
    b.ue(0)  # slice_type = P
    b.ue(0)  # pic_parameter_set_id
    b.u(1, 4)  # frame_num
    b.u(0)  # field_pic_flag = 0 (PAFF frame!)
    b.u(2, 4)  # pic_order_cnt_lsb = 2
    b.u(0)  # num_ref_idx_active_override_flag
    b.u(0)  # ref_pic_list_modification_flag_l0
    if weighted:
        b.ue(1)  # luma_log2_weight_denom = 1
        b.ue(2)  # chroma_log2_weight_denom = 2
        b.u(1)  # luma_weight_l0_flag
        b.se(3)
        b.se(-5)
        b.u(1)  # chroma_weight_l0_flag
        b.se(3)
        b.se(7)
        b.se(-2)
        b.se(64)
    b.u(0)  # adaptive_ref_pic_marking_mode_flag
    if cabac:
        b.ue(0)  # cabac_init_idc
    b.se(0)  # slice_qp_delta
    b.ue(deblock_idc)
    if deblock_idc != 1:
        b.se(-2)
        b.se(1)
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(0, 26)
        for mb in range(4):
            c.decision(11, 1)
            if mb < 3:
                c.range -= 2
                c.renormalize()
        b.bits.extend(c.finish())
    else:
        b.ue(4)  # mb_skip_run = 4
    nal = b.nal(0x41, trailing=not cabac)
    second = len(nal).to_bytes(4, 'big') + nal
    predicted = [p[:] for p in planes]
    if weighted:
        for component, (depth, weight, offset, denom) in enumerate([(yd, 3, -5, 1), (cd, 3, 7, 2), (cd, -2, 64, 2)]):
            predicted[component] = [
                max(0, min((1 << depth) - 1, ((weight * v + (1 << (denom - 1))) >> denom) + (offset << (depth - 8))))
                for v in predicted[component]
            ]
    return [(first, first_samples), (second, sum(predicted, []))]


def paff_frame_422_b_packets(yd, cd, cabac, bipred, mode, deblock_idc):
    from generate_avc_field_b_cabac_mixed_samples import mb_type
    depths = [yd, cd, cd]
    a = sample_planes(2, yd, cd, 32, 32, 0)
    future = [[(3 * v + 29) & ((1 << depth) - 1) for v in plane] for plane, depth in zip(a, depths)]
    first, first_samples = progressive_or_paff_frame_pcm_packet(2, yd, cd, cabac, 32, 0, True, deblock_idc, False)

    # Second packet in decode order: reference I-frame with frame_num = 1, poc = 6
    b_ref = Writer()
    b_ref.ue(0)
    b_ref.ue(2)
    b_ref.ue(0)
    b_ref.u(1, 4)  # frame_num = 1
    b_ref.u(0)  # field_pic_flag = 0
    b_ref.u(6, 4)  # pic_order_cnt_lsb = 6
    b_ref.u(0)  # adaptive_ref_pic_marking_mode_flag
    b_ref.se(0)
    b_ref.ue(deblock_idc)
    if deblock_idc != 1:
        b_ref.se(-2)
        b_ref.se(1)
    if cabac:
        while len(b_ref.bits) % 8:
            b_ref.u(1)
        c = CabacWriter(-1, 26)
    for mb in range(4):
        if cabac:
            if mb > 0:
                c.range -= 2
                c.renormalize()
            left = mb % 2 != 0
            top = mb >= 2
            c.decision(3 + int(left) + int(top), 1)
            b_ref.bits.extend(c.finish())
            b_ref.align()
        else:
            b_ref.ue(25)
            b_ref.align()
        mx = mb % 2
        my = mb // 2
        for p, (depth, stride, bw) in enumerate([(yd, 32, 16), (cd, 16, 8), (cd, 16, 8)]):
            for row in range(16):
                for col in range(bw):
                    b_ref.u(future[p][(my * 16 + row) * stride + mx * bw + col], depth)
        if cabac:
            c.low = 0
            c.range = 510
            c.width = 9
    if cabac:
        b_ref.bits.extend(c.finish())
    nal_ref = b_ref.nal(0x41, trailing=not cabac)
    later = len(nal_ref).to_bytes(4, 'big') + nal_ref

    # Third packet in decode order: non-reference PAFF frame B-picture with frame_num = 2, poc = 2
    weights = [[3, 1, -2], [-1, 3, 4]]
    offsets = [[-7, 5, 64], [9, -4, -32]]
    b = Writer()
    b.ue(0)  # first_mb_in_slice
    b.ue(1)  # slice_type = B
    b.ue(0)  # pic_parameter_set_id
    b.u(2, 4)  # frame_num = 2
    b.u(0)  # field_pic_flag = 0 (PAFF frame!)
    b.u(2, 4)  # pic_order_cnt_lsb = 2
    b.u(int(mode != 'temporal'))  # direct_spatial_mv_pred_flag
    b.u(0)  # num_ref_idx_active_override_flag
    b.u(0)  # ref_pic_list_modification_flag_l0
    b.u(0)  # ref_pic_list_modification_flag_l1
    if bipred == 1:
        b.ue(1)
        b.ue(1)
        for l in range(2):
            b.u(1)
            b.se(weights[l][0])
            b.se(offsets[l][0])
            b.u(1)
            for p in [1, 2]:
                b.se(weights[l][p])
                b.se(offsets[l][p])
    if cabac:
        b.ue(0)
    b.se(0)
    b.ue(deblock_idc)
    if deblock_idc != 1:
        b.se(-2)
        b.se(1)
    code = {'l0': 1, 'l1': 2, 'bi': 3, 'spatial': 0, 'temporal': 0, 'skip': 0}[mode]
    if cabac:
        while len(b.bits) % 8:
            b.u(1)
        c = CabacWriter(0, 26)
        for mb in range(4):
            left_not_skip = (mb % 2 == 1) and (mode != 'skip')
            top_not_skip = (mb >= 2) and (mode != 'skip')
            c.decision(24 + int(left_not_skip) + int(top_not_skip), int(mode == 'skip'))
            if mode != 'skip':
                mb_type(c, code, (int(mb % 2 == 1) + int(mb >= 2)) if code != 0 else 0)
                for _ in range(2 if code == 3 else 1 if code in [1, 2] else 0):
                    c.mvd(0, 0)
                    c.mvd(1, 0)
                left_zero = mb % 2 == 1
                top_zero = mb >= 2
                c.decision(73 + (1 if left_zero else 0) + (2 if top_zero else 0), 0)
                c.decision(74 + (2 if top_zero else 0), 0)
                c.decision(75 + (1 if left_zero else 0), 0)
                c.decision(76, 0)
                c.decision(77, 0)
            if mb < 3:
                c.range -= 2
                c.renormalize()
        b.bits.extend(c.finish())
    elif mode == 'skip':
        b.ue(4)
    else:
        for _ in range(4):
            b.ue(0)
            b.ue(code)
            for _ in range(2 if code == 3 else 1 if code in [1, 2] else 0):
                b.se(0)
                b.se(0)
            b.ue(0)
    nal = b.nal(0x01, trailing=not cabac)
    middle = len(nal).to_bytes(4, 'big') + nal

    target = []
    for p, depth in enumerate(depths):
        maximum = (1 << depth) - 1
        scale = 1 << (depth - 8)
        values = []
        for x, y in zip(a[p], future[p]):
            if mode in ['l0', 'l1']:
                l = int(mode == 'l1')
                v = [x, y][l]
                v = ((weights[l][p] * v + 1) >> 1) + offsets[l][p] * scale if bipred == 1 else v
            elif bipred == 0:
                v = (x + y + 1) // 2
            elif bipred == 2:
                v = (43 * x + 21 * y + 32) // 64
            else:
                v = ((weights[0][p] * x + weights[1][p] * y + 2) >> 2) + ((sum(row[p] for row in offsets) * scale + 1) >> 1)
            values.append(max(0, min(maximum, v)))
        target.append(values)
    return [(first, first_samples), (later, sum(future, [])), (middle, sum(target, []))]


def generate():
    OUT.mkdir(exist_ok=True)
    cases = []
    golden = bytearray()

    def add_frame_case(name, kind, fmt, yd, cd, width, height, crop, config, frame_packets_and_samples):
        (OUT / f'{name}.avcc').write_bytes(config)
        mp4_packets = []
        records = []
        for i, (packet, samples) in enumerate(frame_packets_and_samples):
            (OUT / f'{name}-{i}.packet').write_bytes(packet)
            mp4_packets.append((i, i == 0, packet))
            at = len(golden) // 2
            golden.extend(struct.pack('<' + str(len(samples)) + 'H', *samples))
            records.append(dict(packet=f'{name}-{i}.packet', offset=at))
        (OUT / f'{name}.mp4').write_bytes(mux(config, mp4_packets, width, height, 25))
        cases.append(dict(
            name=name,
            kind=kind,
            format=fmt,
            luma_depth=yd,
            chroma_depth=cd,
            width=width,
            height=height,
            crop=crop,
            frames=records,
        ))

    # 1. Progressive 4:4:4 I_PCM with deblocking enabled (disable_deblocking_filter_idc = 0 and 2)
    for (yd, cd), cabac, deblock_idc, aso, cropped in itertools.product(
        [(8, 8), (10, 12), (14, 8)],
        [False, True],
        [0, 2],
        [False, True],
        [False, True],
    ):
        name = f'prog-filtered-f3-y{yd}-c{cd}-e{int(cabac)}-d{deblock_idc}-a{int(aso)}-crop{int(cropped)}'
        config = prog_configuration(3, yd, cd, cropped, cabac=cabac, height_mbs=1)
        frames = [
            progressive_or_paff_frame_pcm_packet(3, yd, cd, cabac, 16, i, False, deblock_idc, aso)
            for i in range(2)
        ]
        add_frame_case(name, 'progressive', 3, yd, cd, 32, 16, [1, 1, 1, 1] if cropped else [0, 0, 0, 0], config, frames)

    # 2. PAFF frame I_PCM across formats 0, 2, 3 (CAVLC/CABAC, deblocking 1/0/2, ASO, crop)
    for fmt, (yd, cd), cabac, deblock_idc, aso in itertools.product(
        [0, 2, 3],
        [(8, 8), (10, 12)],
        [False, True],
        [1, 0, 2],
        [False, True],
    ):
        sx = 2 if fmt == 2 else 1
        cropped = aso
        name = f'paff-frame-pcm-f{fmt}-y{yd}-c{cd}-e{int(cabac)}-d{deblock_idc}-a{int(aso)}-crop{int(cropped)}'
        config = interlaced_configuration(fmt, yd, cd, cropped, cabac=cabac, height_mbs=1, mbaff=False)
        frames = [
            progressive_or_paff_frame_pcm_packet(fmt, yd, cd, cabac, 32, i, True, deblock_idc, aso)
            for i in range(2)
        ]
        add_frame_case(name, 'paff_frame', fmt, yd, cd, 32, 32, [sx, sx, 2, 2] if cropped else [0, 0, 0, 0], config, frames)

    # 3. PAFF frame compressed I_16x16 across formats 0 and 2 (CAVLC/CABAC, deblocking 1/0)
    for fmt, (yd, cd), cabac, deblock_idc in itertools.product(
        [0, 2],
        [(8, 8), (10, 12)],
        [False, True],
        [1, 0],
    ):
        sx = 2 if fmt == 2 else 1
        name = f'paff-frame-i16-f{fmt}-y{yd}-c{cd}-e{int(cabac)}-d{deblock_idc}'
        config = interlaced_configuration(fmt, yd, cd, True, cabac=cabac, height_mbs=1, mbaff=False)
        frames = [
            paff_frame_compressed_packet(fmt, yd, cd, cabac, i, deblock_idc)
            for i in range(2)
        ]
        add_frame_case(name, 'paff_frame_compressed', fmt, yd, cd, 32, 32, [sx, sx, 2, 2], config, frames)

    # 4. PAFF field 4:4:4 I_PCM with deblocking enabled (disable_deblocking_filter_idc = 0 and 2)
    for (yd, cd), cabac, deblock_idc, aso, cropped in itertools.product(
        [(8, 8), (10, 12), (14, 8)],
        [False, True],
        [0, 2],
        [False, True],
        [False, True],
    ):
        name = f'paff-field-filtered-f3-y{yd}-c{cd}-e{int(cabac)}-d{deblock_idc}-a{int(aso)}-crop{int(cropped)}'
        config = interlaced_configuration(3, yd, cd, cropped, cabac=cabac, height_mbs=1, mbaff=False)
        (OUT / f'{name}.avcc').write_bytes(config)
        packets, samples = paff_field_filtered_pcm_packets(3, yd, cd, cabac, deblock_idc, aso)
        for i, p in enumerate(packets):
            (OUT / f'{name}-{i}.packet').write_bytes(p)
        at = len(golden) // 2
        golden.extend(struct.pack('<' + str(len(samples)) + 'H', *samples))
        (OUT / f'{name}.mp4').write_bytes(mux(config, [(i, i == 0, p) for i, p in enumerate(packets)], 32, 32, 50))
        cases.append(dict(
            name=name,
            kind='paff_field',
            format=3,
            luma_depth=yd,
            chroma_depth=cd,
            width=32,
            height=32,
            crop=[1, 1, 2, 2] if cropped else [0, 0, 0, 0],
            frames=[dict(packets=[f'{name}-0.packet', f'{name}-1.packet'], offset=at)],
        ))

    # 5. MBAFF 4:4:4 I_PCM with deblocking enabled (disable_deblocking_filter_idc = 0 and 2)
    for (yd, cd), cabac, topology, deblock_idc, aso in itertools.product(
        [(8, 8), (10, 12)],
        [False, True],
        MODES,
        [0, 2],
        [False, True],
    ):
        cropped = aso
        name = f'mbaff-filtered-f3-y{yd}-c{cd}-e{int(cabac)}-{topology}-d{deblock_idc}-a{int(aso)}-crop{int(cropped)}'
        config = interlaced_configuration(3, yd, cd, cropped, cabac=cabac, height_mbs=2, mbaff=True)
        frames = [
            mbaff_filtered_pcm_packet(3, yd, cd, cabac, topology, deblock_idc, aso, i)
            for i in range(2)
        ]
        add_frame_case(name, 'mbaff', 3, yd, cd, 32, 64, [1, 1, 2, 2] if cropped else [0, 0, 0, 0], config, frames)

    # 6. PAFF frame 4:2:2 P and B inter pictures (frame_mbs_only_flag = 0, mb_adaptive_frame_field_flag = 0, field_pic_flag = 0)
    for (yd, cd), cabac, weighted, deblock_idc in itertools.product(
        [(8, 8), (10, 12)],
        [False, True],
        [False, True],
        [1, 0],
    ):
        name = f'paff-frame-422-p-y{yd}-c{cd}-e{int(cabac)}-w{int(weighted)}-d{deblock_idc}'
        config = interlaced_configuration(2, yd, cd, True, cabac=cabac, height_mbs=1, weighted=weighted, mbaff=False)
        frames = paff_frame_422_p_packets(yd, cd, cabac, weighted, deblock_idc)
        add_frame_case(name, 'paff_frame_422_p', 2, yd, cd, 32, 32, [2, 2, 2, 2], config, frames)

    for (yd, cd), cabac, bipred, mode in itertools.product(
        [(8, 8), (10, 12)],
        [False, True],
        [0, 1, 2],
        ['l0', 'l1', 'bi', 'spatial', 'temporal', 'skip'],
    ):
        name = f'paff-frame-422-b-y{yd}-c{cd}-e{int(cabac)}-w{bipred}-{mode}'
        config = interlaced_configuration(2, yd, cd, True, cabac=cabac, height_mbs=1, bipred=bipred, mbaff=False)
        frames = paff_frame_422_b_packets(yd, cd, cabac, bipred, mode, 0)
        (OUT / f'{name}.avcc').write_bytes(config)
        records = []
        for i, (packet, samples) in enumerate(frames):
            (OUT / f'{name}-{i}.packet').write_bytes(packet)
            at = len(golden) // 2
            golden.extend(struct.pack('<' + str(len(samples)) + 'H', *samples))
            records.append(dict(packet=f'{name}-{i}.packet', offset=at))
        (OUT / f'{name}.mp4').write_bytes(
            mux(config, [(0, True, frames[0][0]), (2, False, frames[1][0]), (1, False, frames[2][0])], 32, 32, 25)
        )
        cases.append(dict(
            name=name,
            kind='paff_frame_422_b',
            format=2,
            luma_depth=yd,
            chroma_depth=cd,
            width=32,
            height=32,
            crop=[2, 2, 2, 2],
            frames=records,
        ))

    (OUT / 'samples.y16').write_bytes(golden)
    (OUT / 'manifest.json').write_text(json.dumps(dict(
        provenance='Owned non-4:2:0 PAFF frame and filtered I_PCM fixtures across monochrome/4:2:2/4:4:4. No external codec or private media.',
        cases=cases,
    ), indent=2) + '\n')


if __name__ == '__main__':
    generate()

