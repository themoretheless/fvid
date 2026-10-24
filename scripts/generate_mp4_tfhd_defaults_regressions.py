#!/usr/bin/env python3
"""Purely author VP9 fMP4 fixtures probing ISO/IEC 14496-12:2015 8.8.7/8.8.8/8.8.12 box-size and default-flag contracts."""
import hashlib, json, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/mp4-tfhd-defaults"


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

    def build_moov(trex_flags, trex_extra=b""):
        trex_body = word(0, 1, 1, 1, len(packet), trex_flags) + trex_extra
        return atom(b"moov", init + atom(b"mvex", atom(b"trex", trex_body)))

    def build_fragment(
        sequence,
        dts,
        tfhd_default_flags,
        trun_flags=0x000001,
        first_flag=None,
        row_flags=None,
        count=3,
        tfhd_extra=b"",
        tfdt_version=0,
        tfdt_extra=b"",
    ):
        tfhd_f = 0x020018 | (0x000020 if tfhd_default_flags is not None else 0)
        tfhd_body = (
            word(tfhd_f, 1, 1, len(packet))
            + (word(tfhd_default_flags) if tfhd_default_flags is not None else b"")
            + tfhd_extra
        )
        tfhd = atom(b"tfhd", tfhd_body)
        prefix = atom(b"mfhd", word(0, sequence))
        if tfdt_version == 0:
            tfdt_body = word(0, dts) + tfdt_extra
        else:
            tfdt_body = word(0x01000000) + struct.pack(">Q", dts) + tfdt_extra
        stamp = atom(b"tfdt", tfdt_body)

        def make_moof(offset):
            trun_body = word(trun_flags, count, offset)
            if trun_flags & 0x000004:
                assert first_flag is not None
                trun_body += word(first_flag)
            if trun_flags & 0x000400:
                assert row_flags is not None and len(row_flags) == count
                for rf in row_flags:
                    trun_body += word(rf)
            return atom(b"moof", prefix + atom(b"traf", tfhd + stamp + atom(b"trun", trun_body)))

        probe = make_moof(0)
        return make_moof(len(probe) + 8) + atom(b"mdat", packet * count)

    # Control 1: tfhd-overrides-trex-nonsync.mp4
    # trex default_sample_flags = NON_SYNC (0x00010000), tfhd default_sample_flags = SYNC (0x00000000)
    # Without the fix, trex's Some(0x00010000) wins over tfhd's 0 when ORed or if trex=0 vs tfhd!
    # Wait: in Control 1, tfhd has Some(0) and trex has Some(0x00010000): Some(0).or_else(...) is Some(0) (sync=true).
    # In Control 2: trex has SYNC (0) and tfhd has NON_SYNC (0x00010000), with first_sample_flags = SYNC (0):
    # sample 0 is sync (from first_sample_flags), samples 1..2 are non-sync (from tfhd override).
    files["tfhd-overrides-trex-nonsync.mp4"] = (
        ftyp
        + build_moov(NON_SYNC_FLAG)
        + build_fragment(1, 0, SYNC_FLAG, 0x000001, None, None, 3)
    )
    controls.append(
        dict(
            file="tfhd-overrides-trex-nonsync.mp4",
            mode="tfhd-sync-overrides-trex-nonsync",
            sync=[True, True, True],
        )
    )

    files["tfhd-nonsync-overrides-trex-sync-with-lead.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, NON_SYNC_FLAG, 0x000005, SYNC_FLAG, None, 3)
    )
    controls.append(
        dict(
            file="tfhd-nonsync-overrides-trex-sync-with-lead.mp4",
            mode="tfhd-nonsync-overrides-trex-sync-with-lead",
            sync=[True, False, False],
        )
    )

    files["tfdt-v1-exact.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, None, 0x000001, None, None, 3, tfdt_version=1)
    )
    controls.append(
        dict(
            file="tfdt-v1-exact.mp4",
            mode="tfdt-v1-exact-with-trex-zero-flags",
            sync=[True, True, True],
        )
    )

    # Case 1 (Valid stream that pre-fix mis-indexes!):
    # trex default_sample_flags = 0x00010000 (non-sync) -> wait: what if trex has 0 (sync) and tfhd is omitted,
    # Wait: if trex has 0 (sync) and tfhd is omitted, header_flags is None, unwrap_or(0) is 0 (sync).
    # When does `flags: (flags != 0).then_some(flags)` cause a bug?
    # Look at `src/container/mp4.rs:1585`:
    # `trex.flags = (flags != 0).then_some(flags)`.
    # What if `tfhd` sets `0x02_0000` (`default-base-is-moof`) AND `0x00_0000` (no `0x20`), so `tfhd` omits `default_sample_flags`,
    # that's covered. What if `trex` sets `default_sample_flags = 0`, why did `mp4.rs` write `(flags != 0).then_some(flags)`?
    # Wait: in `mp4.rs:1654`:
    # `let mut header_flags = None; if flags & 0x00_0020 != 0 { header_flags = Some(u32be(header, at)?); }` (note `at += 4` was missing!).
    # Because `at += 4` was missing and `at == header.len()` was never checked:
    # Malformed Case 1: `tfhd-trailing-bytes.mp4` (tfhd has 4 undeclared trailing bytes)
    files["tfhd-trailing-bytes.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, SYNC_FLAG, 0x000001, None, None, 3, tfhd_extra=word(0xDEAD_BEEF))
    )
    cases.append(
        dict(
            file="tfhd-trailing-bytes.mp4",
            expected_error="MP4 track-fragment header size does not match its flags",
        )
    )

    # Malformed Case 2: `tfhd-no-flags-trailing-bytes.mp4` (tfhd without 0x20 has 4 undeclared trailing bytes)
    files["tfhd-no-flags-trailing-bytes.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, None, 0x000001, None, None, 3, tfhd_extra=word(0x0001_0000))
    )
    cases.append(
        dict(
            file="tfhd-no-flags-trailing-bytes.mp4",
            expected_error="MP4 track-fragment header size does not match its flags",
        )
    )

    # Malformed Case 3: `trex-trailing-bytes.mp4` (trex is 28 bytes instead of 24)
    files["trex-trailing-bytes.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG, trex_extra=word(0xDEAD_BEEF))
        + build_fragment(1, 0, SYNC_FLAG, 0x000001, None, None, 3)
    )
    cases.append(
        dict(
            file="trex-trailing-bytes.mp4",
            expected_error="MP4 track-extends box size does not match its fields",
        )
    )

    # Malformed Case 4: `tfdt-v0-trailing-bytes.mp4` (tfdt v0 is 12 bytes instead of 8, e.g. v0 header with 64-bit time)
    files["tfdt-v0-trailing-bytes.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, SYNC_FLAG, 0x000001, None, None, 3, tfdt_version=0, tfdt_extra=word(99))
    )
    cases.append(
        dict(
            file="tfdt-v0-trailing-bytes.mp4",
            expected_error="MP4 track-fragment decode time size does not match its version",
        )
    )

    # Malformed Case 5: `tfdt-v1-trailing-bytes.mp4` (tfdt v1 is 16 bytes instead of 12)
    files["tfdt-v1-trailing-bytes.mp4"] = (
        ftyp
        + build_moov(SYNC_FLAG)
        + build_fragment(1, 0, SYNC_FLAG, 0x000001, None, None, 3, tfdt_version=1, tfdt_extra=word(99))
    )
    cases.append(
        dict(
            file="tfdt-v1-trailing-bytes.mp4",
            expected_error="MP4 track-fragment decode time size does not match its version",
        )
    )

    files["manifest.json"] = (
        json.dumps(
            dict(
                provenance="Pure Python authored 16x16 neutral VP9 fMP4, 3 independent 20ms frames at 50Hz. Probes ISO/IEC 14496-12:2015 8.8.3 (trex), 8.8.7 (tfhd), and 8.8.12 (tfdt) exact box-size validation and default flag resolution. No private codec parameters, encoder, FFmpeg, or network.",
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
