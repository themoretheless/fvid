#!/usr/bin/env python3
"""Generate synthetic colour LUT and HDR mastering display regression fixtures."""

from __future__ import annotations

import json
import struct
from pathlib import Path

FIXTURE_DIR = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "playback-errors" / "color"


def build_3dl_nan_node_text() -> str:
    """2x2x2 .3dl LUT containing a NaN node value."""
    lines = ["2"]
    for i in range(8):
        if i == 3:
            lines.append("NaN 2048 2048")
        else:
            lines.append("0 0 0")
    return "\n".join(lines) + "\n"


def build_cube_inverted_domain_text() -> str:
    """2x2x2 .cube LUT with DOMAIN_MIN >= DOMAIN_MAX."""
    return (
        "LUT_3D_SIZE 2\n"
        "DOMAIN_MIN 1.0 0.0 0.0\n"
        "DOMAIN_MAX 0.0 1.0 1.0\n"
        "0 0 0\n"
        "1 0 0\n"
        "0 1 0\n"
        "1 1 0\n"
        "0 0 1\n"
        "1 0 1\n"
        "0 1 1\n"
        "1 1 1\n"
    )


def build_spi1d_inverted_from_text() -> str:
    """.spi1d LUT with inverted From range."""
    return (
        "Version 1\n"
        "From 1.0 0.0\n"
        "Length 3\n"
        "Components 1\n"
        "{\n"
        "0.0\n"
        "0.5\n"
        "1.0\n"
        "}\n"
    )


def build_1d_lut_between_grid_spike_cube_text() -> str:
    """33-entry 1D .cube LUT that matches identity at all 17 1/16th steps (even indices)
    but has a +0.5 spike at odd index 1 (x = 1/32 = 0.03125)."""
    lines = ["LUT_1D_SIZE 33"]
    for i in range(33):
        v = i / 32.0
        if i == 1:
            lines.append(f"{v + 0.5:.6f} {v:.6f} {v:.6f}")
        else:
            lines.append(f"{v:.6f} {v:.6f} {v:.6f}")
    return "\n".join(lines) + "\n"


def build_zero_mdcv_payload() -> bytes:
    """24-byte all-zero placeholder MDCV payload."""
    return b"\x00" * 24


def build_collinear_mdcv_payload() -> bytes:
    """24-byte MDCV payload where R, G, and B primaries are collinear (all at 0.3, 0.3)."""
    # ST 2086 units: 0.3 / 0.00002 = 15000, white D65 = (15635, 16450), max=1000.0 (10_000_000), min=0.0001 (1)
    return struct.pack(
        ">HHHHHHHHII",
        15000, 15000,  # green
        15000, 15000,  # blue
        15000, 15000,  # red
        15635, 16450,  # white
        10_000_000,
        1,
    )


def generate_fixtures(out_dir: Path = FIXTURE_DIR) -> dict[str, str]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "nan_node.3dl": build_3dl_nan_node_text().encode("utf-8"),
        "inverted_domain.cube": build_cube_inverted_domain_text().encode("utf-8"),
        "inverted_from.spi1d": build_spi1d_inverted_from_text().encode("utf-8"),
        "spike_between_probes_1d.cube": build_1d_lut_between_grid_spike_cube_text().encode("utf-8"),
        "zero_placeholder.mdcv": build_zero_mdcv_payload(),
        "collinear_primaries.mdcv": build_collinear_mdcv_payload(),
    }
    manifest = {}
    for name, data in files.items():
        path = out_dir / name
        path.write_bytes(data)
        manifest[name] = {
            "size": len(data),
        }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return {k: str(out_dir / k) for k in files}


if __name__ == "__main__":
    generate_fixtures()
