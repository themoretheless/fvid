#!/usr/bin/env python3
"""Purely author VP9 fMP4 fixtures probing ISO/IEC 14496-12:2015 8.8.4/8.8.6 single-mfhd-per-moof and single-trex-per-track contracts."""
import hashlib, json, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/mp4-mfhd-trex"


def atoms(data):
    at = 0
    while at < len(data):
        size, kind = struct.unpack_from(">I4s", data, at)
        assert size >= 8 and at + size <= len(data)
        yield kind, data[at + 8 : at + size]
        at += size
    assert at == len(data)


def generate():
    packet = frame(8, "420", 16, 16)
    regular = mp4([packet] * 3, 8, "420", (16, 16), [1, 2, 3])
    original = dict(atoms(regular))
    files = {"regular.mp4": regular, "frame.vp9": packet}
    controls = []
    cases = []

    empty = {
        b"stts": word(0, 0),
        b"stsz": word(0, 0, 0),
        b"stsc": word(0, 0),
        b"stco": word(0, 0),
        b"stss": word(0, 0),
    }

    def rewrite(data):
        out = b""
        for k, b in atoms(data):
            if k in [b"trak", b"mdia", b"minf", b"stbl"]:
                b = rewrite(b)
            elif k in empty:
                b = empty[k]
            out += atom(k, b)
        return out

    init = rewrite(original[b"moov"])
    ftyp = atom(b"ftyp", b"iso5" + word(0) + b"iso5vp09")
    tfhd = atom(b"tfhd", word(0x020038, 1, 1, len(packet), 0))

    def make_fragment(mfhd_atoms, sequence, dts, count):
        stamp = atom(b"tfdt", word(0, dts))

        def make_moof(offset):
            trun = atom(b"trun", word(0x000001, count, offset))
            return atom(b"moof", mfhd_atoms + atom(b"traf", tfhd + stamp + trun))

        probe = make_moof(0)
        return make_moof(len(probe) + 8) + atom(b"mdat", packet * count)

    # Control 1: valid-single-fragment.mp4
    moov_single = atom(
        b"moov", init + atom(b"mvex", atom(b"trex", word(0, 1, 1, 1, len(packet), 0)))
    )
    files["valid-single-fragment.mp4"] = (
        ftyp + moov_single + make_fragment(atom(b"mfhd", word(0, 1)), 1, 0, 3)
    )
    controls.append(dict(file="valid-single-fragment.mp4", mode="single-trex-single-mfhd"))

    # Control 2: valid-three-fragments.mp4
    files["valid-three-fragments.mp4"] = (
        ftyp
        + moov_single
        + b"".join(make_fragment(atom(b"mfhd", word(0, i + 1)), i + 1, i, 1) for i in range(3))
    )
    controls.append(dict(file="valid-three-fragments.mp4", mode="three-sequential-mfhd-fragments"))

    # Control 3: valid-multi-track-trex.mp4 (trex for track 1 and trex for unreferenced track 2)
    moov_two_distinct_trex = atom(
        b"moov",
        init
        + atom(
            b"mvex",
            atom(b"trex", word(0, 1, 1, 1, len(packet), 0))
            + atom(b"trex", word(0, 2, 1, 1, len(packet), 0)),
        ),
    )
    files["valid-multi-track-trex.mp4"] = (
        ftyp + moov_two_distinct_trex + make_fragment(atom(b"mfhd", word(0, 1)), 1, 0, 3)
    )
    controls.append(dict(file="valid-multi-track-trex.mp4", mode="distinct-track-trex-entries"))

    # Malformed Case 1: missing-mfhd.mp4 (moof contains traf without mfhd)
    files["missing-mfhd.mp4"] = ftyp + moov_single + make_fragment(b"", 1, 0, 3)
    cases.append(
        dict(
            file="missing-mfhd.mp4",
            expected_error="missing MP4 mfhd box",
        )
    )

    # Malformed Case 2: duplicate-mfhd.mp4 (moof contains two mfhd boxes)
    files["duplicate-mfhd.mp4"] = (
        ftyp
        + moov_single
        + make_fragment(atom(b"mfhd", word(0, 1)) + atom(b"mfhd", word(0, 2)), 1, 0, 3)
    )
    cases.append(
        dict(
            file="duplicate-mfhd.mp4",
            expected_error="duplicate MP4 singleton box",
        )
    )

    # Malformed Case 3: mfhd-zero-sequence.mp4 (mfhd sequence_number == 0)
    files["mfhd-zero-sequence.mp4"] = (
        ftyp + moov_single + make_fragment(atom(b"mfhd", word(0, 0)), 0, 0, 3)
    )
    cases.append(
        dict(
            file="mfhd-zero-sequence.mp4",
            expected_error="zero MP4 movie-fragment sequence number",
        )
    )

    # Malformed Case 4: mfhd-trailing-bytes.mp4 (mfhd is 12 bytes instead of 8)
    files["mfhd-trailing-bytes.mp4"] = (
        ftyp + moov_single + make_fragment(atom(b"mfhd", word(0, 1, 99)), 1, 0, 3)
    )
    cases.append(
        dict(
            file="mfhd-trailing-bytes.mp4",
            expected_error="MP4 movie-fragment header size does not match its fields",
        )
    )

    # Malformed Case 5: trex-zero-track-id.mp4 (trex track_ID == 0)
    moov_zero_trex = atom(
        b"moov",
        init
        + atom(
            b"mvex",
            atom(b"trex", word(0, 0, 1, 1, len(packet), 0))
            + atom(b"trex", word(0, 1, 1, 1, len(packet), 0)),
        ),
    )
    files["trex-zero-track-id.mp4"] = (
        ftyp + moov_zero_trex + make_fragment(atom(b"mfhd", word(0, 1)), 1, 0, 3)
    )
    cases.append(
        dict(
            file="trex-zero-track-id.mp4",
            expected_error="zero MP4 track-extends track ID",
        )
    )

    # Malformed Case 6: duplicate-trex-track-id.mp4 (two trex boxes for track_ID == 1)
    moov_dup_trex = atom(
        b"moov",
        init
        + atom(
            b"mvex",
            atom(b"trex", word(0, 1, 1, 1, len(packet), 0))
            + atom(b"trex", word(0, 1, 1, 2, len(packet), 0x00010000)),
        ),
    )
    files["duplicate-trex-track-id.mp4"] = (
        ftyp + moov_dup_trex + make_fragment(atom(b"mfhd", word(0, 1)), 1, 0, 3)
    )
    cases.append(
        dict(
            file="duplicate-trex-track-id.mp4",
            expected_error="duplicate MP4 track-extends track ID",
        )
    )

    files["manifest.json"] = (
        json.dumps(
            dict(
                provenance="Pure Python authored 16x16 neutral VP9 fMP4, 3 independent 20ms frames at 50Hz. Probes ISO/IEC 14496-12:2015 8.8.3 (trex track_ID > 0 and uniqueness per track) and 8.8.4/8.8.5 (mandatory singleton mfhd in moof with 8-byte payload and non-zero sequence_number). No private codec parameters, encoder, FFmpeg, or network.",
                controls=controls,
                cases=cases,
                sha256={n: hashlib.sha256(b).hexdigest() for n, b in files.items()},
            ),
            indent=2,
        )
        + "\n"
    ).encode()
    return files


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    for n, b in generate().items():
        (OUT / n).write_bytes(b)
