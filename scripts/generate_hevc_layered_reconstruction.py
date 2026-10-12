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
    output_cases = []
    random_access_cases = []
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
                # F.10.1 extraction: timing SEI precedes removed layers/temporal NALs.
                sei = lambda payload: ((39<<9)|1).to_bytes(2,"big")+escape(bytes(payload))
                timing = [sei([kind,0,128]) for kind in (0,1,130)]
                retained = [sei([144,4,0,1,0,2,128]), sei([133,3,1,0,128,128])]
                high_temporal = ((41<<9)|2).to_bytes(2,"big")+b"\x80"
                extraction_group = [*timing,*retained,high_temporal,*groups[0]]
                extraction_stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*parameters,*extraction_group])
                (D/"hevc-layered-extraction-timing.mp4").write_bytes(mux(extraction_stream,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=[extraction_group],multilayer=True))
                split_groups = [[*timing,*retained],[high_temporal,*groups[0]]]
                (D/"hevc-layered-extraction-split-timing.mp4").write_bytes(mux(extraction_stream,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=split_groups,multilayer=True))
                # Intra CRA/RASL/RADL/TRAIL headers with original PCM entropy.
                # Empty local RPS and disabled temporal/inter-layer prediction.
                def output_pps(nal):
                    value = bits(unescape(nal[2:]))
                    r = Reader(value); r.ue(); r.ue()
                    at = r.at + 1
                    return nal[:2] + escape(packed(value[:at]+"1"+value[at+1:]))
                def intra_nal(kind, poc, target_layer, extension=None, inter_layer=True, picture_output=None, discard_prior=False, retain_zero=False):
                    h = meta["packets"][0][0]
                    raw = bytes(h["rbsp"])
                    value = bits(raw)
                    end = h["entropy_byte_offset"]*8
                    align = value[:end].rfind("1")
                    r = Reader(value,2); r.ue()
                    start = r.at
                    r.read(meta["extra_bits"]); assert r.ue() == 2
                    if meta["output_flag"]: r.read(1)
                    head = "1"+(str(int(discard_prior)) if 16 <= kind <= 23 else "")+ue(int(target_layer != 0))+value[start:r.at]
                    if picture_output is not None:
                        assert not meta["output_flag"]
                        head += str(int(picture_output))
                    head += f"{poc:0{meta['poc_bits']}b}"+"0"
                    if meta["short_term_sets"]: head += "0"
                    head += (ue(1)+ue(0)+ue(poc-1)+"0") if retain_zero else ue(0)+ue(0)
                    if meta["long_term_present"]:
                        if meta["long_term_entries"]: head += ue(0)
                        head += ue(0)
                    if meta["temporal_mvp_enabled"]: head += "0"
                    if target_layer and inter_layer: head += "0"
                    head += value[r.at:align]
                    if extension is not None:
                        extension += "0"*((-len(extension))%8)
                        head += ue(len(extension)//8)+extension
                    head += "1"
                    return ((kind<<9)|(target_layer<<3)|1).to_bytes(2,"big")+escape(packed(head)+raw[end//8:])
                for name, eos, fallback in [("entry-and-later-cra",False,False),("eos-before-later-cra",True,False),("eob-before-later-cra","bitstream",False),("alternative-radl",False,True)]:
                    kinds = [(21,21),(6,8) if fallback else (8,8),(0,0),(21,21),(8,8)]
                    pocs = [2,1,3,6,5]
                    rasl_groups = [[intra_nal(k,poc,id) for k,id in zip(pair,[0,3])] for pair,poc in zip(kinds,pocs)]
                    if eos: rasl_groups[2].append((((37 if eos=="bitstream" else 36)<<9)|1).to_bytes(2,"big")+b"\x80")
                    rasl_parameters = parameters
                    if fallback:
                        rvps,_,_ = vps(fmt,independent=False,base_format=fmt,output_highest=True,alternative_output=True)
                        rv = bits(unescape(rvps[2:]))
                        rvps = rvps[:2]+escape(packed(rv[:32]+source_ptl+rv[128:]))
                        rasl_parameters = [rvps,*parameters[1:]]
                    stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*rasl_parameters,*sum(rasl_groups,[])])
                    filename = "hevc-layered-random-access-"+name+".mp4"
                    (D/filename).write_bytes(mux(stream,meta["depth"],*meta["dimensions"],chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],access_units=rasl_groups,multilayer=True))
                    if name == "alternative-radl":
                        (D/"hevc-layered-random-access-alternative-radl-ctts.mp4").write_bytes(mux(
                            stream,meta["depth"],*meta["dimensions"],chroma_format=meta["chroma_format"],
                            chroma_depth=meta["chroma_depth"],access_units=rasl_groups,multilayer=True,
                            presentation_order=[1,0,2,4,3]))
                    random_access_cases.append(dict(file=filename,pocs=pocs,decoded=[[0,3],[0] if fallback else [],[0,3],[0,3],[] if eos else [0,3]],output_masks=[8,1,8,8,8] if fallback else [9,0,9,9,0 if eos else 9]))
                # Original PCM pixels with deliberately reordered AU POCs.
                for output_name, output_pocs in [
                    ("reorder",[0,2,1,4,3]), ("latency",[0,6,1,2,3,4,5])]:
                    reorder_groups = [[intra_nal(21 if i == 0 else 0,poc,id)
                        for id in [0,3]] for i,poc in enumerate(output_pocs)]
                    reorder_stream = b"".join(b"\x00\x00\x00\x01"+n
                        for n in [*parameters,*sum(reorder_groups,[])])
                    (D/("hevc-layered-output-"+output_name+".mp4")).write_bytes(mux(
                        reorder_stream,meta["depth"],*meta["dimensions"],
                        chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                        access_units=reorder_groups,multilayer=True))
                    if output_name == "reorder":
                        (D/"hevc-layered-output-reorder-ctts.mp4").write_bytes(mux(
                            reorder_stream,meta["depth"],*meta["dimensions"],
                            chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                            access_units=reorder_groups,multilayer=True,presentation_order=output_pocs))
                # Four storage slots with three reorder AUs: a retained hidden
                # reference can fill the slot otherwise available for decoding.
                pressure_vps,_,_ = vps(fmt,independent=False,base_format=fmt,
                    ordering_rows=[([4,4],3,0)]*3)
                pv = bits(unescape(pressure_vps[2:]))
                pressure_vps = pressure_vps[:2]+escape(packed(pv[:32]+source_ptl+pv[128:]))
                pressure_parameters = [pressure_vps,*[
                    output_pps(n) if ((n[0]>>1)&63)==34 else n for n in parameters[1:]]]
                for pressure_name, flags in [
                    ("fullness",[False,True,True,True,False]),
                    ("shared-reference",[True,True,True,False]),
                    ("rps-release",[False,True,True,True,False])]:
                    pressure_groups = [[intra_nal(21 if poc == 0 else 0,poc,id,
                        picture_output=flag,retain_zero=poc>0 and
                        not (pressure_name == "rps-release" and poc == len(flags)-1))
                        for id in [0,3]] for poc,flag in enumerate(flags)]
                    pressure_stream = b"".join(b"\x00\x00\x00\x01"+n
                        for n in [*pressure_parameters,*sum(pressure_groups,[])])
                    (D/("hevc-layered-output-dpb-"+pressure_name+".mp4")).write_bytes(mux(
                        pressure_stream,meta["depth"],*meta["dimensions"],
                        chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                        access_units=pressure_groups,multilayer=True))
                    if pressure_name == "fullness":
                        corrupt_groups = [g[:] for g in pressure_groups]
                        last = corrupt_groups[-1][-1]
                        corrupt_groups[-1][-1] = last[:2]+escape(unescape(last[2:])[:-256])
                        corrupt_stream = b"".join(b"\x00\x00\x00\x01"+n
                            for n in [*pressure_parameters,*sum(corrupt_groups,[])])
                        (D/"hevc-layered-output-dpb-fullness-truncated.mp4").write_bytes(mux(
                            corrupt_stream,meta["depth"],*meta["dimensions"],
                            chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                            access_units=corrupt_groups,multilayer=True))
                phase_vps,_,_ = vps(fmt,independent=True,base_format=fmt,
                    ordering_rows=[([4,4],3,0)]*3)
                ph = bits(unescape(phase_vps[2:]))
                phase_vps = phase_vps[:2]+escape(packed(ph[:32]+source_ptl+ph[128:]))
                phase_sps = parameter_ids(layer(params[33][0],3),104,[1])
                phase_pps = parameter_ids(layer(params[34][0],3),0,[1,1])
                phase_parameters = [phase_vps,params[33][0],phase_sps,
                    output_pps(params[34][0]),output_pps(phase_pps)]
                phase_groups = [[intra_nal(21 if poc == 0 or id == 3 and poc == 4 else 0,
                    poc,id,inter_layer=False,picture_output=0<poc<4,
                    retain_zero=poc>0 and not (id == 3 and poc == 4))
                    for id in [0,3]] for poc in range(5)]
                phase_groups[3].append(((36<<9)|(3<<3)|1).to_bytes(2,"big")+b"\x80")
                phase_stream = b"".join(b"\x00\x00\x00\x01"+n
                    for n in [*phase_parameters,*sum(phase_groups,[])])
                (D/"hevc-layered-output-dpb-layer-phase.mp4").write_bytes(mux(
                    phase_stream,meta["depth"],*meta["dimensions"],
                    chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                    access_units=phase_groups,multilayer=True))
                # Prior-output actions must be staged with reconstruction.
                for prior_name, prior_kind, discard, eos in [
                    ("bla-flush",16,False,False), ("bla-discard",16,True,False),
                    ("cra-after-eos",21,False,True)]:
                    prior_groups = [
                        [intra_nal(21,0,id) for id in [0,3]],
                        [intra_nal(0,1,id) for id in [0,3]],
                        [intra_nal(prior_kind,2,id,discard_prior=discard) for id in [0,3]],
                    ]
                    if eos: prior_groups[1].append(((36<<9)|1).to_bytes(2,"big")+b"\x80")
                    prior_stream = b"".join(b"\x00\x00\x00\x01"+n
                        for n in [*parameters,*sum(prior_groups,[])])
                    (D/("hevc-layered-output-"+prior_name+".mp4")).write_bytes(mux(
                        prior_stream,meta["depth"],*meta["dimensions"],
                        chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                        access_units=prior_groups,multilayer=True))
                    if prior_name == "bla-flush":
                        for suffix, corrupt in [("truncated",True),("eob",False)]:
                            special_groups = [g[:] for g in prior_groups]
                            if corrupt:
                                last = special_groups[2][-1]
                                special_groups[2][-1] = last[:2]+escape(unescape(last[2:])[:-256])
                            else:
                                special_groups[2].append(((37<<9)|1).to_bytes(2,"big")+b"\x80")
                            special_stream = b"".join(b"\x00\x00\x00\x01"+n
                                for n in [*parameters,*sum(special_groups,[])])
                            (D/("hevc-layered-output-bla-flush-"+suffix+".mp4")).write_bytes(mux(
                                special_stream,meta["depth"],*meta["dimensions"],
                                chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],
                                access_units=special_groups,multilayer=True))
                # NoClrasOutput resets every layer's FirstPicInLayerDecodedFlag.
                # BLA/base EOS must clear the independent layer's old POC history;
                # an ordinary unanchored CRA must retain its precise refusal.
                from generate_hevc_poc_reset_fixtures import reset_pps
                cvps,_,_ = vps(fmt,independent=True,poc_absent=False,base_format=fmt)
                cv = bits(unescape(cvps[2:]))
                cvps = cvps[:2]+escape(packed(cv[:32]+source_ptl+cv[128:]))
                csps = parameter_ids(layer(params[33][0],3),104,[1])
                cpps = parameter_ids(layer(params[34][0],3),0,[1,1])
                clras_parameters = [cvps,params[33][0],csps,output_pps(reset_pps(params[34][0],False)),output_pps(cpps)]
                # Hidden base pictures establish the history without requiring
                # additional output reordering across the later cycle-zero CRA.
                for cname, base_kind, eos in [("bla",16,False),("eos",21,True),("unanchored-cra",21,False)]:
                    clras_groups = [
                        [intra_nal(21,127,0,"1",picture_output=False)],
                        [intra_nal(1,255,0,"",picture_output=False),intra_nal(1,255,3,inter_layer=False,picture_output=False)],
                        [intra_nal(base_kind,1,0,ue(0),picture_output=True),intra_nal(21,1,3,inter_layer=False,picture_output=True)],
                    ]
                    if eos: clras_groups[1].append(((36<<9)|1).to_bytes(2,"big")+b"\x80")
                    clras_stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*clras_parameters,*sum(clras_groups,[])])
                    (D/("hevc-layered-clras-"+cname+"-poc.mp4")).write_bytes(mux(clras_stream,meta["depth"],*meta["dimensions"],chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],access_units=clras_groups,multilayer=True))
                def output_slice(nal, original, flag):
                    source = bytes(original["rbsp"])
                    entropy = source[original["entropy_byte_offset"]:]
                    value = bits(unescape(nal[2:]))
                    boundary = len(value)-len(entropy)*8
                    align = value[:boundary].rfind("1")
                    header = value[:align]
                    assert original["first"] and not original["dependent"]
                    r = Reader(header, 1)
                    if 16 <= ((nal[0]>>1)&63) <= 23: r.read(1)
                    r.ue(); r.read(meta["extra_bits"]); r.ue()
                    at = r.at
                    header = header[:at]+str(int(flag))+header[at+int(meta["output_flag"]):]
                    return nal[:2]+escape(packed(header+"1")+entropy)
                for name, alternative, missing, flags, expected, error in [
                    ("alternative-missing",True,True,None,[8,1],None),
                    ("disabled-missing",False,True,None,[8,0],None),
                    ("alternative-hidden",True,False,[False,False],[8,0],None),
                    ("alternative-disagree",True,False,[True,False],[8],"HEVC alternative output pic_output_flag disagrees across layers"),
                ]:
                    policy_vps, _, _ = vps(fmt, independent=False, base_format=fmt, output_highest=True, alternative_output=alternative)
                    policy_bits = bits(unescape(policy_vps[2:]))
                    policy_vps = policy_vps[:2]+escape(packed(policy_bits[:32]+source_ptl+policy_bits[128:]))
                    policy_parameters = [policy_vps,*parameters[1:]]
                    policy_groups = [list(group) for group in groups]
                    if missing: policy_groups[1] = base_predicted
                    if flags is not None:
                        policy_parameters = [policy_vps,*[output_pps(n) if ((n[0]>>1)&63)==34 else n for n in parameters[1:]]]
                        for au in range(2):
                            originals = meta["packets"][au]*2
                            policy_groups[au] = [output_slice(n,h,True if au==0 else flags[int(i>=len(meta["packets"][au]))]) for i,(n,h) in enumerate(zip(groups[au],originals))]
                    stream = b"".join(b"\x00\x00\x00\x01"+n for n in [*policy_parameters,*policy_groups[0],*policy_groups[1]])
                    filename = "hevc-layered-output-"+name+".mp4"
                    (D/filename).write_bytes(mux(stream,meta["depth"],*meta["dimensions"],chroma_format=meta["chroma_format"],chroma_depth=meta["chroma_depth"],access_units=policy_groups,multilayer=True))
                    output_cases.append(dict(file=filename,seed=seed+".mp4",output_masks=expected,error=error,missing=missing,hidden=flags==[False,False]))
    (D/"hevc-layered-random-access.json").write_text(json.dumps(dict(cases=random_access_cases),indent=2)+"\n")
    # Parameter-state witness, not coded-picture acceptance: inference must
    # follow the latest active base PPS even if the enhancement NAL is unchanged.
    import generate_hevc_layered_pps as gp
    prefix = D / "hevc-layered-pps-infer-from-sps"
    read = lambda suffix: Path(str(prefix) + suffix).read_bytes()
    current_sps = parameter_ids(read(".current.sps"), 7, [1])
    current_pps = parameter_ids(read(".pps"), 0, [1, 1])
    (D / "hevc-layered-activation-inference.mp4").write_bytes(
        gp.movie(read(".vps"),read(".base.sps"),current_sps,read(".base.pps"),current_pps))
    (D / "hevc-layered-output-policy.json").write_text(json.dumps(dict(cases=output_cases),indent=2)+"\n")
    (D / "hevc-layered-access-units.json").write_text(json.dumps(dict(cases=access_cases),indent=2)+"\n")
    (D / "hevc-layered-reconstruction.json").write_text(json.dumps(dict(cases=cases,replacements=replacements),indent=2)+"\n")


if __name__ == "__main__":
    main()
