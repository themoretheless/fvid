#!/usr/bin/env python3
"""Purely author MP4/MOV AAC sample-description fixtures (ISO v0, QuickTime SoundDescription v1, QuickTime v2, and wave/esds nesting) with analytical IMDCT PCM reference."""
import hashlib, json, math, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/mp4-aac-quicktime"


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
    # Mono AAC-LC raw_data_block:
    # id_syn_ele = 0 (SCE, 3b), element_instance_tag = 0 (4b), global_gain = gain (8b),
    # ics_reserved_bit = 0 (1b), window_sequence = 0 (ONLY_LONG, 2b), window_shape = 0 (SINE, 1b),
    # max_sfb = 1 (6b, band 0 = bins 0..4), predictor_data_present = 0 (1b),
    # section_cb = 1 (4b), section_len = 1 (5b),
    # scalefactor delta = 0 (codeword '0', 1b -> scalefactor = gain),
    # pulse_data_present = 0 (1b),
    # tns_data_present = 0 (1b), gain_control_data_present = 0 (1b),
    # spectral codebook 1 quad (0,0,0,0) = index 40 -> codeword '0' (1b),
    # id_syn_ele = 7 (TERM, 3b).
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
    # Analytical 1024-sample AAC-LC ONLY_LONG SINE window IMDCT + overlap-add:
    # For quantized coefficient q <= 0 (since pulse subtracts from 0 -> -amplitude):
    # x_quant = -(|q|^(4/3)) * 2^((gain - 100)/4)
    # IMDCT_2048[i] = (2 / 1024) * x_quant * cos(pi / 1024 * (i + 0.5 + 512) * (k + 0.5))
    # Windowed[i] = IMDCT_2048[i] * sin(pi / 2048 * (i + 0.5))
    # Normalized PCM[n] = (overlap[n] + Windowed[n]) / 65536.0
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


def canonical_mvhd(body, next_id, duration):
    body = bytearray(body)
    body[12:16] = word(48000)
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


def make_audio_track(track_id, mp4a_payload, packets, offset):
    stbl = (
        atom(b"stsd", word(0, 1) + atom(b"mp4a", mp4a_payload))
        + atom(b"stts", word(0, 1, len(packets), 1024))
        + atom(b"stsc", word(0, 1, 1, len(packets), 1))
        + atom(b"stsz", word(0, 0, len(packets), *map(len, packets)))
        + atom(b"stco", word(0, 1, offset))
    )
    dinf = atom(b"dinf", atom(b"dref", word(0, 1) + atom(b"url ", word(1))))
    smhd = atom(b"smhd", word(0, 0))
    mdhd = bytearray(24)
    mdhd[12:20] = word(48000, len(packets) * 1024)
    mdia = atom(
        b"mdia",
        atom(b"mdhd", bytes(mdhd))
        + atom(b"hdlr", word(0, 0) + b"soun" + bytes(12))
        + atom(b"minf", smhd + dinf + atom(b"stbl", stbl)),
    )
    tkhd = bytearray(84)
    tkhd[:4] = word(3)
    tkhd[12:16] = word(track_id)
    tkhd[20:24] = word(len(packets) * 1024)
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


def mp4a_v1_header(channels=1, sample_rate=48000):
    entry = bytearray(mp4a_v0_header(channels, sample_rate))
    entry[8:10] = struct.pack(">H", 1)
    return bytes(entry) + struct.pack(">IIII", 1024, 0, 0, 2)


def mp4a_v2_header(channels=1, sample_rate=48000.0):
    entry = bytearray(64)
    entry[6:8] = b"\x00\x01"
    entry[8:10] = struct.pack(">H", 2)
    entry[16:20] = struct.pack(">HH", 3, 16)
    entry[24:28] = word(0x00010000)
    entry[28:32] = word(72)
    entry[32:40] = struct.pack(">d", float(sample_rate))
    entry[40:44] = word(channels)
    entry[44:48] = word(0x7F000000)
    entry[48:64] = word(16, 0, 0, 1024)
    return bytes(entry)


