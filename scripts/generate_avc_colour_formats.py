#!/usr/bin/env python3
"""Owned AVC Matroska non-4:2:0 (monochrome/4:2:2/4:4:4) and Identity RGB (GBR) fixtures.
No encoding, external codec, production decoder, private media or network.
"""
import hashlib
import json
import struct
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux
from generate_hevc_matroska_formats import movie as hevc_movie, display as yuv_display
from generate_hevc_identity_rgb import display as rgb_display

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/avc-colour-formats'


def avc_configuration(fmt, ydepth, cdepth, cropped, vui=None, separate=False, level=51):
    b = Writer()
    for value in [244, 0, level]:
        b.u(value, 8)
    b.ue(0)
    b.ue(fmt)
    if fmt == 3:
        b.u(int(separate))
    b.ue(ydepth - 8)
    b.ue(cdepth - 8)
    b.u(0)
    b.u(0)
    b.ue(0)
    b.ue(0)
    b.ue(0)
    b.ue(1)
    b.u(0)
    b.ue(1)
    b.ue(0)
    b.u(1)
    b.u(1)
    b.u(int(cropped))
    if cropped:
        for _ in range(4):
            b.ue(1)
    if vui is None:
        b.u(0)
    else:
        full, primaries, transfer, matrix = vui
        b.u(1)  # vui_parameters_present_flag
        b.u(0)  # aspect_ratio_info_present_flag
        b.u(0)  # overscan_info_present_flag
        b.u(1)  # video_signal_type_present_flag
        b.u(5, 3)  # video_format = Unspecified
        b.u(int(full))  # video_full_range_flag
        b.u(1)  # colour_description_present_flag
        b.u(primaries, 8)
        b.u(transfer, 8)
        b.u(matrix, 8)
        b.u(0)  # chroma_loc_info_present_flag
        b.u(0)  # timing_info_present_flag
        b.u(0)  # nal_hrd_parameters_present_flag
        b.u(0)  # vcl_hrd_parameters_present_flag
        b.u(0)  # pic_struct_present_flag
        b.u(0)  # bitstream_restriction_flag
    sps = b.nal(0x67)
    b = Writer()
    b.ue(0)
    b.ue(0)
    b.u(0)
    b.u(0)
    b.ue(0)
    b.ue(0)
    b.ue(0)
    b.u(0)
    b.u(0, 2)
    b.se(0)
    b.se(0)
    b.se(0)
    b.u(1)
    b.u(0)
    b.u(0)
    pps = b.nal(0x68)
    return (
        bytes([1, 244, 0, level, 255, 225])
        + len(sps).to_bytes(2, 'big')
        + sps
        + bytes([1])
        + len(pps).to_bytes(2, 'big')
        + pps
    )


