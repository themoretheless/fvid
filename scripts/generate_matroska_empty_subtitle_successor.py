"""Offline authored VP9/UTF8/SSA/ASS Matroska witnesses for empty/comment subtitle blocks after untimed cues."""
from pathlib import Path
import hashlib, json, struct
from generate_vp9_chroma import ebml
from generate_mp4_seek_io_regressions import generate as video_source
from generate_matroska_ssa_text import u, track, SSA, ASS

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/matroska-empty-subtitle-successor"

def generate():
    source = video_source()
    files = {f"frame-{i}.packet": source[f"frame-{i}.packet"] for i in range(3)}
    cases = []
    for codec in ["UTF8", "SSA", "ASS"]:
        for mode, raw_texts, pts, durations in [
            ("empty-middle", ["First caption", "   ", "Later caption"], [0, 30_000_000, 80_000_000], [None, None, 40_000_000]),
            ("markup-middle", ["First caption", "<i></i>" if codec == "UTF8" else "{\\i1}{\\i0}", "Later caption"], [0, 30_000_000, 80_000_000], [None, None, 40_000_000]),
            ("empty-tail", ["First caption", "Second caption", "   "], [0, 40_000_000, 70_000_000], [40_000_000, None, None]),
            ("visible-control", ["First caption", "Middle caption", "Later caption"], [0, 30_000_000, 80_000_000], [None, None, 40_000_000]),
        ]:
            packets = [
                t.encode()
                if codec == "UTF8"
                else f'{i},{"" if codec == "SSA" else "0"},Default,,0,0,0,,{t}'.encode()
                for i, t in enumerate(raw_texts)
            ]
            for i, p in enumerate(packets):
                files[f"{codec.lower()}-{mode}-{i}.packet"] = p
            name = f"{codec.lower()}-{mode}.mkv"
            header = ebml("1a45dfa3", ebml("4282", b"matroska") + ebml("4287", b"\x04") + ebml("4285", b"\x02"))
            info = ebml(
                "1549a966",
                ebml("2ad7b1", u(1_000_000))
                + ebml("4489", struct.pack(">d", 120.0))
                + ebml("4d80", b"fvid-synthetic")
                + ebml("5741", b"fvid-synthetic"),
            )
            video = track(
                1,
                1,
                "V_VP9",
                ebml("23e383", u(40_000_000)) + ebml("e0", ebml("b0", u(16)) + ebml("ba", u(16))),
            )
            sub_extra = ebml("63a2", SSA if codec == "SSA" else ASS) if codec != "UTF8" else b""
            sub = track(2, 17, "S_TEXT/" + codec, sub_extra)
            cluster = ebml("e7", b"\x00")
            for i in range(3):
                v_ts = struct.pack(">h", i * 40)
                cluster += ebml("a3", b"\x81" + v_ts + b"\x80" + source[f"frame-{i}.packet"])
                s_ts = struct.pack(">h", pts[i] // 1_000_000)
                if durations[i] is not None:
                    payload = b"\x82" + s_ts + b"\x00" + packets[i]
                    cluster += ebml("a0", ebml("a1", payload) + ebml("9b", u(durations[i] // 1_000_000)))
                else:
                    payload = b"\x82" + s_ts + b"\x80" + packets[i]
                    cluster += ebml("a3", payload)
            files[name] = header + ebml("18538067", info + ebml("1654ae6b", video + sub) + ebml("1f43b675", cluster))
            non_empty = [
                (t.strip(), p, d)
                for t, p, d in zip(raw_texts, pts, durations)
                if t.strip() and t not in ("<i></i>", "{\\i1}{\\i0}")
            ]
            cues = []
            for idx, (t, p, d) in enumerate(non_empty):
                end = p + d if d is not None else next((q for _, q, _ in non_empty[idx + 1 :] if q > p), p + 2_000_000_000)
                if end > p:
                    cues.append(dict(text=t, start_ns=p, end_ns=end))
            cases.append(
                dict(
                    file=name,
                    codec=codec.lower(),
                    mode=mode,
                    pts_ns=pts,
                    packet_durations_ns=durations,
                    cues=cues,
                )
            )
    files["manifest.json"] = (
        json.dumps(
            dict(
                provenance="Pure authored 16x16 three-color VP9 with invented UTF8/SSA/ASS text. Empty, whitespace-only and markup-only subtitle blocks following untimed SimpleBlock cues, plus visible controls. No private media, FFmpeg or network.",
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
