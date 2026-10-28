#!/usr/bin/env python3
"""Generate deterministic synthetic Ogg audio stream-selection, discrete-layout, and prefix packet-budget fixtures."""

from __future__ import annotations

import pathlib
import struct


ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURES_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "ogg-audio-selection-budget"


def _build_ogg_crc_table() -> list[int]:
    table: list[int] = []
    for index in range(256):
        word = index << 24
        for _ in range(8):
            if word & 0x8000_0000:
                word = ((word << 1) ^ 0x04C1_1DB7) & 0xFFFF_FFFF
            else:
                word = (word << 1) & 0xFFFF_FFFF
        table.append(word)
    return table


_OGG_CRC_TABLE = _build_ogg_crc_table()


def ogg_crc(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc = ((crc << 8) ^ _OGG_CRC_TABLE[((crc >> 24) ^ byte) & 0xFF]) & 0xFFFF_FFFF
    return crc


def ogg_page(serial: int, sequence: int, granule: int, flags: int, packets: list[bytes]) -> bytes:
    table = bytearray()
    body = bytearray()
    for packet in packets:
        body.extend(packet)
        rest = len(packet)
        while rest >= 255:
            table.append(255)
            rest -= 255
        table.append(rest)
    header_prefix = struct.pack("<4sBBqII", b"OggS", 0, flags, granule, serial, sequence)
    header_suffix = bytes([len(table)]) + bytes(table) + bytes(body)
    crc = ogg_crc(header_prefix + b"\x00\x00\x00\x00" + header_suffix)
    return header_prefix + struct.pack("<I", crc) + header_suffix


def opus_head_family0(channels: int, pre_skip: int = 312, input_rate: int = 48_000) -> bytes:
    return struct.pack("<8sBBHIhB", b"OpusHead", 1, channels, pre_skip, input_rate, 0, 0)


def opus_head_family255(channels: int, pre_skip: int = 312, input_rate: int = 48_000) -> bytes:
    mapping = bytes(range(channels))
    return (
        struct.pack("<8sBBHIhBBB", b"OpusHead", 1, channels, pre_skip, input_rate, 0, 255, channels, 0)
        + mapping
    )


def opus_tags(title: str = "Synthetic Opus") -> bytes:
    vendor = b"fvid-test"
    comment = f"TITLE={title}".encode("utf-8")
    return (
        b"OpusTags"
        + struct.pack("<I", len(vendor))
        + vendor
        + struct.pack("<I", 1)
        + struct.pack("<I", len(comment))
        + comment
    )


def celt_20ms_packet(size: int, stereo: bool = False) -> bytes:
    assert size >= 2
    toc = 0xFC if stereo else 0xF8
    return bytes([toc]) + (b"\x00" * (size - 1))


def build_skeleton_plus_opus() -> bytes:
    """Multiplexed Ogg where logical stream 0 is Skeleton (`fisb`) and stream 1 is Opus."""
    skeleton_bos = b"fisbone\x00" + b"\x00" * 44
    skeleton_fishead = b"fisb" + b"\x00" * 60
    opus_bos = opus_head_family0(2, pre_skip=0)
    opus_comment = opus_tags("TrackAtIndexOne")
    pkt0 = celt_20ms_packet(16, stereo=True)
    pkt1 = celt_20ms_packet(16, stereo=True)

    pages = [
        # Stream 0 BOS (Skeleton)
        ogg_page(serial=100, sequence=0, granule=0, flags=0x02, packets=[skeleton_fishead]),
        # Stream 1 BOS (Opus)
        ogg_page(serial=200, sequence=0, granule=0, flags=0x02, packets=[opus_bos]),
        # Stream 0 packet + EOS
        ogg_page(serial=100, sequence=1, granule=0, flags=0x04, packets=[skeleton_bos]),
        # Stream 1 tags
        ogg_page(serial=200, sequence=1, granule=0, flags=0x00, packets=[opus_comment]),
        # Stream 1 audio packet 0 (960 samples)
        ogg_page(serial=200, sequence=2, granule=960, flags=0x00, packets=[pkt0]),
        # Stream 1 audio packet 1 (1920 samples) + EOS
        ogg_page(serial=200, sequence=3, granule=1920, flags=0x04, packets=[pkt1]),
    ]
    return b"".join(pages)


def build_discrete_family255_stereo_opus() -> bytes:
    """Single-stream Ogg Opus using mapping family 255 (discrete 2-channel layout, mask=None)."""
    head = opus_head_family255(2, pre_skip=0)
    tags = opus_tags("DiscreteStereo")
    # Two single-stream packets concatenated for multistream (2 uncoupled streams):
    # In multistream Opus with 2 streams, stream 0 is self-delimited and stream 1 is final.
    # Self-delimited CELT 20ms packet: [toc=0xF8, len=3, 0, 0, 0], followed by final stream [toc=0xF8, 0, 0, 0].
    ms_pkt = bytes([0xF8, 3, 0, 0, 0, 0xF8, 0, 0, 0])
    pages = [
        ogg_page(serial=300, sequence=0, granule=0, flags=0x02, packets=[head]),
        ogg_page(serial=300, sequence=1, granule=0, flags=0x00, packets=[tags]),
        ogg_page(serial=300, sequence=2, granule=960, flags=0x00, packets=[ms_pkt]),
        ogg_page(serial=300, sequence=3, granule=1920, flags=0x04, packets=[ms_pkt]),
    ]
    return b"".join(pages)


def build_prefix_budget_opus() -> bytes:
    """Single-stream Ogg Opus where packet 0 is 12 bytes (20 ms) and packet 1 is 120 bytes (20 ms)."""
    head = opus_head_family0(1, pre_skip=0)
    tags = opus_tags("PrefixBudget")
    small_pkt = celt_20ms_packet(12, stereo=False)
    large_pkt = celt_20ms_packet(120, stereo=False)
    pages = [
        ogg_page(serial=400, sequence=0, granule=0, flags=0x02, packets=[head]),
        ogg_page(serial=400, sequence=1, granule=0, flags=0x00, packets=[tags]),
        ogg_page(serial=400, sequence=2, granule=960, flags=0x00, packets=[small_pkt]),
        ogg_page(serial=400, sequence=3, granule=1920, flags=0x04, packets=[large_pkt]),
    ]
    return b"".join(pages)


def _pack_vorbis_bits(fields: list[tuple[int, int]]) -> bytes:
    out = bytearray()
    at = 0
    for value, width in fields:
        for i in range(width):
            if at % 8 == 0:
                out.append(0)
            out[at // 8] |= ((value >> i) & 1) << (at % 8)
            at += 1
    return bytes(out)


def _vorbis_link(serial: int, channels: int = 1, sample_rate: int = 48_000) -> bytes:
    ident = b"\x01vorbis" + struct.pack("<IBIiiiBB", 0, channels, sample_rate, 0, 0, 0, 0x86, 1)
    comment = b"\x03vorbis" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0) + b"\x01"
    fields = (
        [(5, 8)]
        + [(b, 8) for b in b"vorbis"]
        + [
            (0, 8),
            (0x564342, 24),
            (1, 16),
            (1, 24),
            (0, 1),
            (0, 1),
            (0, 5),
            (0, 4),
            (0, 6),
            (0, 16),
            (0, 6),
            (1, 16),
            (0, 5),
            (0, 2),
            (6, 4),
            (0, 6),
            (0, 16),
            (0, 24),
            (128, 24),
            (31, 24),
            (0, 6),
            (0, 8),
            (0, 3),
            (0, 1),
            (0, 6),
            (0, 16),
            (0, 1),
            (0, 1),
            (0, 2),
            (0, 8),
            (0, 8),
            (0, 8),
            (2, 6),
        ]
    )
    for long_flag in [0, 0, 1]:
        fields.extend([(long_flag, 1), (0, 16), (0, 16), (0, 8)])
    setup = _pack_vorbis_bits(fields + [(1, 1)])
    modes = [0, 2, 1, 2, 2, 0]
    packets = []
    for i, mode in enumerate(modes):
        header = [(0, 1), (mode, 2)]
        if mode == 2:
            header.extend(
                [
                    (int(i > 0 and modes[i - 1] == 2), 1),
                    (int(i + 1 < len(modes) and modes[i + 1] == 2), 1),
                ]
            )
        packets.append(_pack_vorbis_bits(header + [(0, 1)] * channels))
    pages = [
        ogg_page(serial=serial, sequence=0, granule=0, flags=0x02, packets=[ident]),
        ogg_page(serial=serial, sequence=1, granule=0, flags=0x00, packets=[comment]),
        ogg_page(serial=serial, sequence=2, granule=0, flags=0x00, packets=[setup]),
    ]
    ends = [0, 80, 160, 240, 368, 448]
    for i, pkt in enumerate(packets):
        granule = 411 if i == 5 else ends[i] if i == 1 else -1
        flags = 0x04 if i == 5 else 0x00
        pages.append(ogg_page(serial=serial, sequence=i + 3, granule=granule, flags=flags, packets=[pkt]))
    return b"".join(pages)


def build_joint_prefix_budget_ogg() -> bytes:
    """Chained Ogg (Link 0: Opus 48 kHz with 12-byte pkt0 [0..960) and 120-byte pkt1 [960..1920),
    followed by Link 1: Vorbis 48 kHz [1920..2331))."""
    head = opus_head_family0(1, pre_skip=0)
    tags = opus_tags("JointPrefixBudget")
    small_pkt = celt_20ms_packet(12, stereo=False)
    large_pkt = celt_20ms_packet(120, stereo=False)
    opus_link = b"".join(
        [
            ogg_page(serial=500, sequence=0, granule=0, flags=0x02, packets=[head]),
            ogg_page(serial=500, sequence=1, granule=0, flags=0x00, packets=[tags]),
            ogg_page(serial=500, sequence=2, granule=960, flags=0x00, packets=[small_pkt]),
            ogg_page(serial=500, sequence=3, granule=1920, flags=0x04, packets=[large_pkt]),
        ]
    )
    return opus_link + _vorbis_link(serial=501, channels=1, sample_rate=48_000)


def generate_fixtures(output_dir: pathlib.Path = FIXTURES_DIR) -> dict[str, pathlib.Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "skeleton_plus_opus": output_dir / "skeleton-plus-opus.ogg",
        "discrete_family255_stereo": output_dir / "discrete-family255-stereo.opus",
        "prefix_budget_opus": output_dir / "prefix-budget.opus",
        "joint_prefix_budget_ogg": output_dir / "joint-prefix-budget.ogg",
    }
    files["skeleton_plus_opus"].write_bytes(build_skeleton_plus_opus())
    files["discrete_family255_stereo"].write_bytes(build_discrete_family255_stereo_opus())
    files["prefix_budget_opus"].write_bytes(build_prefix_budget_opus())
    files["joint_prefix_budget_ogg"].write_bytes(build_joint_prefix_budget_ogg())
    return files


if __name__ == "__main__":
    generated = generate_fixtures()
    for name, path in sorted(generated.items()):
        print(f"{name}: {path.relative_to(ROOT)} ({path.stat().st_size} bytes)")