def avc_movie(private, packets, width, height, rate, rotation, crop):
    from generate_ffv1_gray_fixtures import element, uint
    video = uint(0xB0, width) + uint(0xBA, height)
    video += b''.join(
        uint(tag, n) for tag, n in zip([0x54CC, 0x54BB, 0x54DD, 0x54AA], crop)
    )
    video += element(
        0x7670,
        uint(0x7671, 0)
        + element(0x7675, struct.pack('>d', 90.0 if rotation == 270 else -float(rotation))),
    )
    track = (
        uint(0xD7, 1)
        + uint(0x83, 1)
        + element(0x86, b'V_MPEG4/ISO/AVC')
        + element(0x63A2, private)
        + uint(0x23E383, 1000000000 // rate)
        + element(0xE0, video)
    )
    blocks = b''.join(
        element(
            0xA3,
            b'\x81'
            + (i * 1000 // rate).to_bytes(2, 'big')
            + (b'\x80' if i == 0 else b'\0')
            + p,
        )
        for i, p in enumerate(packets)
    )
    return element(0x1A45DFA3, element(0x4282, b'matroska')) + element(
        0x18538067,
        element(0x1549A966, uint(0x2AD7B1, 1000000))
        + element(0x1654AE6B, element(0xAE, track))
        + element(0x1F43B675, uint(0xE7, 0) + blocks),
    )


def make_pcm_packets(fmt, yd, cd, separate=False):
    sx, sy = (2, 1) if fmt == 2 else (1, 1)
    packets = []
    planes_per_frame = []
    for i in range(2):
        planes = []
        for p, depth in enumerate([yd, cd, cd]):
            w, h = (32, 16) if p == 0 else (0, 0) if fmt == 0 else (32 // sx, 16 // sy)
            maximum = (1 << depth) - 1
            values = [
                (x * 31 + y * 7 + p * 43 + i * 17) & maximum
                for y in range(h)
                for x in range(w)
            ]
            if values:
                values[:4] = [0, maximum, 1, maximum - 1]
            planes.append(values)
        planes_per_frame.append(planes)
        if separate:
            nals = []
            for plane_id in range(3):
                b = Writer()
                b.ue(0)
                b.ue(2)
                b.ue(0)
                b.u(plane_id, 2)
                b.u(i, 4)
                if i == 0:
                    b.ue(0)
                b.u(i * 2, 4)
                if i == 0:
                    b.u(0)
                    b.u(0)
                else:
                    b.u(0)
                b.se(0)
                b.ue(1)
                depth = yd if plane_id == 0 else cd
                for mb in range(2):
                    b.ue(25)
                    b.align()
                    for y in range(16):
                        for x in range(16):
                            b.u(planes[plane_id][y * 32 + mb * 16 + x], depth)
                nal = b.nal(0x65 if i == 0 else 0x41)
                nals.append(len(nal).to_bytes(4, 'big') + nal)
            packets.append(b''.join(nals))
        else:
            b = Writer()
            b.ue(0)
            b.ue(2)
            b.ue(0)
            b.u(i, 4)
            if i == 0:
                b.ue(0)
            b.u(i * 2, 4)
            if i == 0:
                b.u(0)
                b.u(0)
            else:
                b.u(0)
            b.se(0)
            b.ue(1)
            for mb in range(2):
                b.ue(25)
                b.align()
                for p, depth in enumerate([yd, cd, cd]):
                    if p and fmt == 0:
                        continue
                    width, height = (16, 16) if p == 0 else (16 // sx, 16 // sy)
                    stride = 32 if p == 0 else 32 // sx
                    for y in range(height):
                        for x in range(width):
                            b.u(planes[p][y * stride + mb * width + x], depth)
            nal = b.nal(0x65 if i == 0 else 0x41)
            packets.append(len(nal).to_bytes(4, 'big') + nal)
    return packets, planes_per_frame


def pack_planar_oracle(fmt, yd, cd, sps_crop, planes):
    output_depth = yd if fmt == 0 else max(yd, cd)
    sub = (2, 1) if fmt == 2 else (2, 2) if fmt == 0 else (1, 1)
    width = 32 - sps_crop[0] - sps_crop[1]
    height = 16 - sps_crop[2] - sps_crop[3]
    values = []
    for c in range(3):
        if c != 0 and fmt == 0:
            cw = (width + 1) // 2
            ch = (height + 1) // 2
            values.extend([1 << (output_depth - 1)] * (cw * ch))
            continue
        sx, sy = (1, 1) if c == 0 else sub
        stride = 32 // sx
        source_depth = yd if c == 0 else cd
        plane = planes[c]
        for y in range(height // sy):
            for x in range(width // sx):
                val = plane[(sps_crop[2] // sy + y) * stride + sps_crop[0] // sx + x]
                values.append(val << (output_depth - source_depth))
    return values, width, height, output_depth, sub


def generate():
    OUT.mkdir(parents=True, exist_ok=True)
    mkv_cases = []
    rgb_cases = []
    matrix_cases = []
    refusals = []
    rate = 25

    # 1. Matroska non-4:2:0 (fmt 0, 2, 3) + 4:2:0 control across depths, SPS crop, MKV crop, and rotation
    for fmt in [0, 2, 3]:
        sx, sy = (2, 1) if fmt == 2 else (1, 1)
        for yd, cd in [(8, 8), (10, 12), (14, 8)]:
            for sps_cropped in [False, True]:
                sps_crop = [sx, sx, sy, sy] if sps_cropped else [0, 0, 0, 0]
                config = avc_configuration(fmt, yd, cd, sps_cropped)
                packets, planes_per_frame = make_pcm_packets(fmt, yd, cd)
                w, h = 32 - sps_crop[0] - sps_crop[1], 16 - sps_crop[2] - sps_crop[3]
                mp4_name = f'avc-mkv-fmt{fmt}-y{yd}-c{cd}-sc{int(sps_cropped)}.mp4'
                mp4_frames = [(i, i == 0, p) for i, p in enumerate(packets)]
                (OUT / mp4_name).write_bytes(mux(config, mp4_frames, w, h, rate))

                for rotation in [0, 90, 180, 270]:
                    for mkv_crop in [[0, 0, 0, 0], [2, 2, 2, 2]]:
                        stem = f'avc-mkv-fmt{fmt}-y{yd}-c{cd}-sc{int(sps_cropped)}-r{rotation}-mc{mkv_crop[0]}'
                        mkv_name = stem + '.mkv'
                        mkv_bytes = avc_movie(config, packets, w, h, rate, rotation, mkv_crop)
                        (OUT / mkv_name).write_bytes(mkv_bytes)
                        frame_records = []
                        for i, planes in enumerate(planes_per_frame):
                            values, vw, vh, out_depth, sub = pack_planar_oracle(
                                fmt, yd, cd, sps_crop, planes
                            )
                            unit = 1 if out_depth == 8 else 2
                            serialize = (
                                (lambda v: bytes(v))
                                if unit == 1
                                else (lambda v: b''.join(n.to_bytes(2, 'little') for n in v))
                            )
                            disp = yuv_display(values, vw, vh, sub, mkv_crop, rotation)
                            packed_file = f'{stem}-{i}.packed'
                            disp_file = f'{stem}-{i}.display'
                            (OUT / packed_file).write_bytes(serialize(values))
                            (OUT / disp_file).write_bytes(serialize(disp))
                            frame_records.append({'packed': packed_file, 'display': disp_file})
                        mkv_cases.append(
                            {
                                'file': mkv_name,
                                'source': mp4_name,
                                'size': [w, h],
                                'depth': yd if fmt == 0 else max(yd, cd),
                                'luma_depth': yd,
                                'chroma_depth': cd,
                                'chroma_format': fmt,
                                'subsampling': list((2, 1) if fmt == 2 else (2, 2) if fmt == 0 else (1, 1)),
                                'rotation': rotation,
                                'crop': mkv_crop,
                                'rate': rate,
                                'frames': frame_records,
                                'sha256': hashlib.sha256(mkv_bytes).hexdigest(),
                            }
                        )

    # 2. AVC Identity RGB (matrix_coefficients = 0 on 4:4:4, both non-separate and separate_colour_plane_flag = 1)
    for depth in [8, 10, 12, 14]:
        for separate in [False, True]:
            for full in [True, False]:
                for sps_cropped in [False, True]:
                    sps_crop = [1, 1, 1, 1] if sps_cropped else [0, 0, 0, 0]
                    w, h = 32 - sps_crop[0] - sps_crop[1], 16 - sps_crop[2] - sps_crop[3]
                    config = avc_configuration(
                        3,
                        depth,
                        depth,
                        sps_cropped,
                        vui=(full, 1, 13, 0),
                        separate=separate,
                    )
                    packets, planes_per_frame = make_pcm_packets(3, depth, depth, separate=separate)
                    stem = (
                        f'avc-identity-rgb-d{depth}-sep{int(separate)}-'
                        + ('full' if full else 'limited')
                        + f'-sc{int(sps_cropped)}'
                    )
                    mp4_name = stem + '.mp4'
                    mp4_frames = [(i, i == 0, p) for i, p in enumerate(packets)]
                    (OUT / mp4_name).write_bytes(mux(config, mp4_frames, w, h, rate))
                    unit = 1 if depth == 8 else 2
                    serialize = (
                        (lambda v: bytes(v))
                        if unit == 1
                        else (lambda v: b''.join(n.to_bytes(2, 'little') for n in v))
                    )

                    def rgb8(v, d=depth, is_full=full):
                        if is_full:
                            den = (1 << d) - 1
                            offset = 0
                        else:
                            den = 219 * (1 << (d - 8))
                            offset = 16 * (1 << (d - 8))
                        return bytes(
                            min(255, max(0, ((n - offset) * 255 + den // 2) // den))
                            for n in v
                        )

                    inputs = []
                    for planes in planes_per_frame:
                        values, vw, vh, _, _ = pack_planar_oracle(
                            3, depth, depth, sps_crop, planes
                        )
                        n = vw * vh
                        # GBR [0, 1, 2] -> RGB [2, 0, 1]
                        rgb = [
                            v
                            for idx in range(n)
                            for v in [values[2 * n + idx], values[idx], values[n + idx]]
                        ]
                        inputs.append(rgb)

                    for rotation in [0, 90]:
                        for mkv_crop in [[0, 0, 0, 0], [2, 2, 2, 2]]:
                            case_stem = f'{stem}-r{rotation}-mc{mkv_crop[0]}'
                            mkv_name = case_stem + '.mkv'
                            mkv_bytes = avc_movie(
                                config, packets, w, h, rate, rotation, mkv_crop
                            )
                            (OUT / mkv_name).write_bytes(mkv_bytes)
                            frame_records = []
                            for i, rgb in enumerate(inputs):
                                turned = rgb_display(rgb, w, h, mkv_crop, rotation)
                                paths = {}
                                for key, content in [
                                    ('rgb', serialize(rgb)),
                                    ('rgb8', rgb8(rgb)),
                                    ('display', serialize(turned)),
                                    ('display8', rgb8(turned)),
                                ]:
                                    fname = f'{case_stem}-{i}.{key}'
                                    (OUT / fname).write_bytes(content)
                                    paths[key] = fname
                                frame_records.append(paths)
                            rgb_cases.append(
                                {
                                    'file': mkv_name,
                                    'mp4': mp4_name,
                                    'size': [w, h],
                                    'depth': depth,
                                    'separate': separate,
                                    'full_range': full,
                                    'rotation': rotation,
                                    'crop': mkv_crop,
                                    'rate': rate,
                                    'frames': frame_records,
                                    'sha256': hashlib.sha256(mkv_bytes).hexdigest(),
                                }
                            )

    # 3. AVC VUI Matrix Coefficients (1=BT.709, 4=FCC, 5=BT.470BG, 6=SMPTE170M, 7=SMPTE240M, 9=BT.2020NCL, 10=BT.2020CL)
    matrix_coeffs = [
        (1, 0.2126, 0.0722),
        (4, 0.30, 0.11),
        (5, 0.299, 0.114),
        (6, 0.299, 0.114),
        (7, 0.212, 0.087),
        (9, 0.2627, 0.0593),
        (10, 0.2627, 0.0593),
    ]
    for code, kr, kb in matrix_coeffs:
        for fmt in [1, 3]:
            for depth in [8, 10]:
                for full in [False, True]:
                    # For fmt == 1, make_pcm_packets needs 4:2:0 support!
                    # Let's build 4:2:0 or 4:4:4 packets:
                    sx, sy = (2, 2) if fmt == 1 else (1, 1)
                    config = avc_configuration(
                        fmt, depth, depth, False, vui=(full, 1, 1, code)
                    )
                    packets = []
                    planes_list = []
                    for i in range(2):
                        planes = []
                        for p in range(3):
                            pw, ph = (32, 16) if p == 0 else (32 // sx, 16 // sy)
                            maximum = (1 << depth) - 1
                            vals = [
                                (x * 29 + y * 13 + p * 53 + i * 19 + 64) & maximum
                                for y in range(ph)
                                for x in range(pw)
                            ]
                            planes.append(vals)
                        planes_list.append(planes)
                        b = Writer()
                        b.ue(0)
                        b.ue(2)
                        b.ue(0)
                        b.u(i, 4)
                        if i == 0:
                            b.ue(0)
                        b.u(i * 2, 4)
                        if i == 0:
                            b.u(0)
                            b.u(0)
                        else:
                            b.u(0)
                        b.se(0)
                        b.ue(1)
                        for mb in range(2):
                            b.ue(25)
                            b.align()
                            for p in range(3):
                                bw, bh = (16, 16) if p == 0 else (16 // sx, 16 // sy)
                                stride = 32 if p == 0 else 32 // sx
                                for y in range(bh):
                                    for x in range(bw):
                                        b.u(planes[p][y * stride + mb * bw + x], depth)
                        nal = b.nal(0x65 if i == 0 else 0x41)
                        packets.append(len(nal).to_bytes(4, 'big') + nal)
                    stem = f'avc-matrix{code}-fmt{fmt}-d{depth}-' + ('full' if full else 'limited')
                    mp4_name = stem + '.mp4'
                    mkv_name = stem + '.mkv'
                    (OUT / mp4_name).write_bytes(
                        mux(config, [(i, i == 0, p) for i, p in enumerate(packets)], 32, 16, rate)
                    )
                    (OUT / mkv_name).write_bytes(
                        avc_movie(config, packets, 32, 16, rate, 0, [0, 0, 0, 0])
                    )
                    matrix_cases.append(
                        {
                            'mp4': mp4_name,
                            'mkv': mkv_name,
                            'matrix': code,
                            'kr': kr,
                            'kb': kb,
                            'full_range': full,
                            'chroma_format': fmt,
                            'depth': depth,
                        }
                    )

    # 4. Refusal: AVC Identity RGB on non-4:4:4 (fmt 1 and fmt 2)
    for fmt in [1, 2]:
        sx, sy = (2, 2) if fmt == 1 else (2, 1)
        config = avc_configuration(fmt, 8, 8, False, vui=(True, 1, 13, 0))
        b = Writer()
        b.ue(0)
        b.ue(2)
        b.ue(0)
        b.u(0, 4)
        b.ue(0)
        b.u(0, 4)
        b.u(0)
        b.u(0)
        b.se(0)
        b.ue(1)
        for mb in range(2):
            b.ue(25)
            b.align()
            for p in range(3):
                bw, bh = (16, 16) if p == 0 else (16 // sx, 16 // sy)
                for _ in range(bw * bh):
                    b.u(128, 8)
        nal = b.nal(0x65)
        packet = len(nal).to_bytes(4, 'big') + nal
        mp4_name = f'avc-identity-rgb-invalid-fmt{fmt}.mp4'
        mkv_name = f'avc-identity-rgb-invalid-fmt{fmt}.mkv'
        (OUT / mp4_name).write_bytes(mux(config, [(0, True, packet)], 32, 16, rate))
        (OUT / mkv_name).write_bytes(
            avc_movie(config, [packet], 32, 16, rate, 0, [0, 0, 0, 0])
        )
        refusals.append(
            {
                'mp4': mp4_name,
                'mkv': mkv_name,
                'error': 'AVC identity RGB requires 4:4:4',
            }
        )

    (OUT / 'manifest.json').write_text(
        json.dumps(
            {
                'provenance': 'Owned synthetic AVC non-4:2:0 Matroska, Identity RGB (GBR), and VUI matrix coefficient fixtures. No external codec, private media, or network.',
                'mkv_cases': mkv_cases,
                'rgb_cases': rgb_cases,
                'matrix_cases': matrix_cases,
                'refusals': refusals,
            },
            indent=2,
        )
        + '\n'
    )


if __name__ == '__main__':
    generate()
