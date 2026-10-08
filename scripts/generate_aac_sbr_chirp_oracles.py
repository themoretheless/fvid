#!/usr/bin/env python3
"""Explicit, offline Decimal references for SBR 6.18.6.2/table 175.
Original mode sequences; numeric DSP oracle, not an encoded HE-AAC fixture.
"""
from decimal import Decimal, localcontext
from pathlib import Path
import hashlib, json, struct
root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
targets = [["0", "0.6", "0.9", "0.98"], ["0.6", "0.75", "0.9", "0.98"],
           ["0", "0.75", "0.9", "0.98"], ["0", "0.75", "0.9", "0.98"]]
data = bytearray()
with localcontext() as ctx:
    ctx.prec = 80
    for sequence in range(1024):
        previous_mode, bandwidth = 0, Decimal(0)
        modes = [(sequence >> (2 * (4-frame))) & 3 for frame in range(5)] + [0]*8
        for mode in modes:
            target = Decimal(targets[previous_mode][mode])
            weight = Decimal("0.75") if target < bandwidth else Decimal("0.90625")
            bandwidth = weight*target + (1-weight)*bandwidth
            if bandwidth < Decimal("0.015625"):
                bandwidth = Decimal(0)
            data.extend(struct.pack("<d", float(bandwidth)))
            previous_mode = mode
name = "aac-sbr-chirp-decimal.f64le"
(root/name).write_bytes(data)
(root/"aac-sbr-chirp-oracles.json").write_text(json.dumps({"source": "GOST R53556.4-2013 6.18.6.2/table 175", "sequences": 1024, "frames": 13, "precision": 80, "file": name, "sha256": hashlib.sha256(data).hexdigest()}, indent=2)+"\n")
