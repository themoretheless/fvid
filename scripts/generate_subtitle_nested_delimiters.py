"""Offline authored Matroska and MP4 subtitle witnesses for Marked=0 SSA packets and nested delimiter stripping."""
from pathlib import Path
import hashlib, json, struct
from generate_vp9_chroma import ebml, atom, word
from generate_mp4_seek_io_regressions import generate as video_source
from generate_matroska_ssa_text import u, track, SSA, ASS
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_mp4_subtitle_negative_clock import text_entry

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/subtitle-nested-delimiters"

def build_mp4(own, raw_texts):
    parts = list(atoms(own["three-colors.mp4"]))
    moov = parts[2][1]
    vtrack = next(b for k, b in atoms(moov) if k == b"trak")
    moov = b"".join(atom(k, b[:-4] + word(3) if k == b"mvhd" else b) for k, b in atoms(moov))
    packets = [struct.pack(">H", len(t.encode())) + t.encode() for t in raw_texts]
    prefix = b"".join(atom(k, b) for k, b in parts[:2])

    def subtitle(data, offset):
        out = b""
        for k, b in atoms(data):
            if k in [b"mdia", b"minf"]:
                b = subtitle(b, offset)
            elif k == b"tkhd":
                b = b[:12] + word(2) + b[16:]
            elif k == b"mdhd":
                b = b[:12] + word(50, 3) + b[20:]
            elif k == b"hdlr":
                b = word(0, 0) + b"sbtl" + bytes(12) + b"Own text\0"
            elif k == b"vmhd":
                k = b"nmhd"
                b = word(0)
            elif k == b"stbl":
                b = (
                    atom(b"stsd", word(0, 1) + text_entry())
                    + atom(b"stts", word(0, 1, 3, 1))
                    + atom(b"stsz", word(0, 0, 3, *map(len, packets)))
                    + atom(b"stsc", word(0, 1, 1, 3, 1))
                    + atom(b"stco", word(0, 1, offset))
                )
            out += atom(k, b)
        return out

    size = len(atom(b"moov", moov + atom(b"trak", subtitle(vtrack, 0))))
    sub = subtitle(vtrack, len(prefix) + size + 8)
    return prefix + atom(b"moov", moov + atom(b"trak", sub)) + atom(b"mdat", b"".join(packets)), packets

def generate():
    source = video_source()
    video = [source[f"frame-{i}.packet"] for i in range(3)]
    files = {f"frame-{i}.packet": video[i] for i in range(3)}
    for i in range(3):
        files[f"frame-{i}.yuv16"] = source[f"frame-{i}.yuv16"]
    mkv_cases = []
    for name, codec, raw_payloads, expected_texts in [
        (
            "ssa-marked0.mkv",
            "SSA",
            [
                "0,Marked=0,Default,NTP,0000,0000,0000,!Effect,First, with commas",
                "1,Marked=1,Default,NTP,0000,0000,0000,!Effect,{\\fnArial<Bold>}Middle\\Nnext",
                "2,Marked=0,Default,NTP,0000,0000,0000,!Effect,<span data=\"{x}\">Last Привет</span>",
            ],
            ["First, with commas", "Middle\nnext", "Last Привет"],
        ),
        (
            "ass-nested-delimiters.mkv",
            "ASS",
            [
                "0,0,Default,,0,0,0,,{\\fnArial<Bold}First line",
                "1,0,Default,,0,0,0,,{\\r<Style}Middle\\N{\\i1>0}next",
                "2,0,Default,,0,0,0,,<font face=\"{Serif\">Last line</font>",
            ],
            ["First line", "Middle\nnext", "Last line"],
        ),
        (
            "utf8-nested-delimiters.mkv",
            "UTF8",
            [
                "<span style=\"{color:red\">First line</span>",
                "{\\fnArial<Bold}Middle\\N<i data=\"{1\">next</i>",
                "{\\an8>}Last line",
            ],
            ["First line", "Middle\nnext", "Last line"],
        ),
    ]:
        payloads = [p.encode() for p in raw_payloads]
        header = ebml("1a45dfa3", ebml("4282", b"matroska") + ebml("4287", b"\x04") + ebml("4285", b"\x02"))
        info = ebml(
            "1549a966",
            ebml("2ad7b1", u(1_000_000))
            + ebml("4489", struct.pack(">d", 120.0))
            + ebml("4d80", b"fvid-synthetic")
            + ebml("5741", b"fvid-synthetic"),
        )
        vt = track(
            1,
            1,
            "V_VP9",
            ebml("23e383", u(40_000_000)) + ebml("e0", ebml("b0", u(16)) + ebml("ba", u(16))),
        )
        sub_extra = ebml("63a2", SSA if codec == "SSA" else ASS) if codec != "UTF8" else b""
        st = track(2, 17, "S_TEXT/" + codec, sub_extra)
        cluster = ebml("e7", b"\x00")
        for i, (v, p) in enumerate(zip(video, payloads)):
            cluster += ebml("a3", b"\x81" + struct.pack(">h", i * 40) + b"\x80" + v)
            cluster += ebml("a0", ebml("a1", b"\x82" + struct.pack(">h", i * 40) + b"\x00" + p) + ebml("9b", u(20)))
            files[f"{name}-{i}.packet"] = p
        files[name] = header + ebml("18538067", info + ebml("1654ae6b", vt + st) + ebml("1f43b675", cluster))
        mkv_cases.append(dict(file=name, codec=codec, texts=expected_texts))

    mp4_raw = [
        "<span style=\"{color:red\">First line</span>",
        "{\\fnArial<Bold}Middle\\N<i data=\"{1\">next</i>",
        "{\\an8>}Last line",
    ]
    mp4_expected = ["First line", "Middle\nnext", "Last line"]
    mp4_bytes, mp4_packets = build_mp4(source, mp4_raw)
    files["tx3g-nested-delimiters.mp4"] = mp4_bytes
    for i, p in enumerate(mp4_packets):
        files[f"tx3g-nested-delimiters.mp4-{i}.packet"] = p

    files["manifest.json"] = (
        json.dumps(
            dict(
                provenance="Pure authored 16x16 three-color VP9 with Matroska S_TEXT/SSA Marked=0 blocks, S_TEXT/ASS and S_TEXT/UTF8 blocks, and MP4 tx3g samples carrying cross-nested angle/brace delimiters inside ASS override blocks and HTML tags. No private media, FFmpeg or network.",
                mkv_cases=mkv_cases,
                mp4_case=dict(file="tx3g-nested-delimiters.mp4", texts=mp4_expected),
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
