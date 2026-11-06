"""Offline authored Matroska and MP4 subtitle witnesses for UTF-8 BOM handling."""
from pathlib import Path
import hashlib, json, struct
from generate_vp9_chroma import ebml, atom, word
from generate_mp4_seek_io_regressions import generate as video_source
from generate_matroska_ssa_text import u, track, SSA, ASS
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_mp4_subtitle_negative_clock import text_entry

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/subtitle-bom-handling"

def build_mp4(own, raw_packets):
    parts = list(atoms(own["three-colors.mp4"]))
    moov = parts[2][1]
    vtrack = next(b for k, b in atoms(moov) if k == b"trak")
    moov = b"".join(atom(k, b[:-4] + word(3) if k == b"mvhd" else b) for k, b in atoms(moov))
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
                    + atom(b"stsz", word(0, 0, 3, *map(len, raw_packets)))
                    + atom(b"stsc", word(0, 1, 1, 3, 1))
                    + atom(b"stco", word(0, 1, offset))
                )
            out += atom(k, b)
        return out

    size = len(atom(b"moov", moov + atom(b"trak", subtitle(vtrack, 0))))
    sub = subtitle(vtrack, len(prefix) + size + 8)
    return prefix + atom(b"moov", moov + atom(b"trak", sub)) + atom(b"mdat", b"".join(raw_packets))

def generate():
    source = video_source()
    video = [source[f"frame-{i}.packet"] for i in range(3)]
    files = {f"frame-{i}.packet": video[i] for i in range(3)}
    for i in range(3):
        files[f"frame-{i}.yuv16"] = source[f"frame-{i}.yuv16"]
    mkv_cases = []
    for name, codec, raw_payloads, expected_texts in [
        (
            "ass-bom.mkv",
            "ASS",
            [
                "\ufeff0,0,Default,,0,0,0,,First BOM line",
                "\ufeffDialogue: 0,0:00:00.04,0:00:00.06,Default,,0,0,0,,Middle BOM\\Nnext",
                "\ufeff2,0,Default,,0,0,0,,{\\i1}Last BOM line{\\i0}",
            ],
            ["First BOM line", "Middle BOM\nnext", "Last BOM line"],
        ),
        (
            "ssa-bom.mkv",
            "SSA",
            [
                "\ufeff0,Marked=0,Default,NTP,0000,0000,0000,!Effect,First SSA BOM",
                "\ufeff1,,Default,NTP,0000,0000,0000,!Effect,Middle SSA BOM",
                "\ufeff2,Marked=1,Default,NTP,0000,0000,0000,!Effect,Last SSA BOM",
            ],
            ["First SSA BOM", "Middle SSA BOM", "Last SSA BOM"],
        ),
        (
            "utf8-bom.mkv",
            "UTF8",
            [
                "\ufeff<i>First UTF8 BOM</i>",
                "\ufeffMiddle UTF8\\Nnext",
                "\ufeffLast UTF8 BOM",
            ],
            ["First UTF8 BOM", "Middle UTF8\nnext", "Last UTF8 BOM"],
        ),
    ]:
        payloads = [p.encode("utf-8") for p in raw_payloads]
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
        private = ebml("63a2", SSA if codec == "SSA" else ASS) if codec in ("SSA", "ASS") else b""
        st = track(2, 17, f"S_TEXT/{codec}", private)
        entries = []
        for i in range(3):
            entries.append(ebml("a3", b"\x81" + (i * 40).to_bytes(2, "big") + b"\x80" + video[i]))
            entries.append(
                ebml(
                    "a0",
                    ebml("a1", b"\x82" + (i * 40).to_bytes(2, "big") + b"\x80" + payloads[i])
                    + ebml("9b", u(20)),
                )
            )
        segment = info + ebml("1654ae6b", vt + st) + ebml("1f43b675", ebml("e7", u(0)) + b"".join(entries))
        files[name] = header + ebml("18538067", segment)
        mkv_cases.append({"file": name, "codec": f"S_TEXT/{codec}", "texts": expected_texts})

    mp4_texts = [
        "\ufeff<b>First tx3g BOM</b>",
        "\ufeffMiddle tx3g\nnext",
        "\ufeffLast tx3g BOM",
    ]
    # Sample 0 uses group_text with params=0 (`\x00\x00` + length + UTF-8 with BOM),
    # Sample 1 uses standard length-prefixed UTF-8 with BOM,
    # Sample 2 uses a box chain (`text` box containing UTF-8 with BOM).
    p0_bytes = mp4_texts[0].encode("utf-8")
    p1_bytes = mp4_texts[1].encode("utf-8")
    p2_bytes = mp4_texts[2].encode("utf-8")
    raw_mp4_packets = [
        b"\x00\x00" + struct.pack(">H", len(p0_bytes)) + p0_bytes,
        struct.pack(">H", len(p1_bytes)) + p1_bytes,
        struct.pack(">I", 8 + len(p2_bytes)) + b"text" + p2_bytes,
    ]
    mp4_bytes = build_mp4(source, raw_mp4_packets)
    files["tx3g-bom.mp4"] = mp4_bytes
    mp4_expected = ["First tx3g BOM", "Middle tx3g\nnext", "Last tx3g BOM"]

    files["manifest.json"] = (
        json.dumps(
            {
                "generator": "scripts/generate_subtitle_bom_handling.py",
                "mkv_cases": mkv_cases,
                "mp4_case": {
                    "file": "tx3g-bom.mp4",
                    "texts": mp4_expected,
                },
                "sha256": {k: hashlib.sha256(v).hexdigest() for k, v in sorted(files.items())},
            },
            indent=2,
        )
        + "\n"
    ).encode()
    return files

if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    for k, v in generate().items():
        (OUT / k).write_bytes(v)
