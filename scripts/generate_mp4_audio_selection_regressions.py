#!/usr/bin/env python3
"""Purely author MP4/MOV and Matroska enabled/default audio track selection regression fixtures."""
import hashlib, json, math, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/mp4-audio-selection"


def boxes(data):
    at = 0
    while at < len(data):
        size, kind = struct.unpack_from(">I4s", data, at)
        assert size >= 8 and at + size <= len(data)
        yield kind, data[at + 8 : at + size]
        at += size
    assert at == len(data)


def pack_bits(bits):
    bits += "0" * (-len(bits) % 8)
    return bytes(int(bits[i : i + 8], 2) for i in range(0, len(bits), 8))


def descriptor(tag, payload):
    assert len(payload) < 128
    return bytes([tag, len(payload)]) + payload


def build_esds(asc):
    return bytes(4) + descriptor(
        3,
        b"\x00\x02\x00"
        + descriptor(4, b"\x40\x15" + bytes(11) + descriptor(5, asc))
        + descriptor(6, b"\x02"),
    )


def build_aac_packet(gain):
    bits = (
        "000"
        + "0000"
        + f"{gain:08b}"
        + "0"
        + "00"
        + "0"
        + "000001"
        + "0"
        + "0001"
        + "00001"
        + "0"
        + "0"
        + "0"
        + "0"
        + "0"
        + "111"
    )
    return pack_bits(bits)


def build_aac_pulse_packet(gain, bin_index, amplitude):
    assert 0 <= bin_index < 4 and 1 <= amplitude <= 15
    bits = (
        "000"
        + "0000"
        + f"{gain:08b}"
        + "0"
        + "00"
        + "0"
        + "000001"
        + "0"
        + "0001"
        + "00001"
        + "0"
        + "1"
        + "00"
        + "000000"
        + f"{bin_index:05b}"
        + f"{amplitude:04b}"
        + "0"
        + "0"
        + "0"
        + "111"
    )
    return pack_bits(bits)


def synthesize_analytical_pcm(frame_specs):
    n = 1024
    window = [math.sin(math.pi / (2 * n) * (i + 0.5)) for i in range(2 * n)]
    overlap = [0.0] * n
    pcm = bytearray()
    for gain, bin_index, amplitude in frame_specs:
        if amplitude == 0:
            scratch = [0.0] * (2 * n)
        else:
            mag = float(amplitude)
            spec = -(mag * (mag ** (1.0 / 3.0)) * (2.0 ** ((gain - 100.0) / 4.0)))
            spec_f32 = struct.unpack("<f", struct.pack("<f", spec))[0]
            scratch = [
                spec_f32
                * (2.0 / n)
                * math.cos(math.pi / n * (i + 0.5 + n / 2.0) * (bin_index + 0.5))
                * window[i]
                for i in range(2 * n)
            ]
        for i in range(n):
            sample = (overlap[i] + scratch[i]) / 65536.0
            pcm.extend(struct.pack("<f", sample))
            overlap[i] = scratch[n + i]
    return bytes(pcm)


def canonical_mvhd(body, next_id, timescale, duration):
    body = bytearray(body)
    body[12:16] = word(timescale)
    body[16:20] = word(duration)
    body[20:24] = word(65536)
    body[24:28] = word(0x01000000)
    for at, value in [(36, 65536), (52, 65536), (68, 1 << 30), (96, next_id)]:
        body[at : at + 4] = word(value)
    return bytes(body)


def rewrite_video_track(data, offset):
    out = b""
    for kind, body in boxes(data):
        if kind in [b"mdia", b"minf"]:
            body = rewrite_video_track(body, offset)
        elif kind == b"stbl":
            stbl_out = b""
            for sk, sb in boxes(body):
                if sk == b"stts":
                    sb = word(0, 1, 3, 1024)
                elif sk == b"stco":
                    sb = word(0, 1, offset)
                stbl_out += atom(sk, sb)
            body = stbl_out
        elif kind == b"tkhd":
            body = bytearray(body)
            body[:4] = word(3)
            body[12:16] = word(1)
            body[20:24] = word(3072)
            body = bytes(body)
        elif kind == b"mdhd":
            body = bytearray(body)
            body[12:20] = word(48000, 3072)
            body = bytes(body)
        out += atom(kind, body)
    return out


