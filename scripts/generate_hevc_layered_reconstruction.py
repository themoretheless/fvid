#!/usr/bin/env python3
"""Owned coded-picture reconstruction witnesses in independent/dependent layers.

Preserve entropy and saved HM pixel oracles from original PCM fixtures. Rewrite
only layer/parameter bindings and the Annex F IDR header. No external codec,
FFmpeg, network or private source material. Run separately from ordinary tests.
"""
import json
import subprocess
from pathlib import Path
from generate_hevc_layered_sps import vps
from generate_hevc_colour_plane_fixtures import bits, packed, escape, unescape, ue_end
from hevc_fixture_mp4 import mux
from generate_hevc_poc_reset_fixtures import Reader, ue

ROOT = Path(__file__).resolve().parents[1]
D = ROOT / "tests/fixtures/playback-errors"


def configuration(data):
    at = 23
    out = {}
    for _ in range(data[22]):
        kind, count = data[at] & 63, int.from_bytes(data[at+1:at+3], "big")
        at += 3
        out[kind] = []
        for _ in range(count):
            n = int.from_bytes(data[at:at+2], "big")
            at += 2
            out[kind].append(data[at:at+n])
            at += n
    assert at == len(data)
    return out


def layer(nal, id):
    head = int.from_bytes(nal[:2], "big")
    return ((head & ~0x1f8) | (id << 3)).to_bytes(2, "big") + nal[2:]


def parameter_ids(nal, start, ids):
    value = bits(unescape(nal[2:]))
    end = start
    for _ in ids:
        end = ue_end(value, end)
    # Preserve syntax through rbsp_stop_one_bit, then re-align padding.
    value = value[:start] + "".join(ue(i) for i in ids) + value[end:value.rfind("1")+1]
    return nal[:2] + escape(packed(value))


def slice_pps(header, start):
    return header[:start] + ue(1) + header[ue_end(header, start):]


