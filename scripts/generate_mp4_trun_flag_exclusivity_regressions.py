#!/usr/bin/env python3
"""Purely author VP9 fMP4 fixtures probing ISO/IEC 14496-12:2015 8.8.8 trun flag mutual exclusion."""
import hashlib, json, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/mp4-trun-flags"


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

    SYNC_FLAG = 0x0000_0000
    NON_SYNC_FLAG = 0x0001_0000

    def build_moov(trex_flags):
        return atom(
            b"moov",
            init + atom(b"mvex", atom(b"trex", word(0, 1, 1, 1, len(packet), trex_flags))),
        )

    def build_fragment(sequence, dts, tfhd_default_flags, trun_flags, first_flag, row_flags):
        tfhd_f = 0x020018 | (0x000020 if tfhd_default_flags is not None else 0)
        tfhd_body = word(tfhd_f, 1, 1, len(packet)) + (
            word(tfhd_default_flags) if tfhd_default_flags is not None else b""
        )
        tfhd = atom(b"tfhd", tfhd_body)
        prefix = atom(b"mfhd", word(0, sequence))
        stamp = atom(b"tfdt", word(0, dts))
        count = len(row_flags)

        def make_moof(offset):
            trun_body = word(trun_flags, count, offset)
            if trun_flags & 0x000004:
                assert first_flag is not None
                trun_body += word(first_flag)
            if trun_flags & 0x000400:
                for rf in row_flags:
                    trun_body += word(rf)
            return atom(b"moof", prefix + atom(b"traf", tfhd + stamp + atom(b"trun", trun_body)))

        probe = make_moof(0)
        return make_moof(len(probe) + 8) + atom(b"mdat", packet * count)

    # Control 1: default-flags.mp4 (trun flags = 0x000001, sync inherited from tfhd/trex)
    files["default-flags.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, SYNC_FLAG, 0x000001, None, [SYNC_FLAG] * 3)
    )
    controls.append(
        dict(
            file="default-flags.mp4",
            mode="default",
            trun_flags=0x000001,
            sync=[True, True, True],
        )
    )

    # Control 2: first-sample-only.mp4 (trun flags = 0x000005, first_sample_flags = SYNC, default = SYNC)
    files["first-sample-only.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, SYNC_FLAG, 0x000005, SYNC_FLAG, [SYNC_FLAG] * 3)
    )
    controls.append(
        dict(
            file="first-sample-only.mp4",
            mode="first-sample-only",
            trun_flags=0x000005,
            sync=[True, True, True],
        )
    )

    # Control 3: per-sample-only.mp4 (trun flags = 0x000401, sample_flags = SYNC for each of 3 samples)
    files["per-sample-only.mp4"] = (
        ftyp
        + build_moov(NON_SYNC_FLAG)
        + build_fragment(1, 0, NON_SYNC_FLAG, 0x000401, None, [SYNC_FLAG] * 3)
    )
    controls.append(
        dict(
            file="per-sample-only.mp4",
            mode="per-sample-only",
            trun_flags=0x000401,
            sync=[True, True, True],
        )
    )

    # Control 4: first-sample-lead-sync.mp4 (trun flags = 0x000005, first_sample_flags = SYNC, default = NON_SYNC)
    files["first-sample-lead-sync.mp4"] = (
        ftyp
        + build_moov(NON_SYNC_FLAG)
        + build_fragment(1, 0, NON_SYNC_FLAG, 0x000005, SYNC_FLAG, [SYNC_FLAG, NON_SYNC_FLAG, NON_SYNC_FLAG])
    )
    controls.append(
        dict(
            file="first-sample-lead-sync.mp4",
            mode="first-sample-lead-sync",
            trun_flags=0x000005,
            sync=[True, False, False],
        )
    )

    # Malformed Case 1: both-populated-first-sync-row-nonsync.mp4
    # first_sample_flags says SYNC (0), row[0] sample_flags says NON_SYNC (0x00010000)
    files["both-populated-first-sync-row-nonsync.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(
            1,
            0,
            SYNC_FLAG,
            0x000405,
            SYNC_FLAG,
            [NON_SYNC_FLAG, SYNC_FLAG, SYNC_FLAG],
        )
    )
    cases.append(
        dict(
            file="both-populated-first-sync-row-nonsync.mp4",
            trun_flags=0x000405,
            sample_count=3,
            contradiction="first_sample_flags=sync vs sample_flags[0]=non_sync",
        )
    )

    # Malformed Case 2: both-populated-first-nonsync-row-sync.mp4
    # first_sample_flags says NON_SYNC (0x00010000), row[0..3] sample_flags say SYNC (0)
    files["both-populated-first-nonsync-row-sync.mp4"] = (
        ftyp
        + build_moov(NON_SYNC_FLAG)
        + build_fragment(
            1,
            0,
            NON_SYNC_FLAG,
            0x000405,
            NON_SYNC_FLAG,
            [SYNC_FLAG, SYNC_FLAG, SYNC_FLAG],
        )
    )
    cases.append(
        dict(
            file="both-populated-first-nonsync-row-sync.mp4",
            trun_flags=0x000405,
            sample_count=3,
            contradiction="first_sample_flags=non_sync vs sample_flags[0]=sync",
        )
    )

    # Malformed Case 3: both-late-fragment.mp4
    # First 2 fragments valid, 3rd fragment sets both 0x000004 and 0x000400
    files["both-late-fragment.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, SYNC_FLAG, 0x000005, SYNC_FLAG, [SYNC_FLAG])
        + build_fragment(2, 1, SYNC_FLAG, 0x000401, None, [SYNC_FLAG])
        + build_fragment(3, 2, SYNC_FLAG, 0x000405, NON_SYNC_FLAG, [SYNC_FLAG])
    )
    cases.append(
        dict(
            file="both-late-fragment.mp4",
            trun_flags=0x000405,
            sample_count=3,
            contradiction="third fragment combines 0x000004 and 0x000400",
        )
    )

    # Malformed Case 4: both-empty-run.mp4
    # Empty run (sample_count=0) with 0x000405 before valid 3-sample fragment
    files["both-empty-run.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, SYNC_FLAG, 0x000405, SYNC_FLAG, [])
        + build_fragment(2, 0, SYNC_FLAG, 0x000001, None, [SYNC_FLAG] * 3)
    )
    cases.append(
        dict(
            file="both-empty-run.mp4",
            trun_flags=0x000405,
            sample_count=0,
            contradiction="empty trun sets both 0x000004 and 0x000400",
        )
    )

    files["manifest.json"] = (
        json.dumps(
            dict(
                provenance="Pure Python authored 16x16 neutral VP9 fMP4, 3 independent 20ms frames at 50Hz. Probes ISO/IEC 14496-12:2015 8.8.8 mutual exclusion between trun first-sample-flags-present (0x000004) and sample-flags-present (0x000400). No private codec parameters, encoder, FFmpeg, or network.",
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
