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
    replacements = []
    access_cases = []
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
            def access_case(name, units, *, error=None, inband=False, **expect):
                filename = "hevc-layered-au-" + name + ".mp4"
                authored = b"".join(b"\x00\x00\x00\x01"+n for n in [*parameters,*units])
                (D/filename).write_bytes(mux(authored,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=[units],multilayer=True,inband_parameters=inband))
                access_cases.append(dict(file=filename,seed=seed+".mp4",baseline=prefix+".mp4",error=error,**expect))
            if label == "pcm" and not independent:
                aud = ((35<<9)|1).to_bytes(2,"big") + b"\x10"
                eos0 = ((36<<9)|1).to_bytes(2,"big") + b"\x80"
                eos3 = ((36<<9)|(3<<3)|1).to_bytes(2,"big") + b"\x80"
                eob = ((37<<9)|1).to_bytes(2,"big") + b"\x80"
                ignored = ((0<<9)|(63<<3)|1).to_bytes(2,"big") + b"\x00\x00\x03\xff"
                access_case("ignored-layer63",[ignored,aud,*all_slices,ignored],ignored=True)
                access_case("aud-eos-eob",[aud,*all_slices,eos0,eos3,eob],ended=9,eob=True)
                access_case("eos-base",[*all_slices,eos0],ended=1)
                access_case("eos-enhancement",[*all_slices,eos3],ended=8)
                access_case("descending-layer",slices+base_slices,error="HEVC layered pictures must increase in layer order")
                access_case("duplicate-first-slice",[*all_slices,slices[0]],error="HEVC slice addresses must increase within one picture")
                access_case("late-aud",[*base_slices,aud,*slices],error="HEVC access-unit delimiter must be first and unique")
                access_case("duplicate-aud",[aud,aud,*all_slices],error="HEVC access-unit delimiter must be first and unique")
                access_case("after-eob",[*all_slices,eob,eos3],error="HEVC NAL follows end of bitstream")
                access_case("layer-after-eos",[eos3,*all_slices],error="HEVC NAL follows end of layer sequence")
                access_case("bad-eos-trailing",[*all_slices,eos3[:-1]+b"\xc0"],error="invalid HEVC EOS/EOB trailing bits")
                access_case("prefix-after-last-vcl",[*all_slices,pps],inband=True,error="HEVC prefix NAL follows last access-unit VCL")
                shared = parameter_ids(params[34][0],0,[1,0])
                access_case("prefix-between-layers",[*base_slices,shared,*slices],inband=True,pps_nal_layer=0)
                value = bits(unescape(slices[0][2:]))
                r = Reader(value,2); r.ue(); r.read(meta["extra_bits"]); r.ue()
                if meta["output_flag"]: r.read(1)
                at = r.at
                changed = slices[0][:2]+escape(packed(value[:at]+f"{1:0{meta['poc_bits']}b}"+value[at+meta["poc_bits"]:]))
                access_case("poc-mismatch",[*base_slices,changed,*slices[1:]],error="HEVC access unit derived POCs disagree")
            if label == "multislice" and not independent:
                access_case("missing-first-slice",[*base_slices,*slices[1:]],error="HEVC layered picture must begin with the first slice")
                access_case("repeated-slice-address",[*all_slices,slices[-1]],error="HEVC dependent segment address out of range")
                import generate_hevc_layered_pps as gp
                changed = gp.pps(layer=3,id=1)
                access_case("pps-update-mid-picture",[*base_slices,slices[0],changed,*slices[1:]],inband=True,error="HEVC parameter binding changes within a picture")
                access_case("identical-pps-new-nal-layer",[*base_slices,slices[0],layer(pps,0),*slices[1:]],inband=True,pps_nal_layer=0)
            if label == "colour-planes" and independent:
                access_case("missing-colour-plane",[*base_slices,*slices[:-1]],error="HEVC layered access unit is missing a colour plane")
            if label == "pcm" and independent:
                # Valid content update under SPS 0: changing its progressive-source PTL flag does
                # not change entropy/pixels, but needs a new layer sequence.
                value = bits(unescape(params[33][0][2:]))
                changed_sps = params[33][0][:2] + escape(packed(value[:48] + str(1-int(value[48])) + value[49:]))
                updated_parameters = [vps_nal, params[33][0], changed_sps, sps, params[34][0], pps]
                stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*updated_parameters,*all_slices])
                filename = "hevc-layered-sps-content-update.mp4"
                (D/filename).write_bytes(mux(stream,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=[all_slices],multilayer=True))
                # Distinct received NAL layers do not make distinct ID spaces.
                # Keep the overwritten units in hvcC so lookup order is tested.
                for kind in (33, 34):
                    replaced = parameter_ids(sps if kind == 33 else pps,
                                             104 if kind == 33 else 0,
                                             [0] if kind == 33 else [0, 0])
                    collision_params = ([vps_nal, params[33][0], replaced, params[34][0], pps]
                                        if kind == 33 else
                                        [vps_nal, params[33][0], sps, params[34][0], replaced])
                    stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*collision_params,*all_slices])
                    filename = f"hevc-layered-global-{kind}-replacement.mp4"
                    (D/filename).write_bytes(mux(stream,meta["depth"],*meta["dimensions"],
                        chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                        access_units=[all_slices],multilayer=True))
                    replacements.append(dict(file=filename,kind=kind,id=0,latest_layer=3))
                # One base-layer PPS may be activated in multiple independent
                # layers. The final PPS 1 replaces the earlier layer-3 PPS 1.
                shared = parameter_ids(params[34][0], 0, [1, 0])
                stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*parameters,shared,*all_slices])
                filename = "hevc-layered-reconstruct-shared-pps.mp4"
                (D/filename).write_bytes(mux(stream,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=[all_slices],multilayer=True))
                cases.append(dict(file=filename,seed=seed+".mp4",independent=True,
                                  layer=3,ols=1,sps_id=0,shared_pps=True))
            if label == "pcm" and not independent:
                # A dependent SPS omits PTL/geometry/ordering and obtains them
                # from the active VPS. Keep the owned coded-picture suffix.
                value = bits(unescape(params[33][0][2:]))
                assert value[4:7] == "000"  # owned single-temporal-layer seed
                r = Reader(value, 104)
                assert r.ue() == 0
                chroma = r.ue()
                if chroma == 3: r.read(1)
                r.ue(); r.ue()
                if r.read(1):
                    for _ in range(4): r.ue()
                r.ue(); r.ue()
                poc_start = r.at
                r.ue()
                poc_end = r.at
                r.read(1)
                r.ue(); r.ue(); r.ue()
                suffix = r.at
                for _ in range(6): r.ue()
                assert r.read(1) == 0  # no scaling syntax needs re-authoring
                body = "0000111" + ue(1) + "0" + value[poc_start:poc_end] + value[suffix:value.rfind("1")+1]
                multilayer_sps = ((33<<9)|(3<<3)|1).to_bytes(2,"big") + escape(packed(body))
                multilayer_pps = parameter_ids(layer(params[34][0],3),0,[1,1])
                new_parameters = [vps_nal,params[33][0],multilayer_sps,params[34][0],multilayer_pps]
                stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*new_parameters,*all_slices])
                filename = "hevc-layered-reconstruct-multilayer-sps.mp4"
                (D/filename).write_bytes(mux(stream,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=[all_slices],multilayer=True))
                cases.append(dict(file=filename,seed=seed+".mp4",independent=False,
                                  layer=3,ols=1,sps_id=1,multilayer_sps=True))
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
                highest_vps, _, _ = vps(fmt, independent=False, base_format=fmt, output_highest=True)
                highest_bits = bits(unescape(highest_vps[2:]))
                highest_vps = highest_vps[:2] + escape(packed(highest_bits[:32]+source_ptl+highest_bits[128:]))
                highest_parameters = [highest_vps,*parameters[1:]]
                highest_stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*highest_parameters,*groups[0],*groups[1]])
                filename = "hevc-layered-reconstruct-inter-prediction-output-highest.mp4"
                (D/filename).write_bytes(mux(highest_stream,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=groups,multilayer=True))
                cases.append(dict(file=filename,seed=seed+".mp4",independent=False,layer=3,ols=1,inter=True,output_highest=True))
    # Parameter-state witness, not coded-picture acceptance: inference must
    # follow the latest active base PPS even if the enhancement NAL is unchanged.
    import generate_hevc_layered_pps as gp
    prefix = D / "hevc-layered-pps-infer-from-sps"
    read = lambda suffix: Path(str(prefix) + suffix).read_bytes()
    current_sps = parameter_ids(read(".current.sps"), 7, [1])
    current_pps = parameter_ids(read(".pps"), 0, [1, 1])
    (D / "hevc-layered-activation-inference.mp4").write_bytes(
        gp.movie(read(".vps"),read(".base.sps"),current_sps,read(".base.pps"),current_pps))
    (D / "hevc-layered-access-units.json").write_text(json.dumps(dict(cases=access_cases),indent=2)+"\n")
    (D / "hevc-layered-reconstruction.json").write_text(json.dumps(dict(cases=cases,replacements=replacements),indent=2)+"\n")


if __name__ == "__main__":
    main()
