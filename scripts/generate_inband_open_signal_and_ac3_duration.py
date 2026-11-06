#!/usr/bin/env python3
"""Offline generator for Batch 568 (FVID-0571..0574) reproducers:
- FVID-0571: Matroska single-layer and multilayer HEVC + AVC in-band opening and post-rewind/seek_to_sync(0) bitstream_colour() and bitstream_hdr().
- FVID-0572: MP4 SingleLayerMp4VideoReader, SingleView (multilayer HEVC), and MixedHevc in-band opening and post-rewind bitstream_colour(), active_colour(), and bitstream_hdr().
- FVID-0573: media_probe::matroska AVC StreamInfo.profile and StreamInfo.level extraction.
- FVID-0574: Matroska A_AC3 and A_EAC3 WebmAudioReader::duration() fallback when Segment/Info/Duration is omitted.
No external codec, FFmpeg, network, or private media.
"""
import json
import struct
from pathlib import Path

from avc_fixture_mp4 import mux as avc_mux
from generate_avc_colour_formats import avc_configuration, make_pcm_packets, pack_planar_oracle
from generate_avc_parameter_prefix_order import parameters as avc_parameters
from generate_eac3_bsid import identify
from generate_eac3_playback import build_blksw_frame
from generate_ffv1_gray_fixtures import element, uint
from generate_hevc_colour_plane_fixtures import child, escape
from generate_hevc_hdr_seek import HDR, packet as length_packet, sei
from generate_hevc_inband_layered_dispatch import configuration, inventory, ROOT
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import coloured, frame as vp9_frame, mp4 as vp9_mp4
from hevc_fixture_mp4 import mux as hevc_mux

OUT = ROOT / 'inband-open-signal-and-ac3-duration'


def atom(k, b):
    return struct.pack('>I', len(b) + 8) + k + b


def word(*v):
    return b''.join(struct.pack('>I', n) for n in v)


def sample_entry(data):
    for k in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
        data = child(data, k)
    k, p = next(atoms(child(data, b'stsd')[8:]))
    return atom(k, p)


def mkv_video_movie(codec, private, packets, size, keys):
    video = uint(0xB0, size[0]) + uint(0xBA, size[1])
    track = (
        uint(0xD7, 1)
        + uint(0x83, 1)
        + element(0x86, codec.encode())
        + element(0x63A2, private)
        + uint(0x23E383, 40_000_000)
        + element(0xE0, video)
    )
    clusters = [
        element(
            0x1F43B675,
            uint(0xE7, i * 40)
            + element(0xA3, b'\x81\0\0' + (b'\x80' if i in keys else b'\0') + pkt),
        )
        for i, pkt in enumerate(packets)
    ]
    return element(0x1A45DFA3, element(0x4282, b'matroska')) + element(
        0x18538067,
        element(0x1549A966, uint(0x2AD7B1, 1_000_000))
        + element(0x1654AE6B, element(0xAE, track))
        + b''.join(clusters),
    )


def mkv_dolby_without_segment_duration(packets, codec, rate=48000, channels=2, default_duration_ns=None):
    video = (
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 1)
        + element(0x86, b'V_VP9')
        + element(0xE0, uint(0xB0, 16) + uint(0xBA, 16))
    )
    audio = (
        uint(0xD7, 2)
        + uint(0x73C5, 2)
        + uint(0x83, 2)
        + element(0x86, codec.encode())
        + (uint(0x23E383, default_duration_ns) if default_duration_ns is not None else b'')
        + element(0xE1, element(0xB5, struct.pack('>d', rate)) + uint(0x9F, channels))
    )
    vp9 = vp9_frame(8, '420', 16, 16)
    step_ms = 32
    clusters = b''.join(
        element(
            0x1F43B675,
            uint(0xE7, i * step_ms)
            + element(0xA3, b'\x81\0\0\x80' + vp9)
            + element(0xA3, b'\x82\0\0\x80' + p),
        )
        for i, p in enumerate(packets)
    )
    # Deliberately omit Segment/Info/Duration (0x4489).
    return element(0x1A45DFA3, element(0x4282, b'matroska')) + element(
        0x18538067,
        element(0x1549A966, uint(0x2AD7B1, 1_000_000))
        + element(0x1654AE6B, element(0xAE, video) + element(0xAE, audio))
        + clusters,
    )


