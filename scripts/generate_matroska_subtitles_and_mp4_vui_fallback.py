#!/usr/bin/env python3
"""Offline generator for Batch 569 (FVID-0575..0579) reproducers:
- FVID-0575: Matroska owned_webm_probe and media_probe codec name normalization for Dolby (A_AC3*, A_EAC3) and text subtitles (S_TEXT/UTF8, S_TEXT/ASCII, S_UTF8, S_TEXT/ASS, S_ASS, S_TEXT/SSA, S_SSA, S_TEXT/WEBVTT).
- FVID-0576: Matroska subtitle conversion and planning with DefaultDuration fallback (SimpleBlock without BlockDuration) and S_TEXT/SSA (including Marked=0 normalization).
- FVID-0577: WebmSubtitleReader support for S_TEXT/WEBVTT, S_TEXT/ASCII, S_UTF8, S_ASS, and S_SSA.
- FVID-0578: ASS/SSA sidecar parse_ass custom [Events] Format: column order, case-insensitive Dialogue:, and \\h hard-space escape in subtitles::plain.
- FVID-0579: MP4 owned_mp4_video_decode container colr (nclx) fallback when AVC or HEVC VUI colour_description_present_flag=1 reports unspecified code 2.
No external codec, FFmpeg, network, or private media.
"""
import json
import struct
from pathlib import Path

from avc_fixture_mp4 import mux as avc_mux
from generate_avc_colour_formats import avc_configuration, make_pcm_packets
from generate_ffv1_gray_fixtures import element, uint
from generate_hevc_colour_plane_fixtures import bits, escape, packed, unescape
from generate_hevc_hdr_seek import packet as length_packet
from generate_hevc_inband_layered_dispatch import configuration, inventory, ROOT
from generate_mp4_fragment_description_regressions import atoms
from hevc_fixture_mp4 import mux as hevc_mux

OUT = ROOT / 'matroska-subtitles-and-mp4-vui-fallback'


def atom(k, b):
    return struct.pack('>I', len(b) + 8) + k + b


def inject_colr_nclx(mp4_bytes, primaries, transfer, matrix, full_range):
    nclx = atom(
        b'colr',
        b'nclx'
        + struct.pack('>HHH', primaries, transfer, matrix)
        + bytes([0x80 if full_range else 0x00]),
    )

    def rewrite(data):
        out = b''
        for k, b in atoms(data):
            if k in [b'moov', b'trak', b'mdia', b'minf', b'stbl']:
                b = rewrite(b)
            elif k == b'stsd':
                header = b[:8]
                entries = b''
                for ek, eb in atoms(b[8:]):
                    entries += atom(ek, eb + nclx)
                b = header + entries
            out += atom(k, b)
        return out

    return rewrite(mp4_bytes)


def replace_hevc_sps_vui_codes(rgb_sps, primaries, transfer, matrix):
    rbsp = bytearray(unescape(rgb_sps[2:]))
    bstr = bits(rbsp)
    stop = bstr.rfind('1')
    bstr = bstr[: stop + 1]
    pattern = (
        '1'
        + '001101'
        + '1'
        + '1'
        + f'{1:08b}{13:08b}{0:08b}'
        + '0000000'
    )
    pos = bstr.rfind(pattern)
    assert pos >= 0, 'expected VUI pattern in rgb_sps'
    replacement = (
        '1'
        + '001101'
        + '1'
        + '1'
        + f'{primaries:08b}{transfer:08b}{matrix:08b}'
        + '0000000'
    )
    new_bstr = bstr[:pos] + replacement + bstr[pos + len(pattern) :]
    return rgb_sps[:2] + escape(packed(new_bstr))