def generate():
    vp9_packet = frame(8, "420", 16, 16)
    video_seed = dict(boxes(mp4([vp9_packet] * 3, 8, "420", (16, 16), [1, 2, 3])))
    ftyp = atom(b"ftyp", video_seed[b"ftyp"])
    video_mdat = video_seed[b"mdat"]
    video_moov = dict(boxes(video_seed[b"moov"]))

    # 3 mono 48 kHz AAC-LC frames (3072 samples total):
    # Frame 0: pulse at bin 1, amplitude 8, gain 100
    # Frame 1: pulse at bin 2, amplitude 4, gain 96
    # Frame 2: zero spectrum (flushes Frame 1 overlap), gain 100
    frame_specs = [(100, 1, 8), (96, 2, 4), (100, 0, 0)]
    aac_packets = [
        build_aac_pulse_packet(100, 1, 8),
        build_aac_pulse_packet(96, 2, 4),
        build_aac_packet(100),
    ]
    audio_payload = b"".join(aac_packets)
    expected_pcm = synthesize_analytical_pcm(frame_specs)

    asc = b"\x11\x88"  # AOT 2 (AAC-LC), freq_index 3 (48000 Hz), channel_config 1 (mono)
    esds_box = atom(b"esds", build_esds(asc))
    wave_box = atom(b"wave", atom(b"frma", b"mp4a") + esds_box + bytes(4)) + bytes(4)

    files = {
        "frame.vp9": vp9_packet,
        "expected.f32le": expected_pcm,
    }
    controls = []
    cases = []

    def make_av_file(mp4a_payload):
        mdat = atom(b"mdat", video_mdat + audio_payload)
        video_offset = len(ftyp) + 8
        audio_offset = video_offset + len(video_mdat)
        v_trak = atom(b"trak", rewrite_video_track(video_moov[b"trak"], video_offset))
        a_trak = make_audio_track(2, mp4a_payload, aac_packets, audio_offset)
        mvhd = atom(b"mvhd", canonical_mvhd(video_moov[b"mvhd"], 3, 3072))
        return ftyp + mdat + atom(b"moov", mvhd + v_trak + a_trak)

    def make_audio_only_file(mp4a_payload):
        mdat = atom(b"mdat", audio_payload)
        audio_offset = len(ftyp) + 8
        a_trak = make_audio_track(1, mp4a_payload, aac_packets, audio_offset)
        mvhd = atom(b"mvhd", canonical_mvhd(video_moov[b"mvhd"], 2, 3072))
        return ftyp + mdat + atom(b"moov", mvhd + a_trak)

    for name, mode, payload, has_video in [
        ("iso-v0-direct.mp4", "v0-direct-esds", mp4a_v0_header(1, 48000) + esds_box, True),
        ("qt-v1-wave.mov", "v1-wave-esds-stale-stereo", mp4a_v1_header(2, 24000) + wave_box, True),
        ("qt-v1-direct.mov", "v1-direct-esds", mp4a_v1_header(1, 48000) + esds_box, True),
        ("qt-v0-wave.mov", "v0-wave-esds", mp4a_v0_header(1, 48000) + wave_box, True),
        ("qt-v2-wave.mov", "v2-wave-esds", mp4a_v2_header(1, 48000.0) + wave_box, True),
        ("qt-v1-audio-only.mov", "v1-wave-audio-only", mp4a_v1_header(1, 48000) + wave_box, False),
    ]:
        files[name] = make_av_file(payload) if has_video else make_audio_only_file(payload)
        controls.append(dict(file=name, mode=mode, has_video=has_video))

    for name, payload, has_video, expected_error in [
        (
            "truncated-v1.mov",
            mp4a_v1_header(1, 48000)[:36],
            False,
            "truncated AAC version-1 sample description",
        ),
        (
            "wave-missing-esds.mov",
            mp4a_v1_header(1, 48000) + atom(b"wave", atom(b"frma", b"mp4a")),
            False,
            "missing MP4 esds box",
        ),
        (
            "direct-missing-esds.mp4",
            mp4a_v1_header(1, 48000),
            False,
            "missing MP4 wave box",
        ),
    ]:
        files[name] = make_av_file(payload) if has_video else make_audio_only_file(payload)
        cases.append(dict(file=name, expected_error=expected_error))

    manifest = dict(
        generator="scripts/generate_mp4_aac_quicktime_regressions.py",
        spec="https://developer.apple.com/documentation/quicktime-file-format/sound_sample_description_version_1",
        controls=controls,
        cases=cases,
        sha256={k: hashlib.sha256(v).hexdigest() for k, v in sorted(files.items())},
    )
    files["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return files


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    for k, v in generate().items():
        (OUT / k).write_bytes(v)