def build_hdr_sei(hdr_dict):
    master = struct.pack(
        '>8HII',
        *[v for p in hdr_dict['primaries'] + [hdr_dict['white']] for v in p],
        hdr_dict['maximum'],
        hdr_dict['minimum'],
    )
    light = struct.pack('>HH', hdr_dict['cll'], hdr_dict['fall'])
    return sei([(137, master), (144, light)])


def build_layer_hdr_sei(hdr_dict, layer_id):
    base_sei = build_hdr_sei(hdr_dict)
    head = (39 << 9) | (layer_id << 3) | 1
    return head.to_bytes(2, 'big') + base_sei[2:]


def generate():
    files = {}

    # 1. AVC with VUI-less avcC and in-band SPS VUI override (full_range=True, primaries=9, transfer=16, matrix=9)
    avc_cfg_no_vui = avc_configuration(3, 8, 8, False, vui=None, level=51)
    avc_cfg_vui = avc_configuration(3, 8, 8, False, vui=(True, 9, 16, 9), level=51)
    vui_sps, vui_pps = avc_parameters(avc_cfg_vui)
    avc_packets, avc_planes = make_pcm_packets(3, 8, 8)
    avc_inband_packets = [
        length_packet([vui_sps, vui_pps]) + avc_packets[0],
        avc_packets[1],
    ]
    avc_frames_oracle = []
    for i, planes in enumerate(avc_planes):
        values, _, _, _, _ = pack_planar_oracle(3, 8, 8, [0] * 4, planes)
        fname = f'avc-frame-{i}.raw'
        files[fname] = bytes(values)
        avc_frames_oracle.append(fname)

    files['avc-inband-vui.mkv'] = mkv_video_movie(
        'V_MPEG4/ISO/AVC', avc_cfg_no_vui, avc_inband_packets, [32, 16], {0}
    )
    files['avc-inband-vui.mp4'] = avc_mux(
        avc_cfg_no_vui,
        [(i, i == 0, p) for i, p in enumerate(avc_inband_packets)],
        32,
        16,
        25,
        inband_parameters=True,
    )

    # 2. Single-layer HEVC with VUI-less hvcC and in-band Identity RGB SPS (primaries=1, transfer=13, matrix=0, full_range=True) + HDR SEI in sample 0
    base_cfg, base_pictures, base_size = inventory(ROOT / 'hevc-pcm-444-reference-rext8.mp4')
    rgb_hvcc = (ROOT / 'hevc-identity-rgb-8-full.hvcc').read_bytes()
    rgb_sps = configuration(rgb_hvcc)[33][0]
    base_params = configuration(base_cfg)
    hdr0 = HDR[0]
    hdr0_sei = build_hdr_sei(hdr0)

    hevc_groups = [
        [base_params[32][0], rgb_sps, base_params[34][0], hdr0_sei, *base_pictures[0]],
        list(base_pictures[1]),
        list(base_pictures[2]),
    ]
    base_prefix = [n for k in (32, 33, 34) for n in base_params[k]]
    hevc_stream = b''.join(
        b'\0\0\0\1' + n for n in base_prefix + [n for g in hevc_groups for n in g]
    )
    files['hevc-single-inband.mp4'] = hevc_mux(
        hevc_stream,
        8,
        *base_size,
        chroma_format=3,
        chroma_depth=8,
        access_units=hevc_groups,
        inband_parameters=True,
    )
    files['hevc-single-inband.mkv'] = mkv_video_movie(
        'V_MPEGH/ISO/HEVC',
        base_cfg,
        [length_packet(g) for g in hevc_groups],
        base_size,
        {0, 1, 2},
    )

    # 3. Multilayer HEVC (SingleView in MP4, Layered in Matroska, and MixedHevc in MP4) with in-band SPS VUI + layer-3 HDR SEI in sample 0
    multi_cfg, multi_groups, multi_size = inventory(
        ROOT / 'hevc-layered-reconstruct-pcm-dependent.mp4'
    )
    multi_params = configuration(multi_cfg)
    # Obtain a 64x64 4:2:0 base SPS with VUI by rewriting base SPS from hevc-pcm-reference-rext8 via the same bit pattern as hevc-identity-rgb-8-full
    # Wait: hevc-pcm-reference-rext8 is 64x64 4:2:0 8-bit, and hevc-layered-reconstruct-pcm-dependent uses hevc-pcm-reference-rext8's base SPS!
    # Let's check if we can also test layer-3 HDR SEI + empty/base-only hvcC in multilayer HEVC!
    hdr1 = HDR[1]
    layer3_sei = build_layer_hdr_sei(hdr1, 3)
    multi_full = [n for k in (32, 33, 34) for n in multi_params[k]]
    multi_inband_groups = [
        [*multi_full, layer3_sei, *multi_groups[0]],
    ]
    multi_stream = b''.join(
        b'\0\0\0\1' + n
        for n in multi_full + [n for g in multi_inband_groups for n in g]
    )
    files['hevc-layered-inband.mp4'] = hevc_mux(
        multi_stream,
        8,
        *multi_size,
        chroma_format=1,
        chroma_depth=8,
        access_units=multi_inband_groups,
        multilayer=True,
        inband_parameters=True,
        empty_configuration=True,
    )
    empty_hvcc = child(
        sample_entry(files['hevc-layered-inband.mp4'])[8 + 78:], b'hvcC'
    )
    files['hevc-layered-inband.mkv'] = mkv_video_movie(
        'V_MPEGH/ISO/HEVC',
        empty_hvcc,
        [length_packet(g) for g in multi_inband_groups],
        multi_size,
        {0},
    )

    # 4. MixedHevc in MP4: description 1 is multilayer hev1 (with empty hvcC and in-band parameters + layer3_sei in sample 0), description 2 is VP9
    vp_pkt, vp_oracle = coloured(8, '420', 64)
    vcontrol = vp9_mp4([vp_pkt], 8, '420', (64, 64), [1])
    ve = sample_entry(vcontrol)
    he = sample_entry(files['hevc-layered-inband.mp4'])
    mixed_packets = [length_packet(multi_inband_groups[0]), vp_pkt]
    mixed_ids = [1, 2]
    template = dict(atoms(vp9_mp4([vp_pkt] * 2, 8, '420', (64, 64), [1, 2])))
    ftyp = atom(b'ftyp', template[b'ftyp'])
    at = len(ftyp) + 8
    offsets = []
    for p in mixed_packets:
        offsets.append(at)
        at += len(p)

    def rewrite_mixed(data):
        out = b''
        for k, b in atoms(data):
            if k in [b'trak', b'mdia', b'minf', b'stbl']:
                b = rewrite_mixed(b)
            elif k == b'stsd':
                b = word(0, 2) + he + ve
            elif k == b'stsz':
                b = word(0, 0, 2, *map(len, mixed_packets))
            elif k == b'stsc':
                b = word(0, 2, 1, 1, 1, 2, 1, 2)
            elif k == b'stco':
                b = word(0, 2, *offsets)
            out += atom(k, b)
        return out

    files['hevc-mixed-inband.mp4'] = (
        ftyp
        + atom(b'mdat', b''.join(mixed_packets))
        + atom(b'moov', rewrite_mixed(template[b'moov']))
    )
    files['vp9-64.yuv'] = bytes(vp_oracle[::2])

    # 5. Matroska A_AC3 and A_EAC3 without Segment/Info/Duration
    ac3_raw = (ROOT.parent / 'audio/ac3-51.ac3').read_bytes()
    ac3_packets = [ac3_raw[i * 1792 : (i + 1) * 1792] for i in range(3)]
    files['ac3-no-segment-duration-default.mkv'] = mkv_dolby_without_segment_duration(
        ac3_packets, 'A_AC3', rate=48000, channels=6, default_duration_ns=32_000_000
    )
    files['ac3-no-segment-duration-gap.mkv'] = mkv_dolby_without_segment_duration(
        ac3_packets, 'A_AC3', rate=48000, channels=6, default_duration_ns=None
    )

    eac3_frame = identify(build_blksw_frame([(0, 0)] * 6), 16)
    eac3_packets = [eac3_frame] * 3
    files['eac3-no-segment-duration.mkv'] = mkv_dolby_without_segment_duration(
        eac3_packets, 'A_EAC3', rate=48000, channels=2, default_duration_ns=32_000_000
    )

    manifest = {
        'avc_frames': avc_frames_oracle,
        'hdr0': hdr0,
        'hdr1': hdr1,
        'provenance': (
            'Owned synthetic AVC/HEVC/VP9/AC-3/E-AC-3 packets and pixel oracles; '
            'offline muxing without FFmpeg, external codecs, private media, or network.'
        ),
    }
    files['manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    return files


if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, content in generate().items():
        (OUT / name).write_bytes(content)
