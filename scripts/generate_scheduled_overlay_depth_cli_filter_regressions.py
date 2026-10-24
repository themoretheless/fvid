#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 234 (`FVID-0241`).

Covers:
1. High-depth (`YUV420p10` / `YUV422p10`) and non-`YUV420` (`YUV422` / `YUV440`)
   scheduled Y4M overlay in `owned_y4m_overlay`, `owned_y4m_decode`, and
   `owned_video_decode`, including post-geometry (`--transpose` / `--rotate`)
   format and pixel aspect normalization.
2. Scheduled Y4M overlay on Matroska/WebM video (`V_MPEG4/ISO/AVC`) with a
   non-zero `DefaultDuration` (`400_000_000` ns = 2.5 fps), where
   `owned_video_decode::process_frame_with_overlay` previously left an `F5:2`
   token in `presented.tokens` before appending `F1000000000:1`, causing
   `Header::frame_rate()` to fail with `"duplicate Y4M frame rate"`.
3. Track enable/default disposition preservation in `native_lossless::write_mp4_selected`
   and `native_lossless_y4m::write_selected` for MP4 and Matroska sources with
   disabled/non-default companion audio tracks.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml
from generate_mp4_audio_selection_regressions import (
    build_aac_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_multitrack_avc_mp4_stream_metadata_regressions import (
    build_avc_16x16_ipcm,
    build_avc_video_track_mp4,
    f64_elem,
    u,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/scheduled-overlay-depth-cli-filters"


def pack_u16_le(values: list[int]) -> bytes:
    return b"".join(struct.pack("<H", v) for v in values)


def build_main_420p10_y4m() -> bytes:
    """4x4 10-bit YUV420p10 at 4 fps (3 frames: t=0, 0.25s, 0.50s)."""
    header = b"YUV4MPEG2 W4 H4 F4:1 Ip A1:1 C420p10 XCOLORRANGE=LIMITED\n"
    frames = []
    for idx in range(3):
        y = [100 + idx * 20] * 16
        cb = [512] * 4
        cr = [512] * 4
        frames.append(b"FRAME\n" + pack_u16_le(y + cb + cr))
    return header + b"".join(frames)


def build_overlay_420p8_y4m() -> bytes:
    """2x2 8-bit YUV420 at 2 fps (2 frames: t=0, 0.50s)."""
    header = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\n"
    f0 = b"FRAME\n" + bytes([50, 50, 50, 50, 80, 160])
    f1 = b"FRAME\n" + bytes([200, 200, 200, 200, 90, 170])
    return header + f0 + f1


def build_main_422_y4m() -> bytes:
    """4x2 8-bit YUV422 at 4 fps with A2:1 pixel aspect (2 frames)."""
    header = b"YUV4MPEG2 W4 H2 F4:1 Ip A2:1 C422 XCOLORRANGE=LIMITED\n"
    frames = []
    for idx in range(2):
        y = bytes([30 + idx * 10] * 8)
        cb = bytes([128] * 4)
        cr = bytes([128] * 4)
        frames.append(b"FRAME\n" + y + cb + cr)
    return header + b"".join(frames)


def build_overlay_440_after_transpose_y4m() -> bytes:
    """2x2 8-bit YUV440 at 4 fps with A1:2 pixel aspect (matches transposed 422 A2:1)."""
    header = b"YUV4MPEG2 W2 H2 F4:1 Ip A1:2 C440 XCOLORRANGE=LIMITED\n"
    frames = []
    for idx in range(2):
        y = bytes([180 + idx * 10] * 4)
        cb = bytes([70 + idx] * 2)
        cr = bytes([190 - idx] * 2)
        frames.append(b"FRAME\n" + y + cb + cr)
    return header + b"".join(frames)


def build_avc_default_duration_mkv(avcc: bytes, avc_packets: list[bytes]) -> bytes:
    """16x16 AVC Matroska with DefaultDuration=400_000_000ns (2.5 fps) + disabled Opus track."""
    header = ebml(
        "1a45dfa3",
        ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2),
    )
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000)
        + f64_elem("4489", 800.0)
        + ebml("4d80", b"FVidOverlayClockMux")
        + ebml("5741", b"FVidOverlayClockApp"),
    )
    video_track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 11)
        + u("83", 1)
        + u("b9", 1)
        + u("88", 1)
        + ebml("86", b"V_MPEG4/ISO/AVC")
        + ebml("63a2", avcc)
        + u("23e383", 400_000_000)
        + ebml("536e", b"MainVideo")
        + ebml("22b59c", b"eng")
        + ebml("e0", u("b0", 16) + u("ba", 16) + ebml("55b0", u("55b1", 6) + u("55b9", 1))),
    )
    opus_head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48000, 0, 0)
    # Companion audio track with FlagEnabled=0 and FlagDefault=0
    audio_track = ebml(
        "ae",
        u("d7", 2)
        + u("73c5", 22)
        + u("83", 2)
        + u("b9", 0)
        + u("88", 0)
        + ebml("86", b"A_OPUS")
        + ebml("63a2", opus_head)
        + u("56aa", 0)
        + u("23e383", 20_000_000)
        + ebml("536e", b"DisabledCommentary")
        + ebml("22b59c", b"jpn")
        + ebml("e1", f64_elem("b5", 48000.0) + u("9f", 1)),
    )
    tracks = ebml("1654ae6b", video_track + audio_track)
    opus_pkt = bytes.fromhex("f8fffe")
    cluster = u("e7", 0)
    for idx, v_pkt in enumerate(avc_packets):
        ts = idx * 400
        cluster += ebml("a3", b"\x81" + ts.to_bytes(2, "big") + b"\x80" + v_pkt)
        cluster += ebml("a3", b"\x82" + ts.to_bytes(2, "big") + b"\x80" + opus_pkt)
    return header + ebml("18538067", info + tracks + ebml("1f43b675", cluster))


