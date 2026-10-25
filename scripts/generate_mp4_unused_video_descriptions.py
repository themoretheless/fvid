#!/usr/bin/env python3
"""Own MP4 table rewrites: unreferenced unknown entries cannot hide native codecs."""
import json,struct
from pathlib import Path
from generate_mp4_fragment_description_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors"
OUT=ROOT/"mp4-unused-video-descriptions"
def atom(k,p):return struct.pack(">I4s",8+len(p),k)+p
def rewrite(data,position,active=None,reference=1):
    sample=0
    def visit(data):
        nonlocal sample
        out=b""
        for k,p in atoms(data):
            if k in [b"moov",b"trak",b"mdia",b"minf",b"stbl",b"mvex",b"moof",b"traf"]:p=visit(p)
            elif k==b"stsd":
                entries=[atom(c,v)for c,v in atoms(p[8:])]
                entries.insert(position,atom(b"zzzz",bytes(6)+reference.to_bytes(2,"big")))
                p=p[:4]+struct.pack(">I",len(entries))+b"".join(entries)
            elif k in [b"stsc",b"tfhd",b"trex"]:
                p=bytearray(p)
                offsets=range(16,len(p),12)if k==b"stsc"else([8]if k==b"trex"or int.from_bytes(p[:4],"big")&2 else[])
                for at in offsets:
                    n=int.from_bytes(p[at:at+4],"big")
                    if n>position:p[at:at+4]=(n+1).to_bytes(4,"big")
                    if k!=b"trex":
                        if sample==active:p[at:at+4]=(position+1).to_bytes(4,"big")
                        sample+=1
                p=bytes(p)
            out+=atom(k,p)
        return out
    return visit(data)
def generate():
    files={};cases=[];refusals=[]
    for name in ["vp9-hevc-vp9","hevc-vp9-hevc","av1-hevc-av1","all-owned-codecs"]:
        for suffix in ["","-fragmented"]:
            source="mixed-layered-descriptions/"+name+suffix+".mp4"
            data=(ROOT/source).read_bytes()
            for position in [0,1,2]:
                file=name+suffix+"-insert"+str(position)+".mp4"
                files[file]=rewrite(data,position)
                cases.append(dict(file=file,source="../"+source,insert=position))
    for suffix in ["","-fragmented"]:
        source="hevc-mp4-description-epochs/hvcC-zero-hdr-inter"+suffix+".mp4"
        for position in [0,1,2]:
            file="pure-hevc"+suffix+"-insert"+str(position)+".mp4"
            files[file]=rewrite((ROOT/source).read_bytes(),position)
            cases.append(dict(file=file,source="../"+source,insert=position))
        data=(ROOT/("mixed-layered-descriptions/vp9-hevc-vp9"+suffix+".mp4")).read_bytes()
        for active in [0,1]:
            file="active"+str(active)+suffix+".mp4"
            files[file]=rewrite(data,0,active=active)
            refusals.append(dict(file=file,kind="active",sample=active))
        file="external-reference"+suffix+".mp4"
        files[file]=rewrite(data,0,reference=2)
        refusals.append(dict(file=file,kind="external"))
    files["manifest.json"]=(json.dumps(dict(cases=cases,refusals=refusals,provenance="Saved owned synthetic codec entropy; original sample-entry/table rewrites; no private media, subprocess or network."),indent=2)+"\n").encode()
    return files
if __name__=="__main__":
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
