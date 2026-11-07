#!/usr/bin/env python3
"""Generate synthetic fixtures for Ogg/Vorbis track metadata, MP3/WAVE INFO limits, and empty tag handling."""

from pathlib import Path
import struct


def ogg_crc32(data: bytes) -> int:
    crc = 0
    for b in data:
        crc ^= b << 24
        for _ in range(8):
            if crc & 0x80000000:
                crc = ((crc << 1) ^ 0x04C11DB7) & 0xFFFFFFFF
            else:
                crc = (crc << 1) & 0xFFFFFFFF
    return crc


def make_ogg_page(serial: int, seq: int, header_type: int, granule: int, packets: list[bytes]) -> bytes:
    lacing = bytearray()
    payload = bytearray()
    for pkt in packets:
        rem = len(pkt)
        while rem >= 255:
            lacing.append(255)
            rem -= 255
        lacing.append(rem)
        payload.extend(pkt)
    head = struct.pack(
        "<4sBBQIIIB",
        b"OggS",
        0,
        header_type,
        granule & 0xFFFFFFFFFFFFFFFF,
        serial,
        seq,
        0,
        len(lacing),
    ) + bytes(lacing)
    full = head + bytes(payload)
    crc = ogg_crc32(full)
    return full[:22] + struct.pack("<I", crc) + full[26:]


def build_opus_with_empty_first_tags() -> bytes:
    opus_head = struct.pack("<8sBBHIhB", b"OpusHead", 1, 1, 312, 48000, 0, 0)
    vendor = b"fvid-test"
    comments = [
        b"TITLE=",
        b"TITLE=Opus Real Title",
        b"DATE=",
        b"YEAR=2026",
        b"LANGUAGE=",
        b"LANGUAGE=eng",
    ]
    tags_body = bytearray(b"OpusTags")
    tags_body.extend(struct.pack("<I", len(vendor)))
    tags_body.extend(vendor)
    tags_body.extend(struct.pack("<I", len(comments)))
    for c in comments:
        tags_body.extend(struct.pack("<I", len(c)))
        tags_body.extend(c)
    # Minimal TOC byte for CELT-only 20ms mono frame + 2 bytes payload
    audio_pkt = bytes([0xF8, 0x00, 0x00])
    p0 = make_ogg_page(1, 0, 0x02, 0, [opus_head])
    p1 = make_ogg_page(1, 1, 0x00, 0, [bytes(tags_body)])
    p2 = make_ogg_page(1, 2, 0x04, 960, [audio_pkt])
    return p0 + p1 + p2


def build_vorbis_with_track_tags(repo_root: Path) -> bytes:
    src = (repo_root / "tests/fixtures/ogg/vorbis-stereo.ogg").read_bytes()
    # Locate the second OggS page which carries the comment and setup packets
    p1_off = src.find(b"OggS", 4)
    p2_off = src.find(b"OggS", p1_off + 4)
    page0 = src[:p1_off]
    page1 = src[p1_off:p2_off]
    rest = src[p2_off:]

    segs1 = page1[26]
    lacing1 = page1[27 : 27 + segs1]
    body1 = page1[27 + segs1 :]
    # Extract packets on page 1
    pkts = []
    cur = bytearray()
    idx = 0
    for l in lacing1:
        cur.extend(body1[idx : idx + l])
        idx += l
        if l < 255:
            pkts.append(bytes(cur))
            cur = bytearray()
    if cur:
        pkts.append(bytes(cur))
    # Replace comment packet (pkts[0]) with one containing empty + non-empty TITLE and LANGUAGE
    vendor = b"fvid-vorbis"
    comments = [
        b"TITLE=",
        b"TITLE=Vorbis Direct Title",
        b"LANGUAGE=",
        b"LANGUAGE=jpn",
    ]
    comment_pkt = bytearray(b"\x03vorbis")
    comment_pkt.extend(struct.pack("<I", len(vendor)))
    comment_pkt.extend(vendor)
    comment_pkt.extend(struct.pack("<I", len(comments)))
    for c in comments:
        comment_pkt.extend(struct.pack("<I", len(c)))
        comment_pkt.extend(c)
    comment_pkt.append(1)  # framing bit
    pkts[0] = bytes(comment_pkt)

    serial = struct.unpack_from("<I", page1, 14)[0]
    seq = struct.unpack_from("<I", page1, 18)[0]
    granule = struct.unpack_from("<Q", page1, 6)[0]
    header_type = page1[5]
    new_page1 = make_ogg_page(serial, seq, header_type, granule, pkts)
    return page0 + new_page1 + rest