def build_overlay_16x16_clock_y4m() -> bytes:
    """4x4 8-bit YUV420 overlay at 5 fps (2 frames: t=0, t=0.40s -> indices 0, 2 at 5fps)."""
    header = b"YUV4MPEG2 W4 H4 F5:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\n"
    frames = []
    for val in [32, 96, 210]:
        y = bytes([val] * 16)
        cb = bytes([80] * 4)
        cr = bytes([160] * 4)
        frames.append(b"FRAME\n" + y + cb + cr)
    return header + b"".join(frames)


def build_mp4_with_disabled_audio(avcc: bytes, avc_packets: list[bytes]) -> bytes:
    """MP4 with 16x16 AVC video (enabled), enabled AAC audio, and disabled AAC commentary (tkhd flags=0)."""
    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    aac_packets = [build_aac_packet(100), build_aac_packet(100)]
    v_payload = b"".join(avc_packets)
    a_payload = b"".join(aac_packets)
    mdat = atom(b"mdat", v_payload + a_payload + a_payload)

    v_offset = len(ftyp) + 8
    a1_offset = v_offset + len(v_payload)
    a2_offset = a1_offset + len(a_payload)

    asc = b"\x11\x88"
    mp4a_payload = mp4a_v0_header(1, 48000) + atom(b"esds", build_esds(asc))

    v_trak = build_avc_video_track_mp4(1, avcc, avc_packets, v_offset, 48000, 1024)
    # tkhd_flags = 3 -> track_enabled = true, default = true
    a1_trak = make_audio_track(2, 3, b"mp4a", mp4a_payload, aac_packets, a1_offset, 48000, 1024)
    # tkhd_flags = 0 -> track_enabled = false, default = false
    a2_trak = make_audio_track(3, 0, b"mp4a", mp4a_payload, aac_packets, a2_offset, 48000, 1024)

    mvhd = bytearray(100)
    mvhd[12:16] = word(48000)
    mvhd[16:20] = word(2048)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 4)]:
        mvhd[at : at + 4] = word(val)

    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + v_trak + a1_trak + a2_trak)


def generate() -> dict[str, bytes]:
    avcc, avc_packets = build_avc_16x16_ipcm()
    files = {
        "main-420p10.y4m": build_main_420p10_y4m(),
        "overlay-420p8.y4m": build_overlay_420p8_y4m(),
        "main-422-aspect.y4m": build_main_422_y4m(),
        "overlay-440-transposed.y4m": build_overlay_440_after_transpose_y4m(),
        "avc-default-duration-disabled-opus.mkv": build_avc_default_duration_mkv(avcc, avc_packets),
        "overlay-4x4-5fps.y4m": build_overlay_16x16_clock_y4m(),
        "avc-disabled-aac.mp4": build_mp4_with_disabled_audio(avcc, avc_packets),
    }
    manifest = {
        "batch_id": "FVID-0241",
        "generator": "scripts/generate_scheduled_overlay_depth_cli_filter_regressions.py",
        "files": {
            name: {
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            for name, data in sorted(files.items())
        },
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
