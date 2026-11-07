#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 580 (FVID-0630..0634):
1. wav-extensible-alaw-16bit-storage.wav: WAVEFORMATEXTENSIBLE G.711 A-law with
   invalid wBitsPerSample=16 and wValidBitsPerSample=8.
2. flac-title-and-language.flac: FLAC with Vorbis comments TITLE and LANGUAGE.
3. mp3-title-and-language.mp3: MP3 with ID3v2.3 TIT2 and TLAN frames.
4. smf-nul-padded-conductor-name.mid: Format-1 SMF whose conductor track has a
   NUL-only FF 03 name (b"\\x00\\x00") and whose musical track has a NUL-terminated
   FF 03 name (b"Lead Synth\\x00").
5. xm-fxx-tempo-change-tail.xm: FastTracker II XM module that starts at speed=6,
   bpm=125 and changes tempo on its single row via F03 / F96 (speed=3, bpm=150)
   so the release tail packet must carry speed=3, bpm=150.
"""

from pathlib import Path
import struct

OUT_DIR = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "playback-errors" / "wav-flac-mp3-smf-xm-playback"


def build_wav_extensible_alaw_16bit_storage() -> bytes:
    subformat = struct.pack(
        "<IHH8s",
        0x00000006,  # WAVE_FORMAT_ALAW
        0x0000,
        0x0010,
        b"\x80\x00\x00\xaa\x00\x38\x9b\x71",
    )
    # wFormatTag=0xFFFE, nChannels=1, nSamplesPerSec=8000, nAvgBytesPerSec=16000,
    # nBlockAlign=2, wBitsPerSample=16 (invalid for G.711!), cbSize=22,
    # wValidBitsPerSample=8, dwChannelMask=4
    fmt_body = struct.pack(
        "<HHIIHHHHHII",
        0xFFFE,
        1,
        8000,
        16000,
        2,
        16,
        22,
        8,
        0,
        0,
        0,
    )[:18] + struct.pack("<HI", 8, 4) + subformat
    assert len(fmt_body) == 40
    fact_body = struct.pack("<I", 4)
    data_body = b"\xd5\xd5\xd5\xd5\xd5\xd5\xd5\xd5"
    riff_payload = (
        b"WAVE"
        + b"fmt " + struct.pack("<I", len(fmt_body)) + fmt_body
        + b"fact" + struct.pack("<I", len(fact_body)) + fact_body
        + b"data" + struct.pack("<I", len(data_body)) + data_body
    )
    return b"RIFF" + struct.pack("<I", len(riff_payload)) + riff_payload


def build_flac_title_and_language() -> bytes:
    # Base minimal valid FLAC from tests/fixtures/flac/tone.flac with injected
    # VORBIS_COMMENT block carrying empty first TITLE, real TITLE, and LANGUAGE.
    repo_root = Path(__file__).resolve().parent.parent
    tone_flac = (repo_root / "tests" / "fixtures" / "flac" / "tone.flac").read_bytes()
    assert tone_flac[:4] == b"fLaC"
    # First block is STREAMINFO (34 bytes). Clear its is-last bit and append our
    # last VORBIS_COMMENT block right before the audio frames.
    # Find end of metadata blocks in tone.flac:
    pos = 4
    while True:
        header = tone_flac[pos]
        is_last = (header & 0x80) != 0
        block_type = header & 0x7F
        length = int.from_bytes(tone_flac[pos + 1 : pos + 4], "big")
        if block_type == 0:
            streaminfo = tone_flac[pos + 4 : pos + 4 + length]
        pos += 4 + length
        if is_last:
            break
    audio_frames = tone_flac[pos:]

    vendor = b"fvid-test"
    comments = [
        b"TITLE=",
        b"TITLE=FLAC Symphony",
        b"LANGUAGE=deu",
    ]
    vc = struct.pack("<I", len(vendor)) + vendor + struct.pack("<I", len(comments))
    for c in comments:
        vc += struct.pack("<I", len(c)) + c

    streaminfo_hdr = bytes([0x00]) + len(streaminfo).to_bytes(3, "big")
    vc_hdr = bytes([0x84]) + len(vc).to_bytes(3, "big")
    return b"fLaC" + streaminfo_hdr + streaminfo + vc_hdr + vc + audio_frames


def synchsafe(n: int) -> bytes:
    return bytes([
        (n >> 21) & 0x7F,
        (n >> 14) & 0x7F,
        (n >> 7) & 0x7F,
        n & 0x7F,
    ])


def id3v23_frame(frame_id: bytes, payload: bytes) -> bytes:
    return frame_id + struct.pack(">I", len(payload)) + b"\x00\x00" + payload


def build_mp3_title_and_language() -> bytes:
    frames = (
        id3v23_frame(b"TIT2", b"\x03MP3 Concerto\x00")
        + id3v23_frame(b"TLAN", b"\x03fra\x00")
    )
    id3 = b"ID3\x03\x00\x00" + synchsafe(len(frames)) + frames
    # MPEG1 Layer III, 128 kbps, 44100 Hz, mono (417 bytes per frame)
    mpeg_frame = bytes([0xFF, 0xFB, 0x90, 0xC4]) + b"\x00" * (417 - 4)
    return id3 + mpeg_frame * 2


def build_smf_nul_padded_conductor_name() -> bytes:
    header = b"MThd" + struct.pack(">IHHH", 6, 1, 2, 480)
    # Track 0: conductor track with FF 03 len=2 "\0\0", then End of Track
    trk0_events = (
        b"\x00\xff\x03\x02\x00\x00"
        b"\x00\xff\x2f\x00"
    )
    trk0 = b"MTrk" + struct.pack(">I", len(trk0_events)) + trk0_events
    # Track 1: musical track with FF 03 "Lead Synth\0", NoteOn at 0, NoteOff at 120, EOT
    name_payload = b"Lead Synth\x00"
    trk1_events = (
        b"\x00\xff\x03" + bytes([len(name_payload)]) + name_payload
        + b"\x00\x90\x3c\x64"
        + b"\x78\x80\x3c\x00"
        + b"\x00\xff\x2f\x00"
    )
    trk1 = b"MTrk" + struct.pack(">I", len(trk1_events)) + trk1_events
    return header + trk0 + trk1


def field(text: str, width: int) -> bytes:
    raw = text.encode("ascii")[:width]
    return raw + b"\x00" * (width - len(raw))


def build_xm_fxx_tempo_change_tail() -> bytes:
    # 2 channels, 1 pattern with 1 row, 1 instrument with 1 8-bit sample.
    # Initial speed=6, bpm=125.
    # Row 0:
    #   ch0: note=49 (C-4), inst=1, vol=0, effect=0x0F ('F'), param=0x03 (speed = 3)
    #   ch1: note=0, inst=0, vol=0, effect=0x0F ('F'), param=0x96 (bpm = 150)
    out = bytearray()
    out += b"Extended Module: "
    out += field("fvid tail tempo", 20)
    out.append(0x1A)
    out += field("FastTracker II v2.00", 20)
    out += struct.pack("<H", 0x0104)   # version
    out += struct.pack("<I", 276)      # header size
    out += struct.pack("<H", 1)        # song length (1 order)
    out += struct.pack("<H", 0)        # restart position
    out += struct.pack("<H", 2)        # channels = 2
    out += struct.pack("<H", 1)        # patterns = 1
    out += struct.pack("<H", 1)        # instruments = 1
    out += struct.pack("<H", 1)        # linear freq table
    out += struct.pack("<H", 6)        # default tempo (speed = 6)
    out += struct.pack("<H", 125)      # default BPM = 125
    assert len(out) == 80
    orders = bytearray(256)
    orders[0] = 0
    out += orders

    # Pattern 0: 1 row, 2 channels, unpacked
    pat_data = bytes([
        49, 1, 0, 0x0F, 0x03,  # ch0: C-4, inst 1, F03 (speed 3)
        0,  0, 0, 0x0F, 0x96,  # ch1: F96 (BPM 150)
    ])
    out += struct.pack("<IBHH", 9, 0, 1, len(pat_data))
    out += pat_data

    # Instrument 1: 1 sample
    inst = bytearray()
    inst += struct.pack("<I", 243)
    inst += field("Lead", 22)
    inst.append(0)
    inst += struct.pack("<H", 1)       # 1 sample
    inst += struct.pack("<I", 40)      # sample header size
    inst += bytes(96)                  # keymap
    inst += bytes(96)                  # envelopes
    inst += bytes(233 - len(inst))
    inst += struct.pack("<H", 0)       # envelope flags
    inst += bytes(4)
    inst += struct.pack("<H", 0)       # fadeout
    inst += struct.pack("<H", 0)       # reserved
    assert len(inst) == 243

    sample_frames = bytes([32, 192] * 8)  # 16 delta bytes
    samp_hdr = bytearray()
    samp_hdr += struct.pack("<III", len(sample_frames), 0, 0)
    samp_hdr += bytes([64, 0, 0, 128, 0, 0])
    samp_hdr += field("sq", 22)
    assert len(samp_hdr) == 40

    out += inst + samp_hdr + sample_frames
    return bytes(out)


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "wav-extensible-alaw-16bit-storage.wav").write_bytes(
        build_wav_extensible_alaw_16bit_storage()
    )
    (OUT_DIR / "flac-title-and-language.flac").write_bytes(
        build_flac_title_and_language()
    )
    (OUT_DIR / "mp3-title-and-language.mp3").write_bytes(
        build_mp3_title_and_language()
    )
    (OUT_DIR / "smf-nul-padded-conductor-name.mid").write_bytes(
        build_smf_nul_padded_conductor_name()
    )
    (OUT_DIR / "xm-fxx-tempo-change-tail.xm").write_bytes(
        build_xm_fxx_tempo_change_tail()
    )


if __name__ == "__main__":
    main()