def synchsafe(n: int) -> bytes:
    return bytes([
        (n >> 21) & 0x7F,
        (n >> 14) & 0x7F,
        (n >> 7) & 0x7F,
        n & 0x7F,
    ])


def id3v23_frame(fid: bytes, body: bytes) -> bytes:
    return fid + struct.pack(">IH", len(body), 0) + body


def build_mp3_with_many_txxx_and_empty_frames() -> bytes:
    frames = bytearray()
    # Empty TIT2 frame before real TIT2 frame, and empty TYER before TDRC
    frames.extend(id3v23_frame(b"TIT2", b"\x03\x00"))
    frames.extend(id3v23_frame(b"TIT2", b"\x03Real MP3 Title\x00"))
    frames.extend(id3v23_frame(b"TYER", b"\x03\x00"))
    frames.extend(id3v23_frame(b"TDRC", b"\x032026\x00"))
    # 66 4-character TXXX frames so total entries > 64
    for i in range(66):
        key = f"C{i:03d}".encode("ascii")
        val = f"val{i}".encode("ascii")
        frames.extend(id3v23_frame(b"TXXX", b"\x03" + key + b"\x00" + val + b"\x00"))
    id3 = b"ID3\x03\x00\x00" + synchsafe(len(frames)) + bytes(frames)
    # Append 2 valid MPEG1 Layer III 128kbps 44100Hz mono frames (417 bytes each)
    mp3_frame = bytearray(417)
    mp3_frame[0:4] = bytes([0xFF, 0xFB, 0x90, 0xC4])
    return id3 + bytes(mp3_frame) * 2


def build_wave_with_empty_and_nonempty_info() -> bytes:
    fmt = struct.pack("<HHIIHH", 1, 1, 8000, 16000, 2, 16)
    pcm = b"\x00\x00" * 80
    # LIST/INFO with empty INAM ("\x00\x00") and non-empty IART ("Synth Artist\x00")
    inam = b"INAM" + struct.pack("<I", 2) + b"\x00\x00"
    iart_val = b"Synth Artist\x00"
    iart = b"IART" + struct.pack("<I", len(iart_val)) + iart_val + b"\x00"
    info_payload = b"INFO" + inam + iart
    info_chunk = b"LIST" + struct.pack("<I", len(info_payload)) + info_payload
    riff_body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt))
        + fmt
        + b"data"
        + struct.pack("<I", len(pcm))
        + pcm
        + info_chunk
    )
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def main() -> None:
    repo_root = Path(__file__).resolve().parent.parent
    out_dir = repo_root / "tests/fixtures/playback-errors/ogg-mp3-wave-metadata"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "opus-empty-first-tags.ogg").write_bytes(build_opus_with_empty_first_tags())
    (out_dir / "vorbis-track-tags.ogg").write_bytes(build_vorbis_with_track_tags(repo_root))
    (out_dir / "mp3-many-txxx-and-empty-tit2.mp3").write_bytes(
        build_mp3_with_many_txxx_and_empty_frames()
    )
    (out_dir / "wave-empty-inam.wav").write_bytes(build_wave_with_empty_and_nonempty_info())


if __name__ == "__main__":
    main()