def make_audio_track(track_id, flags, codec, entry_payload, packets, offset, rate=48000, delta=1024):
    stbl = (
        atom(b"stsd", word(0, 1) + atom(codec, entry_payload))
        + atom(b"stts", word(0, 1, len(packets), delta))
        + atom(b"stsc", word(0, 1, 1, len(packets), 1))
        + atom(b"stsz", word(0, 0, len(packets), *map(len, packets)))
        + atom(b"stco", word(0, 1, offset))
    )
    dinf = atom(b"dinf", atom(b"dref", word(0, 1) + atom(b"url ", word(1))))
    smhd = atom(b"smhd", word(0, 0))
    mdhd = bytearray(24)
    mdhd[12:20] = word(rate, len(packets) * delta)
    mdia = atom(
        b"mdia",
        atom(b"mdhd", bytes(mdhd))
        + atom(b"hdlr", word(0, 0) + b"soun" + bytes(12))
        + atom(b"minf", smhd + dinf + atom(b"stbl", stbl)),
    )
    tkhd = bytearray(84)
    tkhd[:4] = word(flags)
    tkhd[12:16] = word(track_id)
    tkhd[20:24] = word(len(packets) * delta)
    tkhd[36:38] = struct.pack(">H", 0x0100)
    tkhd[40:44] = tkhd[56:60] = word(65536)
    tkhd[72:76] = word(0x40000000)
    return atom(b"trak", atom(b"tkhd", bytes(tkhd)) + mdia)


def mp4a_v0_header(channels=1, sample_rate=48000):
    entry = bytearray(28)
    entry[6:8] = b"\x00\x01"
    entry[8:10] = struct.pack(">H", 0)
    entry[16:20] = struct.pack(">HH", channels, 16)
    entry[24:28] = word(sample_rate << 16)
    return bytes(entry)


def sowt_v0_header(channels=1, sample_rate=8000):
    entry = bytearray(28)
    entry[6:8] = b"\x00\x01"
    entry[8:10] = struct.pack(">H", 0)
    entry[16:20] = struct.pack(">HH", channels, 16)
    entry[24:28] = word(sample_rate << 16)
    return bytes(entry)


