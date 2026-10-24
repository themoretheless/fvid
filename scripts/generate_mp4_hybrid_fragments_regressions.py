#!/usr/bin/env python3
"""Purely author VP9 MP4 fixtures probing ISO/IEC 14496-12:2015 8.8.4/8.8.8 non-empty init stbl + moof fragments."""
import hashlib, json, struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/mp4-hybrid-fragments"


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
    regular3 = mp4([packet] * 3, 8, "420", (16, 16), [1, 2, 3])
    regular2 = mp4([packet] * 2, 8, "420", (16, 16), [1, 2])
    regular1 = mp4([packet] * 1, 8, "420", (16, 16), [1])
    files = {"regular.mp4": regular3, "frame.vp9": packet}
    controls = []
    cases = []

    empty = {
        b"stts": word(0, 0),
        b"stsz": word(0, 0, 0),
        b"stsc": word(0, 0),
        b"stco": word(0, 0),
        b"stss": word(0, 0),
    }

    def rewrite_empty(data):
        out = b""
        for k, b in atoms(data):
            if k in [b"trak", b"mdia", b"minf", b"stbl"]:
                b = rewrite_empty(b)
            elif k in empty:
                b = empty[k]
            out += atom(k, b)
        return out

    def make_fragment(sequence, dts, count, use_tfdt=True):
        tfhd = atom(b"tfhd", word(0x020038, 1, 1, len(packet), 0))
        mfhd = atom(b"mfhd", word(0, sequence))
        stamp = atom(b"tfdt", word(0, dts)) if use_tfdt else b""

        def make_moof(offset):
            trun = atom(b"trun", word(0x000001, count, offset))
            return atom(b"moof", mfhd + atom(b"traf", tfhd + stamp + trun))

        probe = make_moof(0)
        return make_moof(len(probe) + 8) + atom(b"mdat", packet * count)

    orig3 = dict(atoms(regular3))
    init_empty = rewrite_empty(orig3[b"moov"])
    ftyp_iso5 = atom(b"ftyp", b"iso5" + word(0) + b"iso5vp09")

    # Control 1: empty-init-three-fragments.mp4 (0 in stbl + 3 in fragments)
    moov_empty = atom(
        b"moov", init_empty + atom(b"mvex", atom(b"trex", word(0, 1, 1, 1, len(packet), 0)))
    )
    files["empty-init-three-fragments.mp4"] = (
        ftyp_iso5 + moov_empty + b"".join(make_fragment(i + 1, i, 1) for i in range(3))
    )
    controls.append(
        dict(file="empty-init-three-fragments.mp4", mode="0-in-stbl-plus-3-fragments")
    )

    # Control 2: hybrid-one-in-stbl-two-in-fragments.mp4 (1 in stbl + 2 in fragments)
    # Start from regular1 (ftyp + moov + mdat), insert mvex into moov, update stco by +len(mvex),
    # and set mdhd/mvhd duration to 0 so fragmented total duration comes from all 3 samples,
    # or set mdhd duration to 3 (both work once track.duration = stated.unwrap_or(track.duration) or max!).
    def build_hybrid(base_mp4, init_count, frag_specs):
        parts = dict(atoms(base_mp4))
        mvex = atom(b"mvex", atom(b"trex", word(0, 1, 1, 1, len(packet), 0)))
        moov = atom(b"moov", parts[b"moov"] + mvex)
        prefix = atom(b"ftyp", parts[b"ftyp"]) + atom(b"mdat", parts[b"mdat"]) + moov
        assert len(prefix) == len(base_mp4) + len(mvex)
        frags = b"".join(
            make_fragment(seq, dts, cnt, use_tfdt) for seq, dts, cnt, use_tfdt in frag_specs
        )
        return prefix + frags

    files["hybrid-one-in-stbl-two-in-fragments.mp4"] = build_hybrid(
        regular1, 1, [(1, 1, 1, True), (2, 2, 1, True)]
    )
    controls.append(
        dict(
            file="hybrid-one-in-stbl-two-in-fragments.mp4",
            mode="1-in-stbl-plus-2-explicit-tfdt-fragments",
        )
    )

    # Control 3: hybrid-two-in-stbl-one-implicit-tfdt-fragment.mp4 (2 in stbl + 1 fragment without tfdt)
    # The fragment omits tfdt, so its dts continues directly from the 2nd stbl sample (dts=2)!
    files["hybrid-two-in-stbl-one-implicit-tfdt-fragment.mp4"] = build_hybrid(
        regular2, 2, [(1, 2, 1, False)]
    )
    controls.append(
        dict(
            file="hybrid-two-in-stbl-one-implicit-tfdt-fragment.mp4",
            mode="2-in-stbl-plus-1-implicit-tfdt-fragment",
        )
    )

    # Malformed Case 1: corrupt-stco-outside-mdat-in-fragmented.mp4
    # Init segment has 1 sample in stbl whose stco points outside mdat (to offset 8), followed by 2 valid fragments.
    # Pre-fix skipped sample_index() when fragmented=true, silently ignoring the corrupt stbl!
    bad_stco = bytearray(files["hybrid-one-in-stbl-two-in-fragments.mp4"])
    # Find stco atom in bad_stco and set its single chunk offset to 8 (inside ftyp, outside mdat)
    idx = bad_stco.index(b"stco")
    struct.pack_into(">I", bad_stco, idx + 4 + 8, 8)
    files["corrupt-stco-outside-mdat-in-fragmented.mp4"] = bytes(bad_stco)
    cases.append(
        dict(
            file="corrupt-stco-outside-mdat-in-fragmented.mp4",
            expected_error="sample chunk outside mdat",
        )
    )

    # Malformed Case 2: corrupt-stsz-in-fragmented.mp4
    # Init segment has a truncated/mismatched stsz table (count=5 with 0 entries when fixed_size=0), followed by 3 valid fragments.
    bad_stsz = bytearray(files["empty-init-three-fragments.mp4"])
    idx = bad_stsz.index(b"stsz")
    struct.pack_into(">I", bad_stsz, idx + 4 + 8, 5)
    files["corrupt-stsz-in-fragmented.mp4"] = bytes(bad_stsz)
    cases.append(
        dict(
            file="corrupt-stsz-in-fragmented.mp4",
            expected_error="invalid sample-size table",
        )
    )

    files["manifest.json"] = (
        json.dumps(
            dict(
                provenance="Pure Python authored 16x16 neutral VP9 MP4/fMP4, 3 independent 20ms frames at 50Hz. Probes ISO/IEC 14496-12:2015 8.8.4/8.8.8 hybrid non-empty moov/trak/mdia/minf/stbl sample tables followed by moof/traf/trun fragments, plus refusal of malformed stbl tables in fragmented files. No private codec parameters, encoder, FFmpeg, or network.",
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
