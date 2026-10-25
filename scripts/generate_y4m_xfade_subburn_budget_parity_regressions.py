#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 256 (`FVID-0263`).

Covers:
1. Multitrack Matroska FFV1+AAC (`multitrack-ffv1-aac.mkv`, track 0 is 4x4 YUV420 FFV1
   with ~25-byte packets and 24-byte decoded Y4M frames; track 1 is 48 kHz mono AAC with
   512-byte packets) for selected-track Y4M export/concat/trim packet-budget checks and
   single-track subtitle-burn refusal priority.
2. Multitrack MP4 AVC+AAC (`multitrack-avc-aac.mp4`, track 0 is 32x16 AVC I_PCM with
   ~778-byte packets and 768-byte decoded Y4M frames; track 1 is 48 kHz mono AAC with
   2048-byte packets) for selected-track MP4 Y4M export/concat/trim packet-budget checks.
3. Y4M segment & cross-fade aspect/rate/geometry fixtures:
   - `seg-4x4-25fps-par1x1.y4m` (4x4, 25fps, A1:1, C420)
   - `seg-4x4-50fps-par1x1.y4m` (4x4, 50fps, A1:1, C420)
   - `seg-4x4-25fps-par2x1.y4m` (4x4, 25fps, A2:1, C420)
   - `seg-4x4-25fps-par2x2.y4m` (4x4, 25fps, A2:2, C420 — equivalent 1:1 aspect)
   - `seg-2x4-25fps-par1x1.y4m` (2x4, 25fps, A1:1, C420)
   - `cues.srt` (minimal plain SRT subtitle file)
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile

from generate_ffv1_gray_fixtures import element, uint
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
OUT = ROOT / "tests/fixtures/playback-errors/y4m-xfade-subburn-budget-parity"


def encode_ffv1_4x4_packets() -> list[bytes]:
    with tempfile.TemporaryDirectory(prefix="fvid-y4m-budget-gen-") as temp:
        temp_dir = Path(temp)
        code = (
            '''
#[path="FRAME"] mod owned_frame;
mod encoder {
    use crate::owned_frame::GeometryFrame;
    type Result<T> = std::result::Result<T, String>;
    fn invalid(message: &str) -> String { message.into() }
    include!("ENCODER");
}
fn main() {
    let out = std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());
    for idx in 0..3u8 {
        // 4x4 YUV420: 16 Y + 4 U + 4 V = 24 bytes
        let mut data = vec![48 + idx * 24; 16];
        data.extend_from_slice(&[112 + idx * 4; 4]);
        data.extend_from_slice(&[144 - idx * 4; 4]);
        let frame = owned_frame::GeometryFrame {
            width: 4,
            height: 4,
            subsampling: Some([2, 2]),
            data,
        };
        let pkt = encoder::encode(&frame, 8).unwrap();
        std::fs::write(out.join(format!("pkt-{idx}.bin")), pkt).unwrap();
    }
}
'''
            .replace("FRAME", str(ROOT / "crates/fvid-media/src/owned_frame.rs"))
            .replace("ENCODER", str(ROOT / "crates/fvid-media/src/owned_ffv1_encoder_impl.rs"))
        )
        src = temp_dir / "gen.rs"
        bin_path = temp_dir / "gen"
        src.write_text(code, encoding="utf-8")
        subprocess.run(["rustc", "--edition=2024", str(src), "-o", str(bin_path)], check=True)
        subprocess.run([str(bin_path), str(temp_dir)], check=True)
        return [(temp_dir / f"pkt-{idx}.bin").read_bytes() for idx in range(3)]


def build_multitrack_matroska_ffv1_aac(v_packets: list[bytes]) -> bytes:
    raw_aac = build_aac_packet(100)
    aac_pkt = raw_aac + bytes(512 - len(raw_aac))
    asc = b"\x11\x88"

    header = element(
        0x1A45DFA3,
        uint(0x4286, 1)
        + uint(0x42F7, 1)
        + uint(0x42F2, 4)
        + uint(0x42F3, 8)
        + element(0x4282, b"matroska")
        + uint(0x4287, 4)
        + uint(0x4285, 2),
    )
    info = element(
        0x1549A966,
        uint(0x2AD7B1, 1_000_000)
        + element(0x4D80, b"fvid-synthetic")
        + element(0x5741, b"fvid-synthetic")
        + element(0x4489, struct.pack(">d", 120.0)),
    )
    colour = element(
        0x55B0,
        uint(0x55B1, 6)
        + uint(0x55BA, 0)
        + uint(0x55BB, 0)
        + uint(0x55B9, 1),
    )
    v_track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 1)
        + element(0x86, b"V_FFV1")
        + uint(0x23E383, 40_000_000)
        + element(0xE0, uint(0xB0, 4) + uint(0xBA, 4) + colour),
    )
    a_settings = element(
        0xE1,
        element(0xB5, struct.pack(">d", 48000.0)) + uint(0x9F, 1),
    )
    a_track = element(
        0xAE,
        uint(0xD7, 2)
        + uint(0x73C5, 2)
        + uint(0x83, 2)
        + element(0x86, b"A_AAC")
        + element(0x63A2, asc)
        + uint(0x23E383, 21_333_334)
        + a_settings,
    )
    tracks = element(0x1654AE6B, v_track + a_track)
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + v_packets[0])
        + element(0xA3, b"\x82\x00\x00\x80" + aac_pkt)
        + element(0xA3, b"\x81\x00\x28\x80" + v_packets[1])
        + element(0xA3, b"\x82\x00\x15\x80" + aac_pkt)
        + element(0xA3, b"\x81\x00\x50\x80" + v_packets[2])
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_multitrack_mp4_avc_aac() -> bytes:
    avcc, avc_packets = build_avc_32x16_ipcm()
    raw_aac = build_aac_packet(100)
    aac_pkt = raw_aac + bytes(2048 - len(raw_aac))
    aac_packets = [aac_pkt, aac_pkt]

    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
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


def build_y4m(width: int, height: int, fps: str, aspect: str) -> bytes:
    hdr = f"YUV4MPEG2 W{width} H{height} F{fps} Ip A{aspect} C420\n".encode("ascii")
    y_len = width * height
    c_len = (width // 2) * (height // 2)
    frames = []
    for idx in range(3):
        y = bytes([50 + idx * 25] * y_len)
        u = bytes([110 + idx * 5] * c_len)
        v = bytes([145 - idx * 5] * c_len)
        frames.append(b"FRAME\n" + y + u + v)
    return hdr + b"".join(frames)


def generate() -> dict[str, bytes]:
    v_packets = encode_ffv1_4x4_packets()
    files = {
        "multitrack-ffv1-aac.mkv": build_multitrack_matroska_ffv1_aac(v_packets),
        "multitrack-avc-aac.mp4": build_multitrack_mp4_avc_aac(),
        "seg-4x4-25fps-par1x1.y4m": build_y4m(4, 4, "25:1", "1:1"),
        "seg-4x4-50fps-par1x1.y4m": build_y4m(4, 4, "50:1", "1:1"),
        "seg-4x4-25fps-par2x1.y4m": build_y4m(4, 4, "25:1", "2:1"),
        "seg-4x4-25fps-par2x2.y4m": build_y4m(4, 4, "25:1", "2:2"),
        "seg-2x4-25fps-par1x1.y4m": build_y4m(2, 4, "25:1", "1:1"),
        "cues.srt": b"1\n00:00:00,000 --> 00:00:00,080\nHi\n",
    }
    manifest = {
        "batch_id": "FVID-0263",
        "generator": "scripts/generate_y4m_xfade_subburn_budget_parity_regressions.py",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