def main():
    build = subprocess.check_output([
        "cargo", "build", "--quiet", "--locked", "--offline", "--no-default-features",
        "--example", "hevc_colour_plane_fixture_meta", "--message-format=json",
    ],cwd=ROOT,text=True)
    helper = next(row["executable"] for row in map(json.loads,build.splitlines())
                  if row.get("reason")=="compiler-artifact" and row.get("executable")
                  and row["target"]["name"]=="hevc_colour_plane_fixture_meta")
    cases = []
    for label, seed in [
        ("pcm", "hevc-pcm-reference-rext8"),
        ("multislice", "hevc-pcm-dependent-rext8"),
        ("colour-planes", "hevc-separate-colour-planes-distinct-synthetic"),
    ]:
        meta = json.loads(subprocess.check_output([helper,str(D / (seed + ".mp4"))],cwd=ROOT))
        params = configuration(bytes(meta["configuration"]))
        fmt = dict(size=meta["dimensions"], depth=[meta["depth"],meta["chroma_depth"]],
                   chroma=meta["chroma_format"], separate=meta["separate_colour_plane"], crop=[0]*4)
        for independent in (True, False):
            prefix = f"hevc-layered-reconstruct-{label}-" + ("independent" if independent else "dependent")
            vps_nal, _, _ = vps(fmt, independent=independent, poc_absent=independent, base_format=fmt)
            # Keep the owned source PTL for both base and independent layers.
            vbits = bits(unescape(vps_nal[2:]))
            source_ptl = bits(unescape(params[33][0][2:]))[8:104]
            vps_nal = vps_nal[:2] + escape(packed(vbits[:32] + source_ptl + vbits[128:]))
            # Dependent layers use the shared base SPS; independent layers own it.
            sps = parameter_ids(layer(params[33][0], 3), 104, [1]) if independent else params[33][0]
            pps = parameter_ids(layer(params[34][0], 3), 0, [1, int(independent)])
            slices = []
            for h in meta["packets"][0]:
                assert h["idr"]
                raw = bytes(h["rbsp"])
                value = bits(raw)
                end = h["entropy_byte_offset"] * 8
                align = value[:end].rfind("1")
                header = value[:align]
                if not independent and not h["dependent"]:
                    insert = ue_end(value, 2)
                    if not h["first"]:
                        insert += int(meta["dependent_enabled"]) + (meta["ctu_count"]-1).bit_length()
                    insert += meta["extra_bits"]
                    insert = ue_end(value, insert) + int(meta["output_flag"])
                    if meta["separate_colour_plane"]:
                        insert += 2
                    # IDR nonbase LSB, then disabled inter-layer prediction.
                    header = value[:insert] + "0"*meta["poc_bits"] + "0" + value[insert:align]
                body = packed(slice_pps(header, 2) + "1") + raw[end//8:]
                slices.append(((h["kind"] << 9) | (3 << 3) | 1).to_bytes(2,"big") + escape(body))
            base_slices = [((h["kind"] << 9) | 1).to_bytes(2,"big") + escape(bytes(h["rbsp"]))
                           for h in meta["packets"][0]]
            parameters = [vps_nal, params[33][0]]
            if sps != params[33][0]: parameters.append(sps)
            parameters += [params[34][0], pps]
            all_slices = base_slices + slices
            stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*parameters,*all_slices])
            movie = mux(stream,meta["depth"],*meta["dimensions"],chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],access_units=[all_slices],multilayer=True)
            (D / (prefix+".mp4")).write_bytes(movie)
            cases.append(dict(file=prefix+".mp4",seed=seed+".mp4",independent=independent,layer=3,ols=1))
            if label == "pcm" and not independent:
                predicted = []
                base_predicted = []
                for h in meta["packets"][1]:
                    assert not h["idr"] and h["first"] and not h["dependent"]
                    assert h["long_term"] == 0
                    raw = bytes(h["rbsp"])
                    value = bits(raw)
                    end = h["entropy_byte_offset"]*8
                    align = value[:end].rfind("1")
                    r = Reader(value,1)
                    r.ue()
                    r.read(meta["extra_bits"])
                    assert r.ue() in (0,1)  # preserve authored B/P entropy initialization
                    if meta["output_flag"]: r.read(1)
                    r.read(meta["poc_bits"])
                    start = r.at
                    if r.read(1):
                        if meta["short_term_sets"] > 1:
                            r.read((meta["short_term_sets"]-1).bit_length())
                    else:
                        r.read(h["short_term_bit_length"])
                    if meta["long_term_present"]:
                        if meta["long_term_entries"]: assert r.ue() == 0
                        assert r.ue() == 0
                    if meta["temporal_mvp_enabled"]: r.read(1)
                    # Empty temporal RPS, one enabled direct inter-layer source.
                    empty = "0" + ("0" if meta["short_term_sets"] else "") + ue(0)+ue(0)
                    if meta["long_term_present"]:
                        empty += (ue(0) if meta["long_term_entries"] else "") + ue(0)
                    if meta["temporal_mvp_enabled"]: empty += str(int(h["temporal_mvp"]))
                    header = slice_pps(value[:start] + empty + "1" + value[r.at:align], 1) + "1"
                    predicted.append(((h["kind"]<<9)|(3<<3)|1).to_bytes(2,"big") + escape(packed(header)+raw[end//8:]))
                    base_predicted.append(((h["kind"]<<9)|1).to_bytes(2,"big") + escape(raw))
                groups = [all_slices,base_predicted+predicted]
                stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*parameters,*groups[0],*groups[1]])
                filename = "hevc-layered-reconstruct-inter-prediction.mp4"
                (D/filename).write_bytes(mux(stream,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=groups,multilayer=True))
                cases.append(dict(file=filename,seed=seed+".mp4",independent=False,layer=3,ols=1,inter=True))
    (D / "hevc-layered-reconstruction.json").write_text(json.dumps(dict(cases=cases),indent=2)+"\n")


if __name__ == "__main__":
    main()
