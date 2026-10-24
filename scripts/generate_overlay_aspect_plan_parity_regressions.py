#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 255 (`FVID-0262`).

Covers:
1. 90-degree rotated Matroska FFV1 video (`rot90-par1x2.mkv`, coded 4x2, stored
   PixelWidth=4, PixelHeight=2, DisplayWidth=2, DisplayHeight=2, PoseRoll=-90.0,
   BT.601 limited range) whose upright display dimensions are 2x4 with upright
   display pixel aspect `2:1`, paired with upright 2x4 Y4M videos (`upright-par2x1.y4m`
   with `A2:1` and `upright-par1x2.y4m` with `A1:2`).
2. Pre-overlay main geometry aspect (`upright-par1x1-4x4.y4m`, 4x4 with `A1:1`
   scaled via `--scale 2:4` to 2x4 with transformed display aspect `2:1`).
3. Overlay planner compatibility checks (`upright-fullrange.y4m` with `XCOLORRANGE=FULL`
   and `upright-444.y4m` with `C444`).
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile

from generate_ffv1_gray_fixtures import element, uint


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/overlay-aspect-plan-parity"


def encode_ffv1_packets() -> list[bytes]:
    with tempfile.TemporaryDirectory(prefix="fvid-overlay-aspect-gen-") as temp:
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
    for idx in 0..2u8 {
        // 4x2 YUV420: 8 Y samples + 2 U samples + 2 V samples = 12 bytes
        let mut data = vec![32 + idx * 40; 8];
        data.extend_from_slice(&[112 + idx * 8; 2]);
        data.extend_from_slice(&[144 - idx * 8; 2]);
        let frame = owned_frame::GeometryFrame {
            width: 4,
            height: 2,
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
        return [(temp_dir / f"pkt-{idx}.bin").read_bytes() for idx in range(2)]


def build_rot90_mkv(packets: list[bytes]) -> bytes:
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
        + element(0x4489, struct.pack(">d", 80.0)),
    )
    colour = element(
        0x55B0,
        uint(0x55B1, 6)  # MatrixCoefficients = BT.601 (6)
        + uint(0x55BA, 0)  # TransferCharacteristics = unspecified (0)
        + uint(0x55BB, 0)  # Primaries = unspecified (0)
        + uint(0x55B9, 1),  # Range = limited/studio (1)
    )
    projection = element(
        0x7670,
        element(0x7675, struct.pack(">f", -90.0)),  # ProjectionPoseRoll = -90.0 -> 90 deg CW
    )
    video = element(
        0xE0,
        uint(0xB0, 4)  # PixelWidth = 4
        + uint(0xBA, 2)  # PixelHeight = 2
        + uint(0x54B2, 3)  # DisplayUnit = 3 (aspect)
        + uint(0x54B0, 2)  # DisplayWidth = 2  (stored coded pixel_aspect = 1:2)
        + uint(0x54BA, 2)  # DisplayHeight = 2 (upright after 90 deg rotation: pixel_aspect = 2:1)
        + colour
        + projection,
    )
    track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 1)
        + element(0x86, b"V_FFV1")
        + uint(0x23E383, 40_000_000)
        + video,
    )
    tracks = element(0x1654AE6B, track)
    blocks = b"".join(
        element(
            0xA3,
            b"\x81" + (idx * 40).to_bytes(2, "big", signed=True) + b"\x80" + pkt,
        )
        for idx, pkt in enumerate(packets)
    )
    cluster = element(0x1F43B675, uint(0xE7, 0) + blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_y4m_2x4(aspect: str, extra_tag: str = "", c_tag: str = "C420") -> bytes:
    tag_suffix = f" {extra_tag}" if extra_tag else ""
    hdr = f"YUV4MPEG2 W2 H4 F25:1 Ip A{aspect} {c_tag}{tag_suffix}\n".encode("ascii")
    if c_tag == "C444":
        frame0 = bytes([60] * 8 + [120] * 8 + [140] * 8)
        frame1 = bytes([90] * 8 + [128] * 8 + [128] * 8)
    else:
        # 2x4 YUV420: 8 Y + 2 U + 2 V = 12 bytes
        frame0 = bytes([60] * 8 + [120, 120, 140, 140])
        frame1 = bytes([90] * 8 + [128, 128, 128, 128])
    return hdr + b"FRAME\n" + frame0 + b"FRAME\n" + frame1


def build_y4m_4x4_par1x1() -> bytes:
    hdr = b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420\n"
    # 4x4 YUV420: 16 Y + 4 U + 4 V = 24 bytes
    frame0 = bytes([48] * 16 + [100] * 4 + [156] * 4)
    frame1 = bytes([88] * 16 + [116] * 4 + [140] * 4)
    return hdr + b"FRAME\n" + frame0 + b"FRAME\n" + frame1


def generate() -> dict[str, bytes]:
    packets = encode_ffv1_packets()
    files = {
        "rot90-par1x2.mkv": build_rot90_mkv(packets),
        "upright-par2x1.y4m": build_y4m_2x4("2:1"),
        "upright-par1x2.y4m": build_y4m_2x4("1:2"),
        "upright-par1x1-4x4.y4m": build_y4m_4x4_par1x1(),
        "upright-fullrange.y4m": build_y4m_2x4("2:1", extra_tag="XCOLORRANGE=FULL"),
        "upright-444.y4m": build_y4m_2x4("2:1", c_tag="C444"),
    }
    manifest = {
        "batch_id": "FVID-0262",
        "generator": "scripts/generate_overlay_aspect_plan_parity_regressions.py",
        "files": sorted(files.keys()),
        "sha256": {
            name: hashlib.sha256(data).hexdigest() for name, data in sorted(files.items())
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