def generate():
    vp9_packet = frame(8, "420", 16, 16)
    video_seed = dict(boxes(mp4([vp9_packet] * 3, 8, "420", (16, 16), [1, 2, 3])))
    ftyp = atom(b"ftyp", video_seed[b"ftyp"])
    video_mdat = video_seed[b"mdat"]
    video_moov = dict(boxes(video_seed[b"moov"]))

    frame_specs = [(100, 1, 8), (96, 2, 4), (100, 0, 0)]
    active_aac_packets = [
        build_aac_pulse_packet(100, 1, 8),
        build_aac_pulse_packet(96, 2, 4),
        build_aac_packet(100),
    ]
    silent_aac_packets = [build_aac_packet(100)] * 3
    expected_aac_pcm = synthesize_analytical_pcm(frame_specs)
    expected_silent_aac_pcm = bytes(3072 * 4)

    asc = b"\x11\x88"
    esds_box = atom(b"esds", build_esds(asc))
    mp4a_payload = mp4a_v0_header(1, 48000) + esds_box

    sowt_payload = sowt_v0_header(1, 8000)
    active_sowt_packets = []
    expected_sowt_pcm = bytearray()
    for index in range(3):
        samples = [((index * 160 + i) * 37) % 32768 - 16384 for i in range(160)]
        active_sowt_packets.append(b"".join(struct.pack("<h", v) for v in samples))
        for value in samples:
            expected_sowt_pcm += struct.pack("<f", value / 32768.0)
    expected_sowt_pcm = bytes(expected_sowt_pcm)
    silent_sowt_packets = [bytes(320)] * 3

    def make_dual_audio_mp4(flags_pair, codec, entry_payload, packets_pair, rate, delta):
        payload_0 = b"".join(packets_pair[0])
        payload_1 = b"".join(packets_pair[1])
        mdat = atom(b"mdat", video_mdat + payload_0 + payload_1)
        video_offset = len(ftyp) + 8
        offset_0 = video_offset + len(video_mdat)
        offset_1 = offset_0 + len(payload_0)
        v_trak = atom(b"trak", rewrite_video_track(video_moov[b"trak"], video_offset))
        a_trak_0 = make_audio_track(
            2, flags_pair[0], codec, entry_payload, packets_pair[0], offset_0, rate, delta
        )
        a_trak_1 = make_audio_track(
            3, flags_pair[1], codec, entry_payload, packets_pair[1], offset_1, rate, delta
        )
        mvhd = atom(
            b"mvhd", canonical_mvhd(video_moov[b"mvhd"], 4, rate, len(packets_pair[0]) * delta)
        )
        return ftyp + mdat + atom(b"moov", mvhd + v_trak + a_trak_0 + a_trak_1)

    def make_dual_audio_only_mp4(flags_pair, codec, entry_payload, packets_pair, rate, delta):
        payload_0 = b"".join(packets_pair[0])
        payload_1 = b"".join(packets_pair[1])
        mdat = atom(b"mdat", payload_0 + payload_1)
        offset_0 = len(ftyp) + 8
        offset_1 = offset_0 + len(payload_0)
        a_trak_0 = make_audio_track(
            1, flags_pair[0], codec, entry_payload, packets_pair[0], offset_0, rate, delta
        )
        a_trak_1 = make_audio_track(
            2, flags_pair[1], codec, entry_payload, packets_pair[1], offset_1, rate, delta
        )
        mvhd = atom(
            b"mvhd", canonical_mvhd(video_moov[b"mvhd"], 3, rate, len(packets_pair[0]) * delta)
        )
        return ftyp + mdat + atom(b"moov", mvhd + a_trak_0 + a_trak_1)

    files = {
        "frame.vp9": vp9_packet,
        "expected-aac.f32le": expected_aac_pcm,
        "expected-silent-aac.f32le": expected_silent_aac_pcm,
        "expected-sowt.f32le": expected_sowt_pcm,
        "disabled-first-aac.mp4": make_dual_audio_mp4(
            (6, 3), b"mp4a", mp4a_payload, (silent_aac_packets, active_aac_packets), 48000, 1024
        ),
        "disabled-first-audio-only.mp4": make_dual_audio_only_mp4(
            (6, 3), b"mp4a", mp4a_payload, (silent_aac_packets, active_aac_packets), 48000, 1024
        ),
        "enabled-nonmovie-first-aac.mp4": make_dual_audio_mp4(
            (1, 3), b"mp4a", mp4a_payload, (silent_aac_packets, active_aac_packets), 48000, 1024
        ),
        "disabled-first-sowt.mp4": make_dual_audio_mp4(
            (6, 3), b"sowt", sowt_payload, (silent_sowt_packets, active_sowt_packets), 8000, 160
        ),
        "both-enabled-aac.mp4": make_dual_audio_mp4(
            (3, 3), b"mp4a", mp4a_payload, (silent_aac_packets, active_aac_packets), 48000, 1024
        ),
    }

    manifest = {
        "generator": "scripts/generate_mp4_audio_selection_regressions.py",
        "controls": [
            {
                "file": "disabled-first-aac.mp4",
                "codec": "aac",
                "flags": [6, 3],
                "selected_stream_index": 2,
                "sample_rate": 48000,
                "channels": 1,
                "sample_frames": 3072,
                "expected_pcm": "expected-aac.f32le",
            },
            {
                "file": "enabled-nonmovie-first-aac.mp4",
                "codec": "aac",
                "flags": [1, 3],
                "selected_stream_index": 2,
                "sample_rate": 48000,
                "channels": 1,
                "sample_frames": 3072,
                "expected_pcm": "expected-aac.f32le",
            },
            {
                "file": "disabled-first-sowt.mp4",
                "codec": "pcm_s16le",
                "flags": [6, 3],
                "selected_stream_index": 2,
                "sample_rate": 8000,
                "channels": 1,
                "sample_frames": 480,
                "expected_pcm": "expected-sowt.f32le",
            },
        ],
        "ambiguous": [
            {
                "file": "both-enabled-aac.mp4",
                "flags": [3, 3],
                "expected_error": "native audio export requires exactly one audio track or an explicit stream index",
            }
        ],
        "files": {},
    }
    for name, body in sorted(files.items()):
        manifest["files"][name] = {
            "bytes": len(body),
            "sha256": hashlib.sha256(body).hexdigest(),
        }
    files["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return files


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, body in generate().items():
        (OUT / name).write_bytes(body)


if __name__ == "__main__":
    main()
