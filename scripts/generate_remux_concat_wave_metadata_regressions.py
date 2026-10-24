#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 237 (`FVID-0244`).

Covers set-then-delete metadata precedence, case-insensitive tag removal, and
uppercase Matroska custom tag normalization across:
1. `owned_mp4_remux` (`remux` / `plan_remux`) and `src/container/mp4_concat.rs`
   (`write_with_options` / `concat_mp4_matroska_with_options`).
2. `owned_matroska_adts_impl::extract_adts_matroska_metadata` (`owned_adts_remux`,
   `owned_concat`, `fvid::media::remux`, `fvid::media::concat`).
3. `src/native_export/pcm_matroska.rs::parse_matroska_pcm_metadata` (`decode_audio`,
   `mix_audio_with_controls`, `merge_audio_with_controls`, `concat_audio_with_options`).
4. `crates/fvid-media/src/owned_wave_metadata.rs::edit_info_chunks` (WAVE `LIST`/`INFO`
   chunk editing across `owned_wave_remux`, `owned_audio_export`, `owned_loudnorm`,
   `native_export`, `native_audio_mix`, `native_loudnorm`, and `native_pcm::normalize`).
"""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct

from generate_mp4_audio_selection_regressions import (
    build_aac_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_rotated_mp4_decode_export_probe_regressions import (
    build_avc_32x16_ipcm,
    build_rotated_avc_video_track,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/remux-concat-wave-metadata-precedence"


def build_adts_mono_silence(frames: int = 2) -> bytes:
    fields = [
        (0, 3),
        (0, 4),
        (100, 8),
        (0, 1),
        (0, 2),
        (0, 1),
        (0, 6),
        (0, 1),
        (0, 1),
        (0, 1),
        (0, 1),
        (7, 3),
    ]
    bits = "".join(format(value, f"0{width}b") for value, width in fields)
    bits += "0" * (-len(bits) % 8)
    payload = int(bits, 2).to_bytes(len(bits) // 8, "big")
    size = 7 + len(payload)
    header = bytes(
        [
            0xFF,
            0xF1,
            0x50,
            0x40 | (size >> 11),
            (size >> 3) & 255,
            ((size & 7) << 5) | 31,
            0xFC,
        ]
    )
    return (header + payload) * frames


def build_wave_s16le_with_info(
    sample_rate: int,
    channels: int,
    frames: int,
    freq: float,
    info_entries: list[tuple[bytes, bytes]],
) -> bytes:
    pcm = bytearray()
    for frame in range(frames):
        t = frame / sample_rate
        val = int(round(10000.0 * math.sin(2.0 * math.pi * freq * t)))
        sample = struct.pack("<h", max(-32768, min(32767, val)))
        for _ in range(channels):
            pcm.extend(sample)

    block_align = channels * 2
    byte_rate = sample_rate * block_align
    fmt_chunk = struct.pack(
        "<4sIHHIIHH",
        b"fmt ",
        16,
        1,
        channels,
        sample_rate,
        byte_rate,
        block_align,
        16,
    )
    data_chunk = struct.pack("<4sI", b"data", len(pcm)) + bytes(pcm)
    if len(pcm) % 2 == 1:
        data_chunk += b"\x00"

    info_body = bytearray(b"INFO")
    for tag, value in info_entries:
        assert len(tag) == 4
        raw = value + b"\x00"
        info_body.extend(tag)
        info_body.extend(struct.pack("<I", len(raw)))
        info_body.extend(raw)
        if len(raw) % 2 == 1:
            info_body.append(0)
    list_chunk = struct.pack("<4sI", b"LIST", len(info_body)) + bytes(info_body)
    if len(info_body) % 2 == 1:
        list_chunk += b"\x00"

    riff_body = b"WAVE" + fmt_chunk + data_chunk + list_chunk
    return struct.pack("<4sI", b"RIFF", len(riff_body)) + riff_body


def build_av_mp4_segment(avcc: bytes, avc_packets: list[bytes]) -> bytes:
    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    aac_packets = [build_aac_packet(100), build_aac_packet(100)]
    v_payload = b"".join(avc_packets)
    a_payload = b"".join(aac_packets)
    mdat = atom(b"mdat", v_payload + a_payload)

    v_offset = len(ftyp) + 8
    a_offset = v_offset + len(v_payload)

    asc = b"\x11\x88"
    mp4a_payload = mp4a_v0_header(1, 48000) + atom(b"esds", build_esds(asc))

    v_trak = build_rotated_avc_video_track(
        1, avcc, avc_packets, v_offset, rotation=0, pasp=(1, 1), rate=48000, delta=1024
    )
    a_trak = make_audio_track(2, 3, b"mp4a", mp4a_payload, aac_packets, a_offset, 48000, 1024)

    mvhd = bytearray(100)
    mvhd[12:16] = word(48000)
    mvhd[16:20] = word(2048)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 3)]:
        mvhd[at : at + 4] = word(val)
    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + v_trak + a_trak)


def generate() -> dict[str, bytes]:
    avcc, avc_packets = build_avc_32x16_ipcm()
    files = {
        "avc-aac-segment.mp4": build_av_mp4_segment(avcc, avc_packets),
        "adts-mono-a.aac": build_adts_mono_silence(frames=2),
        "adts-mono-b.aac": build_adts_mono_silence(frames=2),
        "tone-info-a.wav": build_wave_s16le_with_info(
            sample_rate=48000,
            channels=2,
            frames=480,
            freq=440.0,
            info_entries=[
                (b"INAM", b"Original Wave Title"),
                (b"IART", b"Original Wave Artist"),
                (b"QWER", b"Opaque Riff Tag"),
            ],
        ),
        "tone-info-b.wav": build_wave_s16le_with_info(
            sample_rate=48000,
            channels=2,
            frames=480,
            freq=660.0,
            info_entries=[
                (b"INAM", b"Second Wave Title"),
                (b"IART", b"Second Wave Artist"),
            ],
        ),
    }
    manifest = {
        "batch_id": "FVID-0244",
        "generator": "scripts/generate_remux_concat_wave_metadata_precedence_regressions.py",
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
