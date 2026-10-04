#!/usr/bin/env python3
"""Generate tiny synthetic reproductions; never read private/source videos."""
from pathlib import Path
import struct
import argparse
import hashlib
import json
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors"


def atom(kind, payload):
    return struct.pack(">I4s", len(payload) + 8, kind) + payload


def children(data):
    at = 0
    while at < len(data):
        size, kind = struct.unpack_from(">I4s", data, at)
        assert 8 <= size <= len(data) - at
        yield kind, data[at + 8:at + size]
        at += size


def rewrite(data, edits=None, bad_avcc=False, duplicate_pts=False, video_metadata=False):
    result = bytearray()
    for kind, payload in children(data):
        if kind in {b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts"}:
            payload = rewrite(payload, edits, bad_avcc, duplicate_pts, video_metadata)
            if kind == b"stbl" and duplicate_pts:
                assert all(k != b"ctts" for k, _ in children(payload))
                # Decode time stays strictly increasing; frame 6 repeats PTS 5.
                if duplicate_pts == "delayed":
                    runs = [(12, 1000)]
                elif duplicate_pts == "run":
                    runs = [(4, 0), (1, -1000), (1, -2000), (1, -3000), (1, -4000), (4, 0)]
                elif duplicate_pts == "tail":
                    runs = [(11, 0), (1, -1000)]
                elif duplicate_pts == "all":
                    runs = [(1, -i * 1000) for i in range(12)]
                else:
                    runs = [(6, 0), (1, -1000), (5, 0)]
                ctts = bytes([1, 0, 0, 0]) + struct.pack(">I", len(runs))
                ctts += b"".join(struct.pack(">Ii", n, offset) for n, offset in runs)
                payload += atom(b"ctts", ctts)
        elif kind == b"stsd":
            entries = bytearray(payload[:8])
            for codec, entry in children(payload[8:]):
                assert codec in {b"avc1", b"hvc1", b"hev1"}
                extra = rewrite(entry[78:], edits, bad_avcc, duplicate_pts, video_metadata)
                if video_metadata:
                    extra = b"".join(atom(k, p) for k, p in children(extra) if k not in {b"colr", b"pasp"})
                    extra += atom(b"colr", b"nclx" + struct.pack(">HHHB", 1, 1, 1, 0))
                    extra += atom(b"pasp", struct.pack(">II", 3, 2))
                entries += atom(codec, entry[:78] + extra)
            payload = bytes(entries)
        elif kind == b"elst" and edits is not None:
            payload = bytes(4) + struct.pack(">I", len(edits))
            payload += b"".join(struct.pack(">IiI", duration, start, 0x10000) for duration, start in edits)
        elif kind == b"mvhd" and edits is not None:
            assert payload[0] == 0
            payload = payload[:16] + struct.pack(">I", sum(d for d, _ in edits)) + payload[20:]
        elif kind == b"tkhd" and edits is not None:
            assert payload[0] == 0
            payload = payload[:20] + struct.pack(">I", sum(d for d, _ in edits)) + payload[24:]
        elif kind == b"avcC" and bad_avcc:
            assert payload[1] == 100
            at = 6
            for _ in range(payload[5] & 31):
                size = int.from_bytes(payload[at:at + 2], "big")
                at += 2 + size
            count = payload[at]
            at += 1
            for _ in range(count):
                size = int.from_bytes(payload[at:at + 2], "big")
                at += 2 + size
            # Reproduce the malformed *metadata*, not any private SPS/PPS/media.
            payload = payload[:at] + bytes.fromhex("7bf7f700")
        if kind == b"moov" and video_metadata:
            payload = b"".join(atom(k, p) for k, p in children(payload) if k != b"udta")
            title = atom(b"\xa9nam", b"FVid synthetic CUDA metadata")
            chapter = bytes(8) + bytes([1]) + struct.pack(">Q", 0) + bytes([5]) + b"Start"
            payload += atom(b"udta", title + atom(b"chpl", chapter))
        if kind == b"tkhd" and video_metadata:
            assert payload[0] == 0
            matrix = struct.pack(">9i", 0, 65536, 0, -65536, 0, 0, 0, 0, 1 << 30)
            payload = payload[:40] + matrix + payload[76:]
        result += atom(kind, payload)
    return bytes(result)


def load_seeds():
    """Validate the complete immutable synthetic corpus before writing output."""
    base = ROOT / "tests/fixtures/playback-errors"
    manifest = json.loads((base / "synthetic-playback-seeds.json").read_text())
    with zipfile.ZipFile(base / "synthetic-playback-seeds.zip") as archive:
        if set(archive.namelist()) != set(manifest):
            raise ValueError("synthetic seed inventory mismatch")
        seeds = {name: archive.read(name) for name in manifest}
    for name, payload in seeds.items():
        if Path(name).name != name or hashlib.sha256(payload).hexdigest() != manifest[name]:
            raise ValueError("synthetic seed integrity failure: " + name)
    return seeds


def ffv1_level_one_source(seed):
    # Keep offsets and all synthetic packets intact; distinguish this fixture
    # by the harmless ftyp minor version. The trigger is the encoder request.
    data = bytearray(seed)
    assert data[4:8] == b"ftyp"
    data[12:16] = struct.pack(">I", 1)
    return bytes(data)


def main():
    seeds = load_seeds()
    OUT.mkdir(parents=True, exist_ok=True)
    # Public synthetic HEVC seed only; never import user videos or parameters.
    # Movie clock 30 Hz, media clock 15360 Hz. Six-frame range is replayed,
    # with leading/interior 0.1-second black spans.
    for seed, name in [("main-ipb.mp4", "hevc-cuda-edit-repeat.mp4"),
                       ("main10-ipb.mp4", "hevc-main10-cuda-edit-repeat.mp4")]:
        hevc = (ROOT / "tests/fixtures/hevc" / seed).read_bytes()
        order = [kind for kind, _ in children(hevc)]
        assert order.index(b"mdat") < order.index(b"moov")
        (OUT / name).write_bytes(rewrite(hevc,
            edits=[(3, -1), (6, 1024), (3, -1), (6, 1024)]))
    control = OUT / "control.mp4"
    control.write_bytes(seeds["control.mp4"])
    (OUT / "ffv1-level-one-source.mp4").write_bytes(ffv1_level_one_source(seeds["control.mp4"]))
    source = control.read_bytes()
    order = [kind for kind, _ in children(source)]
    # moov changes size; mdat must precede it so chunk offsets stay valid.
    assert order.index(b"mdat") < order.index(b"moov")
    cases = {
        "avc-cuda-video-metadata.mp4": dict(video_metadata=True),
        "edit-empty-spans.mov": dict(edits=[(1000, -1), (2000, 0), (1000, -1), (2000, 0)]),
        "edit-gap.mov": dict(edits=[(2000, 0), (8000, 4000)]),
        "edit-three-ranges.mov": dict(edits=[(2000, 0), (2000, 4000), (4000, 8000)]),
        "edit-repeat.mov": dict(edits=[(1000, 0), (12000, 0)]),
        "avcc-invalid-reserved.mp4": dict(bad_avcc=True),
        "duplicate-pts.mp4": dict(duplicate_pts=True),
        "duplicate-pts-run.mp4": dict(duplicate_pts="run"),
        "duplicate-pts-tail.mp4": dict(duplicate_pts="tail"),
        "duplicate-pts-all.mp4": dict(duplicate_pts="all"),
        "delayed-video-start.mp4": dict(duplicate_pts="delayed"),
    }
    for name, options in cases.items():
        path = OUT / name
        path.write_bytes(rewrite(source, **options))
        print(f"{name}: {path.stat().st_size} bytes")
    inter = OUT / "control-inter-frames.mp4"
    inter.write_bytes(seeds["control-inter-frames.mp4"])
    source = inter.read_bytes()
    assert [k for k, _ in children(source)].index(b"mdat") < [k for k, _ in children(source)].index(b"moov")
    def origin(data):
        for kind, payload in children(data):
            if kind == b"elst":
                assert payload[0] == 0 and int.from_bytes(payload[4:8], "big") == 1
                return int.from_bytes(payload[12:16], "big", signed=True)
            if kind in {b"moov", b"trak", b"edts"}:
                found = origin(payload)
                if found is not None:
                    return found
        return None
    start = origin(source)
    assert start is not None and start >= 0
    (OUT / "edit-gap-inter-frames.mov").write_bytes(rewrite(source, edits=[(1500, start), (8500, start + 3500)]))
    audio = OUT / "control-aac.mp4"
    audio.write_bytes(seeds["control-aac.mp4"])
    def audio_variant(data, quicktime=False):
        out = bytearray()
        for kind, payload in children(data):
            if kind in {b"moov", b"trak", b"mdia", b"minf", b"stbl"}:
                payload = audio_variant(payload, quicktime)
            elif kind == b"stsd":
                entries = bytearray(payload[:8])
                for codec, entry in children(payload[8:]):
                    if codec == b"mp4a":
                        entry = bytearray(entry)
                        if quicktime:
                            entry[8:10] = struct.pack(">H", 1)
                            entry = entry[:28] + struct.pack(">IIII", 1024, 0, 0, 2) + atom(b"wave", bytes(entry[28:]))
                        else:
                            entry[16:18] = struct.pack(">H", 2)
                        entry = bytes(entry)
                    entries += atom(codec, entry)
                payload = bytes(entries)
            out += atom(kind, payload)
        return bytes(out)
    (OUT / "aac-quicktime-v1.mov").write_bytes(audio_variant(audio.read_bytes(), True))
    (OUT / "aac-stale-channels.mp4").write_bytes(audio_variant(audio.read_bytes()))
    for name, payload in seeds.items():
        if name.startswith("opus-"):
            (OUT / name).write_bytes(payload)
    (OUT / "opus-regression.webm").write_bytes((OUT / "opus-mono.webm").read_bytes())

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=OUT)
    OUT = parser.parse_args().output_dir
    main()
