#!/usr/bin/env python3
"""Author a three-frame raw video for HTTP body read-fault injection."""
import hashlib
import json
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"


def main():
    header = b"YUV4MPEG2 W2 H2 F30:1 Ip C420jpeg\n"
    data = header + b"".join(b"FRAME\n" + bytes([32 + i * 16]) * 4
                             + bytes([120, 136]) for i in range(3))
    name = "http-interrupted-body.y4m"
    (OUT / name).write_bytes(data)
    (OUT / "http-interrupted-body.json").write_text(json.dumps({
        "file": name, "sha256": hashlib.sha256(data).hexdigest(),
        "interrupt_at": len(header) + 8, "frames": 3,
        "provenance": "Three authored progressive YUV420 frames; reader interruption is injected after a partial body download. No private media, network or external tools.",
    }, indent=2) + "\n")
    budget_data = data.replace(b"F30:1", b"F24:1")
    (OUT / "http-byte-limit.y4m").write_bytes(budget_data)
    (OUT / "http-byte-limit.json").write_text(json.dumps({
        "file": "http-byte-limit.y4m", "sha256": hashlib.sha256(budget_data).hexdigest(),
        "frames": 3, "reproduction": "Unknown-length body read exceeds max_bytes by an entire chunk before refusing; reader must consume at most the budget plus one overflow-detection byte.",
        "provenance": "Three authored progressive YUV420 frames; no private media, network or external tools.",
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