def mkv_subtitle_container(tracks_spec, packets_spec):
    """Build a synthetic Matroska file with one or more subtitle tracks.
    tracks_spec: list of dicts {number, codec, name, language, private, default_duration_ns}
    packets_spec: list of dicts {track, pts_ms, duration_ms (None for SimpleBlock), payload}
    """
    track_entries = b''
    for t in tracks_spec:
        entry = (
            uint(0xD7, t['number'])
            + uint(0x73C5, t['number'])
            + uint(0x83, 17)
            + element(0x86, t['codec'].encode())
        )
        if t.get('name'):
            entry += element(0x536E, t['name'].encode())
        if t.get('language'):
            entry += element(0x22B59C, t['language'].encode())
        if t.get('private'):
            entry += element(0x63A2, t['private'])
        if t.get('default_duration_ns'):
            entry += uint(0x23E383, t['default_duration_ns'])
        track_entries += element(0xAE, entry)

    clusters = b''
    for p in packets_spec:
        vint_track = bytes([0x80 | p['track']])
        block_body = vint_track + struct.pack('>h', 0) + b'\x80' + p['payload']
        if p.get('duration_ms') is None:
            cluster_body = uint(0xE7, p['pts_ms']) + element(0xA3, block_body)
        else:
            bg = element(0xA1, block_body) + uint(0x9B, p['duration_ms'])
            cluster_body = uint(0xE7, p['pts_ms']) + element(0xA0, bg)
        clusters += element(0x1F43B675, cluster_body)

    return element(0x1A45DFA3, element(0x4282, b'matroska')) + element(
        0x18538067,
        element(0x1549A966, uint(0x2AD7B1, 1_000_000))
        + element(0x1654AE6B, track_entries)
        + clusters,
    )


