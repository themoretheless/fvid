#!/usr/bin/env python3
"""Purely author container loudness, Matroska audio plan/export speaker layout,
QuickTime PCM probe default bit depth, and native loudness implicit PS regressions.
"""
import hashlib, json, math, struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_flac_coded_numbers_regressions import frame as flac_frame, stream as flac_stream
from generate_he_aac_packet_fixtures import DEST, box

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/container-loudness-plan"


def u(tag, n):
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def simple_block(track_no, rel_ms, flags, payload):
    assert 1 <= track_no <= 126
    return ebml("a3", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, flags) + payload)


def vorbis_comment_block(fields, last=True):
    body = bytearray(struct.pack("<I", 0))  # empty vendor
    body += struct.pack("<I", len(fields))
    for f in fields:
        raw = f.encode("utf-8")
        body += struct.pack("<I", len(raw)) + raw
    header = bytes([(0x80 if last else 0x00) | 4]) + len(body).to_bytes(3, "big")
    return header + bytes(body)


def build_matroska_flac():
    # 11 frames of 1920 samples (40 ms exact at 48000 Hz; 11 * 40 = 440 ms > 400 ms loudness gate)
    packets = [flac_frame(i, 1920, False, 8192 if i % 2 == 0 else -8192, block_code=7, rate_code=10) for i in range(11)]
    raw_flac = flac_stream(packets, [1920] * 11, rate=48000)
    streaminfo = raw_flac[8:42]
    private = b"fLaC\x80\x00\x00\x22" + streaminfo
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 1)
        + u("83", 2)
        + ebml("86", b"A_FLAC")
        + ebml("63a2", private)
        + ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1) + u("6264", 16)),
    )
    blocks = u("e7", 0)
    for i, pkt in enumerate(packets):
        stamp = i * 40  # 1920 / 48000 = 40 ms exact!
        blocks += simple_block(1, stamp, 0x80, pkt)
    return header + ebml("18538067", info + ebml("1654ae6b", audio) + ebml("1f43b675", blocks))


def build_matroska_flac_4ch_unassigned():
    # Take the 4-channel FLAC frame from flac-native/extended-lpc-4-1-0-0.flac and attach WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0
    original = (DEST / "flac-native/extended-lpc-4-1-0-0.flac").read_bytes()
    streaminfo = original[8:42]
    pkt = original[42:]
    private = (
        b"fLaC\x00\x00\x00\x22"
        + streaminfo
        + vorbis_comment_block(["WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0"], last=True)
    )
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 1)
        + u("83", 2)
        + ebml("86", b"A_FLAC")
        + ebml("63a2", private)
        + ebml("e1", ebml("b5", struct.pack(">d", 8000.0)) + u("9f", 4) + u("6264", 16)),
    )
    blocks = u("e7", 0) + simple_block(1, 0, 0x80, pkt)
    return header + ebml("18538067", info + ebml("1654ae6b", audio) + ebml("1f43b675", blocks))


def build_matroska_mp1():
    # Take the first 55 frames of synthetic 48000 Hz mono MP1 from mp1-native (55 * 384 = 21120 samples > 0.4 s)
    src_mp1 = (DEST / "mp1-native/48000-m3-e0-plain.mp1").read_bytes()
    meta = (DEST / "mp1-native/48000-m3-e0-plain.meta").read_bytes()
    sizes = list(struct.unpack(">7I", meta[9:]))
    base_packets = []
    at = 0
    for s in sizes:
        base_packets.append(src_mp1[at : at + s])
        at += s
    packets = [base_packets[i % len(base_packets)] for i in range(56)]
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 1)
        + u("83", 2)
        + ebml("86", b"A_MPEG/L1")
        + ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1)),
    )
    blocks = u("e7", 0)
    for i, pkt in enumerate(packets):
        stamp = i * 8  # 384 / 48000 = 8 ms exact
        blocks += simple_block(1, stamp, 0x80, pkt)
    return header + ebml("18538067", info + ebml("1654ae6b", audio) + ebml("1f43b675", blocks))


def build_matroska_opus(channels, mapping_family, streams, coupled, mapping):
    # OpusHead: magic (8) + version (1) + channels (1) + pre_skip (2) + sample_rate (4) + gain (2) + family (1) + [streams, coupled, mapping...]
    head = (
        b"OpusHead"
        + bytes([1, channels])
        + struct.pack("<HIhB", 0, 48000, 0, mapping_family)
        + bytes([streams, coupled, *mapping])
    )
    # Minimal multistream Opus packet with `streams` elementary streams, each 20 ms silence TOC 0xf8fffe
    # For N streams: N-1 self-delimited frames (TOC=0xf8 + frame_size=2 + 2 frame bytes) + 1 final undelimited frame (3 bytes)
    self_delimited = b"\xf8\x02\xff\xfe"
    final_frame = b"\xf8\xff\xfe"
    pkt = self_delimited * (streams - 1) + final_frame
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 1)
        + u("83", 2)
        + ebml("86", b"A_OPUS")
        + ebml("63a2", head)
        + ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", channels)),
    )
    blocks = u("e7", 0)
    for i in range(4):
        blocks += simple_block(1, i * 20, 0x80, pkt)
    return header + ebml("18538067", info + ebml("1654ae6b", audio) + ebml("1f43b675", blocks))


