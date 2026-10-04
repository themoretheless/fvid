#!/usr/bin/env python3
"""Write a short synthetic float WAVE for native packet-control acceptance."""
from pathlib import Path
import struct

output = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/audio-packet-controls.wav"
payload = b"".join(struct.pack("<f", index / 1000.0) for index in range(1000))
fmt = struct.pack("<HHIIHH", 3, 1, 48000, 192000, 4, 32)
body = b"WAVEfmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(payload)) + payload
output.write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)
