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
    for depth in [8u8, 10, 16] {
        for index in 0..2 {
            let mut samples = Vec::new();
            let maximum = (1u32 << depth) - 1;
            for pixel in 0..12u32 {
                let value = match pixel { 0 => 0, 1 => maximum, 2 => 1 << (depth - 1), _ => (pixel * 977 + index * 1237) & maximum };
                let bytes = (value as u16).to_le_bytes();
                samples.extend_from_slice(&bytes[..if depth == 8 { 1 } else { 2 }]);
            }
            let packet = encoder::encode_gray(4, 3, &samples, depth).unwrap();
            std::fs::write(output.join(format!("ffv1-gray-{depth}-{index}.packet")), packet).unwrap();
            std::fs::write(output.join(format!("ffv1-gray-{depth}-{index}.gray")), samples).unwrap();
        }
    }
}
'''.replace("FRAME", str(ROOT / "crates/fvid-media/src/owned_frame.rs")).replace("ENCODER", str(ROOT / "crates/fvid-media/src/owned_ffv1_encoder_impl.rs"))
        source, binary = temp / "generate.rs", temp / "generate"
        source.write_text(code)
        subprocess.run(["rustc", "--edition=2024", str(source), "-o", str(binary)], check=True)
        subprocess.run([str(binary), str(args.output.resolve())], check=True)
    for depth in [8, 10, 16]:
        header = element(0x1A45DFA3, uint(0x4286, 1) + uint(0x42F7, 1) + uint(0x42F2, 4) + uint(0x42F3, 8) + element(0x4282, b"matroska") + uint(0x4287, 4) + uint(0x4285, 2))
        info = element(0x1549A966, uint(0x2AD7B1, 1_000_000) + element(0x4D80, b"fvid-synthetic") + element(0x5741, b"fvid-synthetic") + element(0x4489, struct.pack(">d", 80)))
        track = uint(0xD7, 1) + uint(0x73C5, 1) + uint(0x83, 1) + element(0x86, b"V_FFV1") + uint(0x23E383, 40_000_000) + element(0xE0, uint(0xB0, 4) + uint(0xBA, 3))
        tracks = element(0x1654AE6B, element(0xAE, track))
        cluster = uint(0xE7, 0)
        for index in range(2):
            packet = (args.output / f"ffv1-gray-{depth}-{index}.packet").read_bytes()
            cluster += element(0xA3, b"\x81" + (index * 40).to_bytes(2, "big") + b"\x80" + packet)
        (args.output / f"ffv1-gray-{depth}.mkv").write_bytes(header + element(0x18538067, info + tracks + element(0x1F43B675, cluster)))

if __name__ == "__main__":
    main()