def build_matroska_pcm_discrete_4ch():
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 1)
        + u("83", 2)
        + ebml("86", b"A_PCM/INT/LIT")
        + ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 4) + u("6264", 16)),
    )
    # 480 frames (10 ms) of 4-channel 16-bit PCM
    payload = b"".join(struct.pack("<hhhh", 4000, -4000, 2000, -2000) for _ in range(480))
    blocks = u("e7", 0) + simple_block(1, 0, 0x80, payload)
    return header + ebml("18538067", info + ebml("1654ae6b", audio) + ebml("1f43b675", blocks))


def build_mov_sowt_v1_zero_bits():
    # Minimal QuickTime MOV with sound v1 sample entry (`sowt`) where sample_size=16 in header
    # and SoundDescription v1 bytes_per_sample=0 (leaving Mp4Reader track.bit_depth == 0, which defaults to 16-bit in PcmDecoder)
    samples = [int(12000 * math.sin(2 * math.pi * 1000 * i / 48000)) for i in range(480)]
    mdat_payload = b"".join(struct.pack("<h", s) for s in samples)
    ftyp = box(b"ftyp", b"qt  \x00\x00\x02\x00qt  ")
    mdat = box(b"mdat", mdat_payload)
    mdat_offset = len(ftyp) + 8

    mvhd = box(
        b"mvhd",
        struct.pack(">IIIIII", 0, 0, 0, 48000, 480, 0x00010000)
        + struct.pack(">H", 0x0100)
        + bytes(10)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + bytes(24)
        + struct.pack(">I", 2),
    )
    tkhd = box(
        b"tkhd",
        struct.pack(">IIIIII", 0x00000007, 0, 0, 1, 0, 480)
        + bytes(8)
        + struct.pack(">HHHH", 0, 0, 0x0100, 0)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + struct.pack(">II", 0, 0),
    )
    mdhd = box(b"mdhd", struct.pack(">IIIIII", 0, 0, 0, 48000, 480, 0))
    hdlr = box(b"hdlr", struct.pack(">II", 0, 0) + b"soun" + bytes(12) + b"SoundHandler\x00")
    smhd = box(b"smhd", struct.pack(">II", 0, 0))
    dref = box(b"dref", struct.pack(">II", 0, 1) + box(b"url ", struct.pack(">I", 1)))
    dinf = box(b"dinf", dref)

    # Sound sample entry v1:
    # 6 reserved + 2 data_ref_index + 2 version(1) + 2 revision(0) + 4 vendor(0)
    # + 2 channels(1) + 2 sample_size(0) + 2 compression_id(0) + 2 packet_size(0) + 4 sample_rate(48000<<16)
    # + 4 samples_per_packet(1) + 4 bytes_per_packet(2) + 4 bytes_per_frame(2) + 4 bytes_per_sample(0)
    sowt_entry = box(
        b"sowt",
        bytes(6)
        + struct.pack(">HHHIHHHHI", 1, 1, 0, 0, 1, 0, 0, 0, 48000 << 16)
        + struct.pack(">IIII", 1, 2, 2, 0),
    )
    stsd = box(b"stsd", struct.pack(">II", 0, 1) + sowt_entry)
    stts = box(b"stts", struct.pack(">III", 0, 1, 480) + struct.pack(">I", 1))
    stsc = box(b"stsc", struct.pack(">IIIII", 0, 1, 1, 480, 1))
    stsz = box(b"stsz", struct.pack(">III", 0, 2, 480))
    stco = box(b"stco", struct.pack(">III", 0, 1, mdat_offset))
    stbl = box(b"stbl", stsd + stts + stsc + stsz + stco)
    minf = box(b"minf", smhd + dinf + stbl)
    mdia = box(b"mdia", mdhd + hdlr + minf)
    trak = box(b"trak", tkhd + mdia)
    moov = box(b"moov", mvhd + trak)
    return ftyp + mdat + moov


def generate():
    files = {
        "flac-loudness.mka": build_matroska_flac(),
        "flac-unassigned-mask.mka": build_matroska_flac_4ch_unassigned(),
        "mp1-loudness.mka": build_matroska_mp1(),
        "opus-quad-family1.mka": build_matroska_opus(4, 1, 2, 2, [0, 1, 2, 3]),
        "opus-discrete-family255-4ch.mka": build_matroska_opus(4, 255, 2, 2, [0, 1, 2, 3]),
        "pcm-discrete-4ch.mka": build_matroska_pcm_discrete_4ch(),
        "mov-sowt-zero-bits.mov": build_mov_sowt_v1_zero_bits(),
    }
    manifest = {
        "generator": "scripts/generate_container_loudness_plan_regressions.py",
        "files": {
            name: {
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            for name, data in sorted(files.items())
        },
    }
    return files, manifest


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    files, manifest = generate()
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Generated {len(files)} fixtures in {OUT}")


if __name__ == "__main__":
    main()
