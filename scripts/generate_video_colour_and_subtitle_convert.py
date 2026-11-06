#!/usr/bin/env python3
"""Offline generator for Batch 571 (FVID-0585..0589) reproducers:
- FVID-0585: WebmVideoReader and Mp4VideoReader preservation of container Colour (colr) matrix and full_range fallback and per-frame presented_signal when AVC/HEVC VUI omits colour_description_present_flag or reports code 2 (unspecified).
- FVID-0586: Matroska and MP4 VP9 (CS_UNKNOWN == 0) and AV1 (matrix == 2) container Colour matrix and full_range fallback across WebmVideoReader, Mp4VideoReader, owned_video_decode, and owned_ffv1_export.
- FVID-0587: Standalone .ssa (SubStation Alpha v4.00 with Format: Marked, ... and Dialogue: Marked=N, ...) subtitle conversion and planning in fvid::native_subtitle and fvid_media::owned_subtitle.
- FVID-0588: Standalone .vtt (WebVTT with WEBVTT header, NOTE/STYLE/REGION blocks, cue IDs, short MM:SS.mmm clocks) and Matroska S_TEXT/WEBVTT track subtitle conversion and planning in fvid::native_subtitle and fvid_media::owned_subtitle.
- FVID-0589: subtitles::ass_block case-insensitive Dialogue:/Comment:/Format:/Style: prefix handling for Matroska S_TEXT/ASS and S_TEXT/SSA blocks.
No external codec, FFmpeg, network, or private media.
"""
import json
import struct
from pathlib import Path

from avc_fixture_mp4 import mux as avc_mux
from generate_avc_colour_formats import avc_configuration, make_pcm_packets
from generate_ffv1_gray_fixtures import element, uint
from generate_hevc_inband_layered_dispatch import ROOT
from generate_matroska_subtitles_and_mp4_vui_fallback import (
    inject_colr_nclx,
    mkv_subtitle_container,
)
from generate_mkv_vui_fallback_and_subtitle_burn import mkv_avc_video
from generate_vp9_chroma import coloured, mp4 as vp9_mp4

OUT = ROOT / 'video-colour-and-subtitle-convert'


def vp9_cs_unknown_frame():
    pkt, _ = coloured(8, '420', side=64)
    ba = bytearray(pkt)
    # Clear color_space bits (3 bits starting at bit offset 32 -> byte 4 bits 7..5)
    # and full_range bit (bit offset 35 -> byte 4 bit 4) so color_space == 0 (CS_UNKNOWN)
    # and full_range == False.
    ba[4] &= 0x0F
    return bytes(ba)


def mkv_vp9_video(packets, width, height, colour_codes=None):
    video_meta = uint(0xB0, width) + uint(0xBA, height)
    if colour_codes is not None:
        matrix, transfer, primaries, range_code = colour_codes
        colour_body = b''
        if matrix is not None:
            colour_body += uint(0x55B1, matrix)
        if transfer is not None:
            colour_body += uint(0x55BA, transfer)
        if primaries is not None:
            colour_body += uint(0x55BB, primaries)
        if range_code is not None:
            colour_body += uint(0x55B9, range_code)
        video_meta += element(0x55B0, colour_body)

    track_entry = (
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 1)
        + element(0x86, b'V_VP9')
        + uint(0x23E383, 40_000_000)
        + element(0xE0, video_meta)
    )

    clusters = b''
    for idx, pkt in enumerate(packets):
        block = b'\x81' + struct.pack('>h', 0) + (b'\x80' if idx == 0 else b'\x00') + pkt
        cluster_body = uint(0xE7, idx * 40) + element(0xA3, block)
        clusters += element(0x1F43B675, cluster_body)

    return element(0x1A45DFA3, element(0x4282, b'matroska')) + element(
        0x18538067,
        element(0x1549A966, uint(0x2AD7B1, 1_000_000))
        + element(0x1654AE6B, element(0xAE, track_entry))
        + clusters,
    )


