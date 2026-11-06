#!/usr/bin/env python3
"""Offline generator for Batch 570 (FVID-0580..0584) reproducers:
- FVID-0580: Matroska owned_video_decode and owned_ffv1_export container Colour fallback when AVC/HEVC VUI reports unspecified code 2 (primaries/transfer/matrix) or omits colour_description_present_flag.
- FVID-0581: ColourDescription::filled_with fallback when primary statement reports H.273 code 2 (unspecified) alongside explicit fallback codes.
- FVID-0582: native_lossless::write_mp4_selected preservation of bitstream VUI colour and HDR10 SEI metadata when MP4 lacks colr/mdcv/ccll boxes.
- FVID-0583: fvid_media::plain_subtitle and parse_subtitle_text support for \\h non-breaking space, HTML tags, non-backslash {comment} blocks, SSA Marked=N payloads, and custom [Events] Format: / case-insensitive Dialogue:.
- FVID-0584: owned_subtitle_burn::supports, plan_burn_subtitles, and burn_subtitles support for external .vtt (WebVTT) and .ass/.ssa (ASS/SSA) subtitle files.
No external codec, FFmpeg, network, or private media.
"""
import json
import struct
from pathlib import Path

from avc_fixture_mp4 import mux as avc_mux
from generate_avc_colour_formats import avc_configuration, make_pcm_packets
from generate_ffv1_gray_fixtures import element, uint
from generate_hevc_inband_layered_dispatch import ROOT
from generate_matroska_subtitles_and_mp4_vui_fallback import inject_colr_nclx

OUT = ROOT / 'mkv-vui-fallback-and-subtitle-burn'


def mkv_avc_video(avcc, packets, width, height, colour_codes=None):
    """Build a synthetic single-track V_MPEG4/ISO/AVC Matroska file with optional Colour element."""
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
        + element(0x86, b'V_MPEG4/ISO/AVC')
        + element(0x63A2, avcc)
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

    # 1. Matroska AVC with VUI unspecified colour codes (primaries=2, transfer=16, matrix=2, full_range=True)
    # and container Colour element (matrix=9 BT.2020nc, transfer=13, primaries=9 BT.2020, range=1 studio).
    # For FVID-0580: owned_video_decode and owned_ffv1_export must preserve matrix=9 and primaries=9 from container
    # while keeping transfer=16 and full_range=True from VUI.
    avc_cfg_vui2 = avc_configuration(3, 8, 8, False, (True, 2, 16, 2), level=51)
    avc_packets, _ = make_pcm_packets(3, 8, 8)
    files['avc-vui2-container-colour.mkv'] = mkv_avc_video(
        avc_cfg_vui2,
        avc_packets,
        32,
        16,
        colour_codes=(9, 13, 9, 1),
    )

    # 2. MP4 AVC with VUI explicit colour (primaries=9, transfer=16, matrix=9, full_range=True)
    # and NO colr atom in the MP4 container.
    # For FVID-0582: native_lossless::write_mp4_selected (fvid::media::transcode_lossless) must
    # write ColourDescription { primaries: 9, transfer: 16, matrix: 9, full_range: true } to the output MKV.
    avc_cfg_vui_only = avc_configuration(3, 8, 8, False, (True, 9, 16, 9), level=51)
    files['avc-vui-only-no-colr.mp4'] = avc_mux(
        avc_cfg_vui_only,
        [(i, i == 0, pkt) for i, pkt in enumerate(avc_packets)],
        32,
        16,
        25,
    )

    # 3. MP4 AVC with container colr nclx having unspecified primaries=2 and matrix=2, transfer=16, full_range=False,
    # and bitstream VUI having primaries=9, transfer=2, matrix=9, full_range=True.
    # For FVID-0581: NativeReader::colour() (which calls container.colour.filled_with(bitstream_colour()))
    # and native_lossless::write_mp4_selected must resolve primaries=9, transfer=16, matrix=9, full_range=False.
    avc_cfg_fill = avc_configuration(3, 8, 8, False, (True, 9, 2, 9), level=51)
    avc_raw_fill = avc_mux(
        avc_cfg_fill,
        [(i, i == 0, pkt) for i, pkt in enumerate(avc_packets)],
        32,
        16,
        25,
    )
    files['avc-colr-vui-cross-fill.mp4'] = inject_colr_nclx(
        avc_raw_fill, primaries=2, transfer=16, matrix=2, full_range=False
    )

    # 4. Synthetic 3-frame Y4M (96x64, 2 fps, 0..500ms, 500..1000ms, 1000..1500ms) + .vtt and .ass/.ssa subtitles
    # for FVID-0583 and FVID-0584.
    y4m_header = b'YUV4MPEG2 W96 H64 F2:1 Ip C420\n'
    frame_bytes = b'FRAME\n' + bytes([32] * (96 * 64) + [64] * (2 * 48 * 32))
    files['burn-source.y4m'] = y4m_header + frame_bytes * 3

    files['burn-cues.vtt'] = (
        'WEBVTT\n\n'
        'NOTE comment block\n\n'
        'cue-1 00:00.500 --> 00:01.000 line:80%\n'
        '<b>VTT</b>\\hBurn\n'
    ).encode('utf-8')

    files['burn-cues.ass'] = (
        '[Script Info]\n'
        'ScriptType: v4.00+\n\n'
        '[Events]\n'
        'Format: Style, Start, End, Layer, Name, MarginL, MarginR, MarginV, Effect, Text\n'
        'dialogue: Default, 0:00:00.50, 0:00:01.00, 0, Alice, 0, 0, 0, , {comment}<i>ASS</i>\\hBurn\n'
    ).encode('utf-8')

    files['burn-cues.ssa'] = (
        '[Script Info]\n'
        'ScriptType: v4.00\n\n'
        '[Events]\n'
        'Format: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n'
        'Dialogue: Marked=0, 0:00:00.50, 0:00:01.00, Default, Bob, 0, 0, 0, , SSA\\hBurn\n'
    ).encode('utf-8')

    manifest = {
        'provenance': (
            'Owned synthetic Matroska/MP4 AVC colour fallback fixtures and Y4M + '
            'WebVTT/ASS/SSA subtitle burn-in fixtures; no FFmpeg, external codecs, '
            'private media, or network.'
        ),
    }
    files['manifest.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    return files


if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, content in generate().items():
        (OUT / name).write_bytes(content)
