#!/usr/bin/env python3
"""Purely author AVI duplicate header/format, idx1 truncation/alignment, and delayed-start PCM seek regression fixtures."""
import hashlib, json, struct
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/avi-headers-seek"


def chunk(tag, body):
    pad = b"\x00" if (len(body) & 1) else b""
    return tag + struct.pack("<I", len(body)) + body + pad


def list_chunk(kind, children):
    return chunk(b"LIST", kind + b"".join(children))


def avih(streams):
    data = bytearray(56)
    struct.pack_into("<I", data, 24, streams)
    return bytes(data)


def strh(kind, scale, rate, start, length):
    data = bytearray(48)
    data[:4] = kind
    data[4:8] = b"\x01\x00\x00\x00"
    struct.pack_into("<IIIII", data, 20, scale, rate, start, length, 1024)
    struct.pack_into("<II", data, 40, 0xFFFFFFFF, 2)
    return bytes(data)


def pcm_strf(channels=1, rate=48000, bits=16):
    align = channels * (bits // 8)
    bps = rate * align
    return struct.pack("<HHIIHH", 1, channels, rate, bps, align, bits)


def build_avi(hdrl_children, movi_payload, idx1_chunk=None):
    hdrl = list_chunk(b"hdrl", hdrl_children)
    movi = chunk(b"LIST", b"movi" + movi_payload)
    body = b"AVI " + hdrl + movi + (idx1_chunk if idx1_chunk is not None else b"")
    return chunk(b"RIFF", body)


def generate():
    pcm_samples = [100, -100, 200, -200, 300, -300, 400, -400]
    pcm_payload = b"".join(struct.pack("<h", s) for s in pcm_samples)
    rec0 = chunk(b"00wb", pcm_payload[:8])
    rec1 = chunk(b"00wb", pcm_payload[8:])
    movi_payload = rec0 + rec1
    # idx1 offsets relative to movi start (which is 4 bytes before rec0)
    idx1_entries = (
        b"00wb" + struct.pack("<III", 16, 4, 8)
        + b"00wb" + struct.pack("<III", 16, 4 + len(rec0), 8)
    )

    files = {}
    controls = []
    cases = []

    # Control 1: Delayed-start PCM stream (dwStart=100 samples, 2 records of 4 frames each)
    delayed_hdrl = [
        chunk(b"avih", avih(1)),
        list_chunk(
            b"strl",
            [
                chunk(b"strh", strh(b"auds", 1, 48000, 100, 8)),
                chunk(b"strf", pcm_strf(1, 48000, 16)),
            ],
        ),
    ]
    files["valid-delayed-pcm.avi"] = build_avi(
        delayed_hdrl, movi_payload, chunk(b"idx1", idx1_entries)
    )
    controls.append(
        dict(
            file="valid-delayed-pcm.avi",
            mode="delayed-start-pcm-seek-and-playback",
        )
    )

    # Invalid 1: Duplicate avih in hdrl
    dup_avih_hdrl = [
        chunk(b"avih", avih(2)),
        chunk(b"avih", avih(1)),
        list_chunk(
            b"strl",
            [
                chunk(b"strh", strh(b"auds", 1, 48000, 0, 8)),
                chunk(b"strf", pcm_strf(1, 48000, 16)),
            ],
        ),
    ]
    files["duplicate-avih.avi"] = build_avi(
        dup_avih_hdrl, movi_payload, chunk(b"idx1", idx1_entries)
    )
    cases.append(
        dict(
            file="duplicate-avih.avi",
            expected_error="duplicate AVI main header",
        )
    )

    # Invalid 2: Duplicate strh in strl
    dup_strh_hdrl = [
        chunk(b"avih", avih(1)),
        list_chunk(
            b"strl",
            [
                chunk(b"strh", strh(b"auds", 1, 48000, 0, 8)),
                chunk(b"strh", strh(b"vids", 1, 25, 0, 8)),
                chunk(b"strf", pcm_strf(1, 48000, 16)),
            ],
        ),
    ]
    files["duplicate-strh.avi"] = build_avi(
        dup_strh_hdrl, movi_payload, chunk(b"idx1", idx1_entries)
    )
    cases.append(
        dict(
            file="duplicate-strh.avi",
            expected_error="duplicate AVI stream header",
        )
    )

    # Invalid 3: Duplicate strf in strl
    dup_strf_hdrl = [
        chunk(b"avih", avih(1)),
        list_chunk(
            b"strl",
            [
                chunk(b"strh", strh(b"auds", 1, 48000, 0, 8)),
                chunk(b"strf", pcm_strf(1, 48000, 16)),
                chunk(b"strf", pcm_strf(2, 48000, 16)),
            ],
        ),
    ]
    files["duplicate-strf.avi"] = build_avi(
        dup_strf_hdrl, movi_payload, chunk(b"idx1", idx1_entries)
    )
    cases.append(
        dict(
            file="duplicate-strf.avi",
            expected_error="duplicate AVI stream format",
        )
    )

    # Invalid 4: Non-16-byte-multiple idx1 (32 valid bytes + 6 trailing bytes)
    base_hdrl = [
        chunk(b"avih", avih(1)),
        list_chunk(
            b"strl",
            [
                chunk(b"strh", strh(b"auds", 1, 48000, 0, 8)),
                chunk(b"strf", pcm_strf(1, 48000, 16)),
            ],
        ),
    ]
    files["unaligned-idx1.avi"] = build_avi(
        base_hdrl, movi_payload, chunk(b"idx1", idx1_entries + b"00wb\x10\x00")
    )
    cases.append(
        dict(
            file="unaligned-idx1.avi",
            expected_error="AVI idx1 chunk has a partial entry",
        )
    )

    manifest = dict(
        issue="AVI duplicate avih/strh/strf rejection, idx1 partial-entry validation, and delayed-start PCM/G.711 seek landing",
        controls=[
            c
            | dict(
                size=len(files[c["file"]]),
                sha256=hashlib.sha256(files[c["file"]]).hexdigest(),
            )
            for c in controls
        ],
        cases=[
            c
            | dict(
                size=len(files[c["file"]]),
                sha256=hashlib.sha256(files[c["file"]]).hexdigest(),
            )
            for c in cases
        ],
    )
    files["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return files


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