def generate():
    files = {}

    # 1. FVID-0585: Matroska and MP4 AVC with VUI code 2 (unspecified matrix=2, primaries=2, transfer=16, full_range=False)
    # and container Colour/colr specifying BT.2020nc (matrix=9, primaries=9, transfer=16, full_range=True).
    avc_cfg_vui2 = avc_configuration(3, 8, 8, False, (False, 2, 16, 2), level=51)
    avc_packets, _ = make_pcm_packets(3, 8, 8)
    files['avc-vui2-bt2020-full.mkv'] = mkv_avc_video(
        avc_cfg_vui2,
        avc_packets,
        32,
        16,
        colour_codes=(9, 16, 9, 2),
    )
    avc_raw_mp4 = avc_mux(
        avc_cfg_vui2,
        [(i, i == 0, pkt) for i, pkt in enumerate(avc_packets)],
        32,
        16,
        25,
    )
    files['avc-vui2-bt2020-full.mp4'] = inject_colr_nclx(
        avc_raw_mp4, primaries=9, transfer=16, matrix=9, full_range=True
    )

    # 2. FVID-0586: Matroska and MP4 VP9 with CS_UNKNOWN (color_space=0, full_range=False)
    # and container Colour/colr specifying BT.2020nc (matrix=9, primaries=9, transfer=13, full_range=True).
    vp9_pkt = vp9_cs_unknown_frame()
    files['vp9-cs-unknown-bt2020.mkv'] = mkv_vp9_video(
        [vp9_pkt, vp9_pkt],
        64,
        64,
        colour_codes=(9, 13, 9, 2),
    )
    vp9_raw_mp4 = vp9_mp4([vp9_pkt, vp9_pkt], 8, '420', size=(64, 64), sync_samples=[1, 2])
    files['vp9-cs-unknown-bt2020.mp4'] = inject_colr_nclx(
        vp9_raw_mp4, primaries=9, transfer=13, matrix=9, full_range=True
    )

    # 3. FVID-0587: Standalone .ssa (SubStation Alpha v4.00) script with Format: Marked, ... and Dialogue: Marked=0, ...
    files['standalone-v4.ssa'] = (
        '[Script Info]\n'
        'Title: Synthetic SSA v4.00\n'
        'ScriptType: v4.00\n'
        'PlayResX: 384\n'
        'PlayResY: 288\n\n'
        '[V4 Styles]\n'
        'Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, TertiaryColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, AlphaLevel, Encoding\n'
        'Style: Default,Arial,18,16777215,255,0,0,0,0,1,1,0,2,10,10,10,0,1\n\n'
        '[Events]\n'
        'Format: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n'
        'Dialogue: Marked=0,0:00:00.20,0:00:00.90,Default,Alice,0,0,0,,First SSA cue\n'
        'Dialogue: Marked=1,0:00:01.00,0:00:01.80,Default,Bob,0,0,0,,Second, SSA cue\n'
    ).encode('utf-8')

    # 4. FVID-0588: Standalone .vtt (WebVTT) file and Matroska S_TEXT/WEBVTT container
    files['standalone-cues.vtt'] = (
        'WEBVTT - Synthetic WebVTT\n\n'
        'NOTE header comment\n\n'
        'STYLE\n'
        '::cue { color: white; }\n\n'
        'cue-alpha\n'
        '00:00.250 --> 00:00.950 line:80%\n'
        '<b>First</b> WebVTT cue\n\n'
        'inline-beta 00:01.100 --> 00:01.900 align:start\n'
        '<i>Second</i> WebVTT cue\n'
    ).encode('utf-8')

    files['matroska-webvtt.mkv'] = mkv_subtitle_container(
        [
            {
                'number': 1,
                'codec': 'S_TEXT/WEBVTT',
                'name': 'WebVTT captions',
                'language': 'eng',
                'private': b'WEBVTT\n\n',
                'default_duration_ns': 700_000_000,
            }
        ],
        [
            {
                'track': 1,
                'pts_ms': 200,
                'duration_ms': 700,
                'payload': b'cue-1\n00:00.200 --> 00:00.900 line:85%\n<b>Muxed</b> WebVTT one',
            },
            {
                'track': 1,
                'pts_ms': 1000,
                'duration_ms': None,
                'payload': b'\n\n<i>Muxed</i> WebVTT two',
            },
        ],
    )

    manifest = {
        'provenance': (
            'Owned synthetic Matroska/MP4 AVC and VP9 container Colour fallback fixtures '
            'and standalone .ssa / .vtt / Matroska S_TEXT/WEBVTT subtitle conversion fixtures; '
            'no FFmpeg, external codecs, private media, or network.'
        ),
    }
    files['manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    return files


if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, content in generate().items():
        (OUT / name).write_bytes(content)
