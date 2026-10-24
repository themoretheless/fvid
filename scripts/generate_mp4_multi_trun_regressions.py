#!/usr/bin/env python3
"""Purely author VP9 fMP4 fixtures probing ISO/IEC 14496-12:2015 8.8.8 multi-trun data-offset Reset vs Implicit Continuity."""
import hashlib, json, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/mp4-multi-trun"


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
    moov = atom(b"moov", init + atom(b"mvex", atom(b"trex", word(0, 1, 1, 1, len(packet), 0))))

    tfhd = atom(b"tfhd", word(0x020038, 1, 1, len(packet), 0))
    prefix = atom(b"mfhd", word(0, 1))
    stamp = atom(b"tfdt", word(0, 0))

    # Control 1: three-contiguous-runs.mp4
    # First trun has data_offset (1 sample), second and third truns omit data_offset (1 sample each)
    # and follow immediately in the same mdat.
    def make_contiguous(offset):
        trun1 = atom(b"trun", word(0x000001, 1, offset))
        trun2 = atom(b"trun", word(0x000000, 1))
        trun3 = atom(b"trun", word(0x000000, 1))
        return atom(b"moof", prefix + atom(b"traf", tfhd + stamp + trun1 + trun2 + trun3))

    probe = make_contiguous(0)
    files["three-contiguous-runs.mp4"] = (
        ftyp + moov + make_contiguous(len(probe) + 8) + atom(b"mdat", packet * 3)
    )
    controls.append(dict(file="three-contiguous-runs.mp4", mode="three-contiguous-runs-in-one-traf"))

    # Control 2: empty-leading-run-then-contiguous.mp4
    # First trun has data_offset and 0 samples; second trun omits data_offset and has 3 samples.
    def make_empty_leading(offset):
        trun1 = atom(b"trun", word(0x000001, 0, offset))
        trun2 = atom(b"trun", word(0x000000, 3))
        return atom(b"moof", prefix + atom(b"traf", tfhd + stamp + trun1 + trun2))

    probe = make_empty_leading(0)
    files["empty-leading-run-then-contiguous.mp4"] = (
        ftyp + moov + make_empty_leading(len(probe) + 8) + atom(b"mdat", packet * 3)
    )
    controls.append(
        dict(
            file="empty-leading-run-then-contiguous.mp4",
            mode="empty-leading-run-sets-cursor-for-second-run",
        )
    )

    # Control 3: two-explicit-offset-runs.mp4
    # Two truns in one traf, each with explicit data_offset pointing into two separate mdats.
    mdat1 = atom(b"mdat", packet)
    mdat2 = atom(b"mdat", packet * 2)

    def make_two_explicit(off1, off2):
        trun1 = atom(b"trun", word(0x000001, 1, off1))
        trun2 = atom(b"trun", word(0x000001, 2, off2))
        return atom(b"moof", prefix + atom(b"traf", tfhd + stamp + trun1 + trun2))

    probe = make_two_explicit(0, 0)
    moof_len = len(probe)
    files["two-explicit-offset-runs.mp4"] = (
        ftyp
        + moov
        + make_two_explicit(moof_len + 8, moof_len + len(mdat1) + 8)
        + mdat1
        + mdat2
    )
    controls.append(
        dict(file="two-explicit-offset-runs.mp4", mode="two-explicit-offset-runs-across-two-mdats")
    )

    # Malformed Case 1: second-traf-missing-data-offset.mp4
    # Fragment 1 has 1 sample at mdat1, followed by a free box (16 bytes), followed by Fragment 2
    # whose trun omits data_offset (0x000000) while tfhd uses default-base-is-moof (0x020038).
    # In Fragment 2, base_data_offset is moof2.start (outside any mdat), whereas pre-fix
    # falsely continues from Fragment 1's list.last() if mdat1 had trailing room!
    # Put all 3 packets in mdat1 after Fragment 1 so pre-fix silently indexes Fragment 2's 2 samples
    # from mdat1 (at Fragment 1's end) even though Fragment 2 started a new traf whose base is moof2!
    def make_frag1(offset):
        trun = atom(b"trun", word(0x000001, 1, offset))
        return atom(b"moof", atom(b"mfhd", word(0, 1)) + atom(b"traf", tfhd + atom(b"tfdt", word(0, 0)) + trun))

    probe1 = make_frag1(0)
    frag1 = make_frag1(len(probe1) + 8) + atom(b"mdat", packet * 3)
    frag2_Without_offset = atom(
        b"moof",
        atom(b"mfhd", word(0, 2))
        + atom(b"traf", tfhd + atom(b"tfdt", word(0, 1)) + atom(b"trun", word(0x000000, 2))),
    )
    files["second-traf-missing-data-offset.mp4"] = ftyp + moov + frag1 + frag2_Without_offset
    cases.append(
        dict(
            file="second-traf-missing-data-offset.mp4",
            expected_error="sample chunk outside mdat",
            description="Second traf omits trun.data_offset with default-base-is-moof; base is moof2 (outside mdat1), not previous traf's last sample",
        )
    )

    # Malformed Case 2: empty-reset-run-outside-mdat.mp4
    # In one traf: trun 1 indexes 1 sample in mdat (which holds 3 packets),
    # trun 2 is empty (0 samples) with explicit data_offset = 0 (pointing to moof start, outside mdat),
    # trun 3 has 2 samples and omits data_offset (so it must start at trun 2's cursor = moof start, outside mdat,
    # NOT at trun 1's last sample!).
    def make_empty_reset(off1):
        trun1 = atom(b"trun", word(0x000001, 1, off1))
        trun2 = atom(b"trun", word(0x000001, 0, 0))
        trun3 = atom(b"trun", word(0x000000, 2))
        return atom(b"moof", prefix + atom(b"traf", tfhd + stamp + trun1 + trun2 + trun3))

    probe = make_empty_reset(0)
    files["empty-reset-run-outside-mdat.mp4"] = (
        ftyp + moov + make_empty_reset(len(probe) + 8) + atom(b"mdat", packet * 3)
    )
    cases.append(
        dict(
            file="empty-reset-run-outside-mdat.mp4",
            expected_error="sample chunk outside mdat",
            description="Empty second trun resets traf data cursor to moof start (outside mdat); third trun without data_offset must inherit that cursor instead of trun 1's last sample",
        )
    )

    files["manifest.json"] = (
        json.dumps(
            dict(
                provenance="Pure Python authored 16x16 neutral VP9 fMP4, 3 independent 20ms frames at 50Hz. Probes ISO/IEC 14496-12:2015 8.8.8 per-traf data cursor continuity across multiple trun boxes and reset at each new traf. No private codec parameters, encoder, FFmpeg, or network.",
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
