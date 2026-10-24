#!/usr/bin/env python3
"""Generate synthetic Ogg framing, Vorbis chain-EOS, and multi-track Opus regressions."""
import hashlib
import json
import struct
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as owned

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/ogg-framing-track"


def sealed(payload: bytes, laces: list[int], granule: int, flags: int, sequence: int, serial: int) -> bytes:
    assert sum(laces) == len(payload), (sum(laces), len(payload))
    buf = bytearray(
        b"OggS"
        + bytes([0, flags])
        + struct.pack("<qII", granule, serial, sequence)
        + bytes(4)
        + bytes([len(laces)])
        + bytes(laces)
        + payload
    )
    crc = 0
    for b in buf:
        crc ^= b << 24
        for _ in range(8):
            crc = ((crc << 1) ^ 0x04C11DB7 if crc & 0x80000000 else crc << 1) & 0xFFFFFFFF
    struct.pack_into("<I", buf, 22, crc)
    return bytes(buf)


def page(payload: bytes, granule: int, flags: int, sequence: int, serial: int) -> bytes:
    laces = [255] * (len(payload) // 255) + [len(payload) % 255]
    return sealed(payload, laces, granule, flags, sequence, serial)


def bits(fields: list[tuple[int, int]]) -> bytes:
    out = bytearray()
    at = 0
    for value, width in fields:
        for i in range(width):
            if at % 8 == 0:
                out.append(0)
            out[at // 8] |= ((value >> i) & 1) << (at % 8)
            at += 1
    return bytes(out)


def vorbis_link(serial: int, eos: bool) -> bytes:
    ident = b"\x01vorbis" + struct.pack("<IBIiiiBB", 0, 2, 8000, 0, 0, 0, 0x86, 1)
    comment = b"\x03vorbis" + bytes(8) + b"\x01"
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
    for long in [0, 0, 1]:
        fields.extend([(long, 1), (0, 16), (0, 16), (0, 8)])
    setup = bits(fields + [(1, 1)])
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
        packets.append(bits(header + [(0, 1), (0, 1)]))
    data = page(ident, 0, 2, 0, serial) + page(comment, 0, 0, 1, serial) + page(setup, 0, 0, 2, serial)
    ends = [0, 80, 160, 240, 368, 448]
    for i, pkt in enumerate(packets):
        granule = (411 if eos else 448) if i == 5 else ends[i] if i == 1 else -1
        flags = (4 if eos else 0) if i == 5 else 0
        data += page(pkt, granule, flags, i + 3, serial)
    return data


def opus_tags(title: str, lang: str) -> bytes:
    vendor = b"fvid"
    comments = [f"TITLE={title}".encode("utf-8"), f"LANGUAGE={lang}".encode("utf-8")]
    out = bytearray(b"OpusTags")
    out += struct.pack("<I", len(vendor)) + vendor
    out += struct.pack("<I", len(comments))
    for c in comments:
        out += struct.pack("<I", len(c)) + c
    return bytes(out)


def generate() -> dict[str, bytes]:
    head_mono = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48000, 0, 0)
    head_stereo = b"OpusHead" + bytes([1, 2]) + struct.pack("<HIhB", 0, 48000, 0, 0)
    tags0 = opus_tags("TrackZero", "eng")
    tags1 = opus_tags("TrackOne", "rus")
    raw20 = bytes.fromhex("f8fffe")
    raw10 = bytes.fromhex("f0fffe")

    # Parallel 2-track Opus fixture: track 0 has 2x20ms (1920 frames), track 1 has 2x10ms (960 frames).
    s0, s1 = 0x46564F00, 0x46564F01
    parallel_opus = b"".join(
        [
            page(head_mono, 0, 2, 0, s0),
            page(head_stereo, 0, 2, 0, s1),
            page(tags0, 0, 0, 1, s0),
            page(tags1, 0, 0, 1, s1),
            page(raw20, 960, 0, 2, s0),
            page(raw10, 480, 0, 2, s1),
            page(raw20, 1920, 4, 3, s0),
            page(raw10, 960, 4, 3, s1),
        ]
    )

    # Reserved header_type_flag (0x08) on BOS page and on audio page.
    reserved_bos = b"".join(
        [
            page(head_mono, 0, 2 | 0x08, 0, s0),
            page(tags0, 0, 0, 1, s0),
            page(raw20, 960, 4, 2, s0),
        ]
    )
    reserved_audio = b"".join(
        [
            page(head_mono, 0, 2, 0, s0),
            page(tags0, 0, 0, 1, s0),
            page(raw20, 960, 4 | 0x80, 2, s0),
        ]
    )

    # 258-byte packet split across pages (255 + 3) with a 0-segment bridge page.
    split_pkt = bytes([0xFB, 0x41, 253]) + bytes.fromhex("fffe") + bytes(253)
    assert len(split_pkt) == 258
    # Valid control: 0-segment bridge page with flags=1 (or flags=0) while packet is open, followed by flags=5 completion.
    continued_empty_bridge = b"".join(
        [
            page(head_mono, 0, 2, 0, s0),
            page(tags0, 0, 0, 1, s0),
            sealed(split_pkt[:255], [255], -1, 0, 2, s0),
            sealed(b"", [], -1, 1, 3, s0),
            sealed(split_pkt[255:], [3], 960, 5, 4, s0),
        ]
    )
    # Malformed: 0-segment page with flags=1 (CONTINUES) when NO packet is open!
    spurious_empty_continue = b"".join(
        [
            page(head_mono, 0, 2, 0, s0),
            page(tags0, 0, 0, 1, s0),
            sealed(b"", [], -1, 1, 2, s0),
            page(raw20, 960, 4, 3, s0),
        ]
    )
    # Malformed: after audio packet 0 finishes cleanly on page 2, page 3 claims CONTINUES (flags=5) when no packet is open!
    spurious_midstream_continue = b"".join(
        [
            page(head_mono, 0, 2, 0, s0),
            page(tags0, 0, 0, 1, s0),
            page(raw20, 960, 0, 2, s0),
            page(raw20, 1920, 4 | 1, 3, s0),
        ]
    )

    # Chained Vorbis where link 0 is missing EOS before link 1 begins.
    vorbis_chained_control = vorbis_link(0x46565430, True) + vorbis_link(0x46565431, True)
    vorbis_unended_first_link = vorbis_link(0x46565430, False) + vorbis_link(0x46565431, True)

    files = {
        "video.mkv": owned()["video.mkv"],
        "mono.head": head_mono,
        "stereo.head": head_stereo,
        "raw20.packet": raw20,
        "raw10.packet": raw10,
        "split20.packet": split_pkt,
        "parallel-opus.ogg": parallel_opus,
        "continued-empty-bridge.ogg": continued_empty_bridge,
        "vorbis-chained-control.ogg": vorbis_chained_control,
        "reserved-bos-flag.ogg": reserved_bos,
        "reserved-audio-flag.ogg": reserved_audio,
        "spurious-empty-continue.ogg": spurious_empty_continue,
        "spurious-midstream-continue.ogg": spurious_midstream_continue,
        "vorbis-unended-first-link.ogg": vorbis_unended_first_link,
    }
    manifest = {
        "provenance": "Synthetic Ogg pages, Opus headers/DTX packets, and silent Vorbis links with valid CRCs and consecutive sequence numbers. No FFmpeg, private media, or network.",
        "controls": [
            {"file": "parallel-opus.ogg", "tracks": 2, "frames": [1920, 960]},
            {"file": "continued-empty-bridge.ogg", "tracks": 1, "frames": [960]},
            {"file": "vorbis-chained-control.ogg", "tracks": 1, "frames": [822]},
        ],
        "malformed_ogg": [
            {"file": "reserved-bos-flag.ogg", "error": "header_type_flag"},
            {"file": "reserved-audio-flag.ogg", "error": "header_type_flag"},
            {"file": "spurious-empty-continue.ogg", "error": "continuation"},
            {"file": "spurious-midstream-continue.ogg", "error": "continuation"},
        ],
        "malformed_vorbis": [
            {"file": "vorbis-unended-first-link.ogg", "error": "before the previous segment ended"},
        ],
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode("utf-8")
    return files


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
