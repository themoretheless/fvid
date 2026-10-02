#!/usr/bin/env python3
"""Write synthetic one-sample ALAC Matroska controls; no external codec tools."""
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
def element(tag, body):
    return bytes.fromhex(tag) + (len(body) | 0x10000000).to_bytes(4, "big") + body
cookie = bytearray(24)
cookie[:4] = (16).to_bytes(4, "big")
cookie[5:10] = bytes([16, 40, 10, 14, 1])
cookie[20:24] = (48000).to_bytes(4, "big")
# SCE, instance/reserved, explicit length, no low bits, raw sample, END.
bits = "000" + "0000" + "0" * 12 + "1" + "00" + "1" + f"{1:032b}" + f"{1:016b}" + "111"
bits += "0" * (-len(bits) % 8)
packet = int(bits, 2).to_bytes(len(bits) // 8, "big")
for label, rate in [("control", 48000), ("rate-mismatch", 44100)]:
    import struct
    audio = element("e1", element("b5", struct.pack(">d", rate)) + element("9f", b"\x01") + element("6264", b"\x10"))
    track = element("ae", element("d7", b"\x01") + element("83", b"\x02") + element("86", b"A_ALAC") + element("63a2", cookie) + audio)
    cluster = element("1f43b675", element("e7", b"\x00") + element("a3", b"\x81\x00\x00\x80" + packet))
    data = element("1a45dfa3", element("4282", b"matroska")) + element("18538067", element("1654ae6b", track) + cluster)
    (ROOT / f"alac-{label}.mka").write_bytes(data)
