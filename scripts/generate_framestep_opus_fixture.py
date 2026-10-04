#!/usr/bin/env python3
"""Synthetic FFV1 + Opus silence temporal-export regression; no external codecs."""
from pathlib import Path
import hashlib
import struct

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors"


def vint(n):
    for size in range(1, 9):
        if n < (1 << (7 * size)) - 1:
            return ((1 << (7 * size)) | n).to_bytes(size, "big")
    raise ValueError("size overflow")


def element(tag, data):
    return tag.to_bytes((tag.bit_length() + 7) // 8, "big") + vint(len(data)) + data


def uint(tag, n):
    return element(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def children(data):
    at = 0
    while at < len(data):
        start = at
        width = 1
        while not data[at] & (0x80 >> (width - 1)):
            width += 1
        tag = int.from_bytes(data[at:at + width], "big")
        at += width
        width = 1
        while not data[at] & (0x80 >> (width - 1)):
            width += 1
        length = int.from_bytes(data[at:at + width], "big") & ((1 << (7 * width)) - 1)
        at += width
        if length == (1 << (7 * width)) - 1:
            length = len(data) - at
        assert at + length <= len(data), (tag, start)
        yield tag, data[at:at + length]
        at += length


def main():
    seed = (OUT / "shuffleplanes-444-8.mkv").read_bytes()
    assert hashlib.sha256(seed).hexdigest() == "56694fe9150e02e6bd9e63cbac5c5d5818a3cd6a15351d7b15f55631d9248d46"
    segment = next(body for tag, body in children(seed) if tag == 0x18538067)
    entries = list(children(segment))
    tracks = next(body for tag, body in entries if tag == 0x1654AE6B)
    video = next(body for tag, body in children(tracks) if tag == 0xAE)
    # The seed has one FFV1 track and independent keyframes, no private data.
    video = b"".join(element(tag, body) for tag, body in children(video) if tag not in (0xD7, 0x73C5, 0x23E383))
    video = uint(0xD7, 1) + uint(0x73C5, 1) + uint(0x23E383, 40_000_000) + video
    blocks = []
    for tag, body in entries:
        if tag == 0x1F43B675:
            for kind, value in children(body):
                if kind == 0xA3:
                    blocks.append(value)
                elif kind == 0xA0:
                    blocks.extend(v for k, v in children(value) if k == 0xA1)
    assert blocks and blocks[0][0] == 0x81 and blocks[0][3] & 6 == 0
    packet = blocks[0][4:]
    config = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 312, 48000, 0, 0)
    audio = uint(0xD7, 2) + uint(0x73C5, 2) + uint(0x83, 2)
    audio += element(0x86, b"A_OPUS") + element(0x63A2, config)
    audio += uint(0x56AA, 6_500_000) + uint(0x56BB, 80_000_000)
    audio += element(0xE1, element(0xB5, struct.pack(">d", 48000.0)) + uint(0x9F, 1))
    header = element(0x1A45DFA3, uint(0x4286, 1) + uint(0x42F7, 1) + uint(0x42F2, 4) + uint(0x42F3, 8)
                     + element(0x4282, b"matroska") + uint(0x4287, 4) + uint(0x4285, 2))
    info = element(0x1549A966, uint(0x2AD7B1, 1_000_000))
    cluster = uint(0xE7, 0)
    for time in range(0, 240, 20):
        if time % 40 == 0:
            block = b"\x81" + struct.pack(">hB", time, 0x80) + packet
            cluster += element(0xA0, element(0xA1, block) + uint(0x9B, 40))
        silence = b"\x82" + struct.pack(">hB", time, 0) + b"\xf8\xff\xfe"
        group = element(0xA1, silence) + uint(0x9B, 20)
        if time == 220:
            group += element(0x75A2, (2_000_000).to_bytes(4, "big", signed=True))
        cluster += element(0xA0, group)
    result = header + element(0x18538067, info + element(0x1654AE6B, element(0xAE, video) + element(0xAE, audio))
                             + element(0x1F43B675, cluster))
    (OUT / "framestep-opus.mkv").write_bytes(result)
    print(f"framestep-opus.mkv: {len(result)} bytes")


if __name__ == "__main__":
    main()
