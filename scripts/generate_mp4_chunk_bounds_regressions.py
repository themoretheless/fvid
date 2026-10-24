#!/usr/bin/env python3
"""Purely author MP4/MOV chunk-offset ordering, non-overlapping chunk, and sample-size regression fixtures."""
import hashlib, json, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/mp4-chunk-bounds"


def atoms(data):
    at = 0
    while at < len(data):
        size, kind = struct.unpack_from(">I4s", data, at)
        assert size >= 8 and at + size <= len(data)
        yield kind, data[at + 8 : at + size]
        at += size
    assert at == len(data)


def rewrite_stbl(data, repl):
    out = b""
    for k, b in atoms(data):
        if k in [b"trak", b"mdia", b"minf"]:
            b = rewrite_stbl(b, repl)
        elif k == b"stbl":
            stbl_out = b""
            for sk, sb in atoms(b):
                if sk in repl:
                    new_kind, new_body = repl[sk]
                    if new_kind is not None:
                        stbl_out += atom(new_kind, new_body)
                else:
                    stbl_out += atom(sk, sb)
            b = stbl_out
        out += atom(k, b)
    return out


def generate():
    packet = frame(8, "420", 16, 16)
    plen = len(packet)
    regular3 = mp4([packet] * 3, 8, "420", (16, 16), [1, 2, 3])
    parts = dict(atoms(regular3))
    ftyp = atom(b"ftyp", parts[b"ftyp"])
    mdat = atom(b"mdat", parts[b"mdat"])
    base_offset = len(ftyp) + 8
    moov = parts[b"moov"]

    files = {"frame.vp9": packet}
    controls = []
    cases = []

    # Control 1: multi-chunk stco (3 chunks of 1 sample each, contiguous)
    multi_stco_moov = rewrite_stbl(
        moov,
        {
            b"stsc": (b"stsc", word(0, 1, 1, 1, 1)),
            b"stco": (
                b"stco",
                word(0, 3, base_offset, base_offset + plen, base_offset + 2 * plen),
            ),
        },
    )
    files["valid-multi-chunk-stco.mp4"] = ftyp + mdat + atom(b"moov", multi_stco_moov)
    controls.append(
        dict(file="valid-multi-chunk-stco.mp4", mode="three-contiguous-stco-chunks")
    )

    # Control 2: multi-chunk co64 (2 chunks: 2 samples + 1 sample, 64-bit offsets)
    co64_payload = (
        word(0, 2)
        + struct.pack(">QQ", base_offset, base_offset + 2 * plen)
    )
    multi_co64_moov = rewrite_stbl(
        moov,
        {
            b"stsc": (b"stsc", word(0, 2, 1, 2, 1, 2, 1, 1)),
            b"stco": (b"co64", co64_payload),
        },
    )
    files["valid-multi-chunk-co64.mp4"] = ftyp + mdat + atom(b"moov", multi_co64_moov)
    controls.append(
        dict(file="valid-multi-chunk-co64.mp4", mode="two-contiguous-co64-chunks")
    )

    # Invalid 1: overlapping stco chunks (chunk 2 starts at base_offset + plen - 1)
    overlap_stco_moov = rewrite_stbl(
        moov,
        {
            b"stsc": (b"stsc", word(0, 2, 1, 1, 1, 2, 2, 1)),
            b"stco": (b"stco", word(0, 2, base_offset, base_offset + plen - 1)),
        },
    )
    files["overlapping-stco.mp4"] = ftyp + mdat + atom(b"moov", overlap_stco_moov)
    cases.append(
        dict(
            file="overlapping-stco.mp4",
            expected_error="overlapping or non-increasing MP4 chunk offsets",
        )
    )

    # Invalid 2: backward co64 chunks (chunk 2 starts at base_offset, before chunk 1 at base_offset + plen)
    backward_co64_moov = rewrite_stbl(
        moov,
        {
            b"stsc": (b"stsc", word(0, 2, 1, 1, 1, 2, 2, 1)),
            b"stco": (
                b"co64",
                word(0, 2) + struct.pack(">QQ", base_offset + plen, base_offset),
            ),
        },
    )
    files["backward-co64.mp4"] = ftyp + mdat + atom(b"moov", backward_co64_moov)
    cases.append(
        dict(
            file="backward-co64.mp4",
            expected_error="overlapping or non-increasing MP4 chunk offsets",
        )
    )

    # Invalid 3: zero-size stsz entry (sample 1 has size 0)
    zero_stsz_moov = rewrite_stbl(
        moov,
        {
            b"stsz": (b"stsz", word(0, 0, 3, plen, 0, plen)),
        },
    )
    files["zero-sample-size-stsz.mp4"] = ftyp + mdat + atom(b"moov", zero_stsz_moov)
    cases.append(
        dict(
            file="zero-sample-size-stsz.mp4",
            expected_error="zero MP4 sample size",
        )
    )

    # Invalid 4: zero-size trun sample in fragmented MP4
    empty_tables = {
        b"stts": (b"stts", word(0, 0)),
        b"stsz": (b"stsz", word(0, 0, 0)),
        b"stsc": (b"stsc", word(0, 0)),
        b"stco": (b"stco", word(0, 0)),
        b"stss": (b"stss", word(0, 0)),
    }
    init_empty = rewrite_stbl(moov, empty_tables)
    moov_frag = atom(
        b"moov", init_empty + atom(b"mvex", atom(b"trex", word(0, 1, 1, 1, plen, 0)))
    )
    ftyp_iso5 = atom(b"ftyp", b"iso5" + word(0) + b"iso5vp09")
    tfhd = atom(b"tfhd", word(0x020000, 1))
    mfhd = atom(b"mfhd", word(0, 1))
    tfdt = atom(b"tfdt", word(0, 0))
    # trun with flags 0x000201 (data-offset + sample-size-present), 2 samples: plen and 0
    probe_trun = atom(b"trun", word(0x000201, 2, 0, plen, 0))
    probe_moof = atom(b"moof", mfhd + atom(b"traf", tfhd + tfdt + probe_trun))
    real_trun = atom(b"trun", word(0x000201, 2, len(probe_moof) + 8, plen, 0))
    real_moof = atom(b"moof", mfhd + atom(b"traf", tfhd + tfdt + real_trun))
    files["zero-sample-size-trun.mp4"] = (
        ftyp_iso5 + moov_frag + real_moof + atom(b"mdat", packet * 2)
    )
    cases.append(
        dict(
            file="zero-sample-size-trun.mp4",
            expected_error="zero MP4 fragment sample size",
        )
    )

    # Invalid 5: empty non-fragmented track (stsz count == 0)
    empty_nonfrag_moov = rewrite_stbl(moov, empty_tables)
    files["empty-nonfragmented-track.mp4"] = (
        ftyp + mdat + atom(b"moov", empty_nonfrag_moov)
    )
    cases.append(
        dict(
            file="empty-nonfragmented-track.mp4",
            expected_error="empty MP4 sample table",
        )
    )

    # Invalid 6: fragmented MP4 with empty trun (count == 0) and 0 total samples
    empty_trun_probe = atom(b"trun", word(0x000001, 0, 0))
    empty_moof_probe = atom(b"moof", mfhd + atom(b"traf", tfhd + tfdt + empty_trun_probe))
    empty_trun_real = atom(b"trun", word(0x000001, 0, len(empty_moof_probe) + 8))
    empty_moof_real = atom(b"moof", mfhd + atom(b"traf", tfhd + tfdt + empty_trun_real))
    files["empty-fragmented-track.mp4"] = (
        ftyp_iso5 + moov_frag + empty_moof_real + atom(b"mdat", packet)
    )
    cases.append(
        dict(
            file="empty-fragmented-track.mp4",
            expected_error="MP4 track states no samples",
        )
    )

    manifest = {
        "generator": "scripts/generate_mp4_chunk_bounds_regressions.py",
        "controls": controls,
        "cases": cases,
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