def generate():
    files = {}

    # 1. Matroska multi-track subtitle fixture for FVID-0575 and FVID-0577:
    # Tracks:
    # 1: S_TEXT/WEBVTT (with cue identifier + timing/settings line in block 0, and two blank header lines in block 1)
    # 2: S_TEXT/ASCII
    # 3: S_UTF8
    # 4: S_ASS
    # 5: S_SSA
    ass_header = (
        b'[Script Info]\nScriptType: v4.00+\n'
        b'[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n'
        b'Style: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n'
        b'[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n'
    )
    ssa_header = (
        b'[Script Info]\nScriptType: v4.00\n'
        b'[V4 Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, TertiaryColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, AlphaLevel, Encoding\n'
        b'Style: Default,Arial,16,16777215,255,0,0,0,0,1,1,0,2,10,10,10,0,1\n'
        b'[Events]\nFormat: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n'
    )
    files['mkv-subtitle-codecs.mkv'] = mkv_subtitle_container(
        [
            {'number': 1, 'codec': 'S_TEXT/WEBVTT', 'private': b'WEBVTT\n\n'},
            {'number': 2, 'codec': 'S_TEXT/ASCII'},
            {'number': 3, 'codec': 'S_UTF8'},
            {'number': 4, 'codec': 'S_ASS', 'private': ass_header},
            {'number': 5, 'codec': 'S_SSA', 'private': ssa_header},
        ],
        [
            {
                'track': 1,
                'pts_ms': 100,
                'duration_ms': 800,
                'payload': b'cue-1\n00:00:00.100 --> 00:00:00.900 line:80%\nWebVTT\\hfirst',
            },
            {
                'track': 1,
                'pts_ms': 1000,
                'duration_ms': 700,
                'payload': b'\n\n<b>WebVTT</b> second',
            },
            {
                'track': 2,
                'pts_ms': 200,
                'duration_ms': 600,
                'payload': b'ASCII line',
            },
            {
                'track': 3,
                'pts_ms': 300,
                'duration_ms': 600,
                'payload': b'Legacy UTF8 line',
            },
            {
                'track': 4,
                'pts_ms': 400,
                'duration_ms': 600,
                'payload': b'0,0,Default,,0,0,0,,Legacy\\hASS',
            },
            {
                'track': 5,
                'pts_ms': 500,
                'duration_ms': 600,
                'payload': b'0,Marked=0,Default,,0,0,0,,Legacy\\hSSA',
            },
        ],
    )

    # 2. Matroska subtitle conversion fixtures for FVID-0576:
    # 2a: S_TEXT/UTF8 with SimpleBlock (no BlockDuration) and DefaultDuration = 750_000_000 ns
    files['utf8-default-duration.mkv'] = mkv_subtitle_container(
        [
            {
                'number': 1,
                'codec': 'S_TEXT/UTF8',
                'name': 'DefaultDurSRT',
                'language': 'eng',
                'default_duration_ns': 750_000_000,
            }
        ],
        [
            {'track': 1, 'pts_ms': 200, 'duration_ms': None, 'payload': b'First default cue'},
            {'track': 1, 'pts_ms': 1200, 'duration_ms': None, 'payload': b'Second default cue'},
        ],
    )

    # 2b: S_TEXT/SSA with SimpleBlock (no BlockDuration), DefaultDuration = 900_000_000 ns, and Marked=0 in Layer field
    files['ssa-default-duration.mkv'] = mkv_subtitle_container(
        [
            {
                'number': 1,
                'codec': 'S_TEXT/SSA',
                'name': 'DefaultDurSSA',
                'language': 'jpn',
                'private': ass_header,
                'default_duration_ns': 900_000_000,
            }
        ],
        [
            {
                'track': 1,
                'pts_ms': 300,
                'duration_ms': None,
                'payload': b'0,Marked=0,Default,Alice,0,0,0,,SSA\\hconverted\\Nline',
            },
            {
                'track': 1,
                'pts_ms': 1500,
                'duration_ms': None,
                'payload': b'1,Marked=1,Default,Bob,0,0,0,,Second SSA cue',
            },
        ],
    )

    # 3. Sidecar ASS/SSA with custom Format: column order, case-insensitive dialogue :, and \h hard space for FVID-0578
    files['custom-format-and-hardspace.ass'] = (
        '[Script Info]\n'
        'ScriptType: v4.00+\n'
        '\n'
        '[Events]\n'
        'Format: Style, Start, End, Layer, Name, MarginL, MarginR, MarginV, Effect, Text\n'
        'dialogue: Default, 0:00:00.25, 0:00:01.50, 0, Alice, 0, 0, 0, , Hard\\hspace\\Nand{\\i1}italics{\\i0}\n'
        'Dialogue : Alt, 0:00:02.00, 0:00:03.25, 0, Bob, 0, 0, 0, , Second\\hcue, with comma\n'
    ).encode('utf-8')

    # 4. MP4 AVC and HEVC with VUI unspecified colour code 2 and container colr (nclx) for FVID-0579
    # AVC: VUI has full_range=True, primaries=2 (unspecified), transfer=16 (PQ), matrix=2 (unspecified).
    # Container colr nclx has primaries=9 (BT.2020), transfer=13, matrix=9 (BT.2020nc), full_range=False.
    # Expected merged colour: full_range=True (from VUI), primaries=9 (from container fallback), transfer=16 (from VUI), matrix=9 (from container fallback).
    avc_cfg = avc_configuration(3, 8, 8, False, (True, 2, 16, 2), level=51)
    avc_packets, _ = make_pcm_packets(3, 8, 8)
    avc_raw_mp4 = avc_mux(
        avc_cfg,
        [(i, i == 0, pkt) for i, pkt in enumerate(avc_packets)],
        32,
        16,
        25,
    )
    files['avc-vui-unspecified-colr.mp4'] = inject_colr_nclx(
        avc_raw_mp4, primaries=9, transfer=13, matrix=9, full_range=False
    )

    # HEVC: VUI has full_range=True, primaries=2 (unspecified), transfer=2 (unspecified), matrix=9 (BT.2020nc).
    # Container colr nclx has primaries=9 (BT.2020), transfer=16 (PQ), matrix=1 (BT.709), full_range=False.
    # Expected merged colour: full_range=True (from VUI), primaries=9 (from container fallback), transfer=16 (from container fallback), matrix=9 (from VUI).
    base_cfg, base_pictures, base_size = inventory(ROOT / 'hevc-pcm-444-reference-rext8.mp4')
    rgb_hvcc = (ROOT / 'hevc-identity-rgb-8-full.hvcc').read_bytes()
    rgb_sps = configuration(rgb_hvcc)[33][0]
    hevc_sps_vui2 = replace_hevc_sps_vui_codes(rgb_sps, 2, 2, 9)
    base_params = configuration(base_cfg)
    hevc_groups = [list(base_pictures[0]), list(base_pictures[1])]
    hevc_prefix = [base_params[32][0], hevc_sps_vui2, base_params[34][0]]
    hevc_stream = b''.join(
        b'\0\0\0\1' + n for n in hevc_prefix + [n for g in hevc_groups for n in g]
    )
    hevc_raw_mp4 = hevc_mux(
        hevc_stream,
        8,
        *base_size,
        chroma_format=3,
        chroma_depth=8,
        access_units=hevc_groups,
    )
    files['hevc-vui-unspecified-colr.mp4'] = inject_colr_nclx(
        hevc_raw_mp4, primaries=9, transfer=16, matrix=1, full_range=False
    )

    manifest = {
        'provenance': (
            'Owned synthetic Matroska subtitle tracks, ASS sidecar, and AVC/HEVC MP4 '
            'tracks with VUI code 2 + nclx colr boxes; no FFmpeg, external codecs, '
            'private media, or network.'
        ),
    }
    files['manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    return files


if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, content in generate().items():
        (OUT / name).write_bytes(content)
