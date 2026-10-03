#!/usr/bin/env python3
"""Generate own FFV1 monochrome packets and short Matroska videos, without FFmpeg."""
import argparse
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def vint(value):
    for size in range(1, 9):
        if value < (1 << (7 * size)) - 1:
            return ((1 << (7 * size)) | value).to_bytes(size, "big")
    raise ValueError("EBML size overflow")

def element(identifier, data):
    return identifier.to_bytes((identifier.bit_length() + 7) // 8, "big") + vint(len(data)) + data

def uint(identifier, value):
    return element(identifier, value.to_bytes(max(1, (value.bit_length() + 7) // 8), "big"))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "tests/fixtures/playback-errors")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="fvid-gray-fixtures-") as temp:
        temp = Path(temp)
        code = '''
#[path="FRAME"] mod owned_frame;
mod encoder {
    use crate::owned_frame::GeometryFrame;
    type Result<T> = std::result::Result<T, String>;
    fn invalid(message: &str) -> String { message.into() }
    include!("ENCODER");
}
fn main() {
    let output = std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());
    let extras = std::path::PathBuf::from(std::env::args_os().nth(2).unwrap());
    for depth in [8u8, 10, 16] {
        for index in 0..if depth == 8 { 6 } else { 2 } {
            let mut samples = Vec::new();
            let maximum = (1u32 << depth) - 1;
            for pixel in 0..12u32 {
                let value = match pixel { 0 => 0, 1 => maximum, 2 => 1 << (depth - 1), _ => (pixel * 977 + index * 1237) & maximum };
                let bytes = (value as u16).to_le_bytes();
                samples.extend_from_slice(&bytes[..if depth == 8 { 1 } else { 2 }]);
            }
            let packet = encoder::encode_gray(4, 3, &samples, depth).unwrap();
            let target = if index < 2 { &output } else { &extras };
            std::fs::write(target.join(format!("ffv1-gray-{depth}-{index}.packet")), packet).unwrap();
            std::fs::write(target.join(format!("ffv1-gray-{depth}-{index}.gray")), samples).unwrap();
        }
    }
    for index in 0..4u8 {
        let mut data = vec![20 + index * 10; 64];
        data.extend_from_slice(&[110 + index; 16]);
        data.extend_from_slice(&[140 - index; 16]);
        let frame = owned_frame::GeometryFrame {width:8,height:8,subsampling:Some([2,2]),data};
        std::fs::write(extras.join(format!("overlay-{index}.packet")), encoder::encode(&frame,8).unwrap()).unwrap();
    }

}
'''.replace("FRAME", str(ROOT / "crates/fvid-media/src/owned_frame.rs")).replace("ENCODER", str(ROOT / "crates/fvid-media/src/owned_ffv1_encoder_impl.rs"))
        source, binary = temp / "generate.rs", temp / "generate"
        source.write_text(code)
        subprocess.run(["rustc", "--edition=2024", str(source), "-o", str(binary)], check=True)
        subprocess.run([str(binary), str(args.output.resolve()), str(temp)], check=True)
        extra_packets = [(temp / f"ffv1-gray-8-{index}.packet").read_bytes() for index in range(2, 6)]
        overlay_packets = [(temp / f"overlay-{index}.packet").read_bytes() for index in range(4)]
    videos = [(depth, 40_000_000, f"ffv1-gray-{depth}.mkv") for depth in [8, 10, 16]]
    videos.append((8, 16_666_667, "matroska-default-duration-60fps.mkv"))
    videos.extend([(8, 40_000_000, "ffv1-positive-start.mkv"),
                   (8, 40_000_000, "ffv1-invalid-range-header.mkv"),
                   (8, 40_000_000, "ffv1-vfr.mkv"),
                   (8, 40_000_000, "ffv1-six-frames.mkv"),
                   (8, 40_000_000, "ffv1-custom-tags.mkv"),
                   (8, 40_000_000, "ffv1-track-tags.mkv")])
    for depth, frame_duration, name in videos:
        header = element(0x1A45DFA3, uint(0x4286, 1) + uint(0x42F7, 1) + uint(0x42F2, 4) + uint(0x42F3, 8) + element(0x4282, b"matroska") + uint(0x4287, 4) + uint(0x4285, 2))
        info = element(0x1549A966, uint(0x2AD7B1, 1_000_000) + element(0x4D80, b"fvid-synthetic") + element(0x5741, b"fvid-synthetic") + element(0x4489, struct.pack(">d", 100 if name == "ffv1-vfr.mkv" else (6 if name == "ffv1-six-frames.mkv" else 2) * frame_duration / 1_000_000)))
        track = uint(0xD7, 1) + uint(0x73C5, 37 if name == "ffv1-track-tags.mkv" else 1) + uint(0x83, 1) + element(0x86, b"V_FFV1") + uint(0x23E383, 0 if name == "ffv1-vfr.mkv" else frame_duration) + element(0xE0, uint(0xB0, 4) + uint(0xBA, 3) + (uint(0x54B0, 8) + uint(0x54BA, 3) if name == "ffv1-vfr.mkv" else b""))
        tracks = element(0x1654AE6B, element(0xAE, track))
        cluster = uint(0xE7, 200 if name == "ffv1-positive-start.mkv" else 0)
        for index in range(6 if name == "ffv1-six-frames.mkv" else 2):
            packet = extra_packets[index-2] if index >= 2 else (args.output / f"ffv1-gray-{depth}-{index}.packet").read_bytes()
            if name == "ffv1-invalid-range-header.mkv" and index == 1:
                packet = b"\xff\xff" + packet[2:]
            if name == "ffv1-vfr.mkv":
                block = b"\x81" + (0 if index == 0 else 73).to_bytes(2, "big") + b"\x00" + packet
                cluster += element(0xA0, element(0xA1, block) + uint(0x9B, 73 if index == 0 else 27))
            else:
                cluster += element(0xA3, b"\x81" + ((index * frame_duration + 500_000) // 1_000_000).to_bytes(2, "big") + b"\x80" + packet)
        tags = b""
        if name in ["ffv1-custom-tags.mkv", "ffv1-track-tags.mkv"]:
            simple = lambda key, value: element(0x67C8, element(0x45A3, key) + element(0x4487, value))
            global_tag = element(0x7373, simple(b"TITLE", b"Synthetic tags") + simple(b"FVID_TEST_NOTE", b"own container metadata") + simple(b"ENCODER", b"synthetic source"))
            track_tag = element(0x7373, simple(b"PRIVATE_TRACK_NOTE", b"not file metadata") + element(0x63C0, uint(0x63C5, 37)))
            tags = element(0x1254C367, global_tag + (track_tag if name == "ffv1-track-tags.mkv" else b""))
        (args.output / name).write_bytes(header + element(0x18538067, info + tracks + element(0x1F43B675, cluster) + tags))

    info = element(0x1549A966, uint(0x2AD7B1, 1_000_000) + element(0x4D80, b"fvid-synthetic") + element(0x5741, b"fvid-synthetic") + element(0x4489, struct.pack(">d", 1600)))
    track = uint(0xD7, 1) + uint(0x73C5, 1) + uint(0x83, 1) + element(0x86, b"V_FFV1") + element(0xE0, uint(0xB0, 8) + uint(0xBA, 8))
    tracks = element(0x1654AE6B, element(0xAE, track))
    cluster = uint(0xE7, 0)
    for packet, pts, duration in zip(overlay_packets, [0, 300, 950, 1350], [300, 650, 400, 250]):
        block = b"\x81" + pts.to_bytes(2, "big") + b"\x00" + packet
        cluster += element(0xA0, element(0xA1, block) + uint(0x9B, duration))
    (args.output / "ffv1-overlay-vfr.mkv").write_bytes(header + element(0x18538067, info + tracks + element(0x1F43B675, cluster)))

if __name__ == "__main__":
    main()
