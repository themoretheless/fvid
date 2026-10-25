#!/usr/bin/env python3
"""Original F.10.2 INBLD movies and byte controls; no codec process or private data."""
import json,struct
from pathlib import Path
from generate_hevc_additional_layer_sets import generate as additional,packet
from generate_hevc_colour_plane_fixtures import child,unescape,escape,packed,packet_units
from generate_hevc_layered_reconstruction import configuration,bits
from hevc_fixture_mp4 import mux
from generate_ffv1_gray_fixtures import element,uint
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'hevc-independent-rewrite'
def inventory(data):
    st=data
    for k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:st=child(st,k)
    entry=child(st,b'stsd')[16:];cfg=child(entry[78:],b'hvcC')
    sizes=child(st,b'stsz');count=int.from_bytes(sizes[8:12],'big');assert sizes[4:8]==bytes(4)
    at=int.from_bytes(child(st,b'stco')[8:12],'big');groups=[]
    for i in range(count):
        n=int.from_bytes(sizes[12+4*i:16+4*i],'big');groups.append([p[4:]for p in packet_units(data[at:at+n])]);at+=n
    return cfg,groups,list(struct.unpack('>HH',entry[24:28]))
def rewrite(nals,temporal=0):
    out=[]
    for n in nals:
        h=int.from_bytes(n[:2],'big');kind=(h>>9)&63;layer=(h>>3)&63
        if (h&7)-1>temporal or kind==32:continue
        keep=layer in [0,3]if kind in [33,34]else(kind==37 or layer==3)
        if keep:out.append((h&~0x1f8).to_bytes(2,'big')+n[2:])
    return out
def mkv(private,groups,size,inline):
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,size[0])+uint(0xba,size[1]))
    clusters=b''.join(element(0x1f43b675,uint(0xe7,i*40000000)+element(0xa3,b'\x81\0\0'+bytes([8 if inline and i==0 else 0x80])+packet(g)))for i,g in enumerate(groups))
    return element(0x1a45dfa3,uint(0x4286,1)+element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,track))+clusters)
def generate():
    seeds=additional();files={};cases=[]
    for family in ['pcm','multislice','colour-planes','subset','external']:
        for available in [0,1]:
            for inline in [0,1]:
                key=f'{family}-available{available}-inline{inline}'
                cfg,groups,size=inventory(seeds[key+'.mp4']);params=configuration(cfg)
                prefix=[n for k in [32,33,34]for n in params.get(k,[])]if not inline else groups[0]
                sps=next(n for n in prefix if n[0]>>1&63==33 and(int.from_bytes(n[:2],'big')>>3)&63==3)
                pattern=bits(unescape(sps[2:]))[8:96]
                def mark(n):
                    kind=n[0]>>1&63
                    if kind==32:
                        b=bits(unescape(n[2:]));at=b.find(pattern);assert at>=0 and b.find(pattern,at+1)<0
                        b=b[:at+87]+'1'+b[at+88:];return n[:2]+escape(packed(b))
                    if kind==33 and(int.from_bytes(n[:2],'big')>>3)&63==3:
                        b=bytearray(unescape(n[2:]));b[11]|=1;return n[:2]+escape(b)
                    return n
                original_prefix=prefix;prefix=list(map(mark,prefix));groups=[list(map(mark,g))for g in groups]
                # Distinct HDR guidance verifies that base metadata is discarded.
                sei0=b'\x4e\x01\x90\x04\0\x28\0\x14\x80';sei3=b'\x4e\x19\x90\x04\0\x64\0\x32\x80'
                first=int(inline);groups[first]=[sei0,sei3,*groups[first]]
                depth=8+(cfg[17]&7);chroma=cfg[16]&3;cd=8+(cfg[18]&7)
                source=mux(b''.join(b'\0\0\0\1'+n for n in prefix+sum(groups,[])),depth,*size,chroma_format=chroma,chroma_depth=cd,access_units=groups,multilayer=True,inband_parameters=True,empty_configuration=bool(inline))
                output_groups=[rewrite(g)for g in groups];output_prefix=rewrite(prefix)
                output=mux(b''.join(b'\0\0\0\1'+n for n in output_prefix+sum(output_groups,[])),depth,*size,chroma_format=chroma,chroma_depth=cd,access_units=output_groups,inband_parameters=True,empty_configuration=bool(inline),independent_layer=True)
                outcfg,_,_=inventory(output)
                files[key+'-source.mp4']=source;files[key+'.mp4']=output;files[key+'.mkv']=mkv(outcfg,output_groups,size,bool(inline))
                oracle='../'+('hevc-pcm-dependent-rext8.yuv'if family=='multislice'else'hevc-separate-colour-planes-distinct-synthetic.yuv'if family=='colour-planes'else'hevc-pcm-reference-rext8.yuv')
                cases.append(dict(name=key,oracle=oracle,first=first,frames=2))
                if family == 'pcm' and available == 1 and inline == 1:
                    # Guidance in a separate sample before a new CLVS must
                    # survive both normal decode and header-only seek priming.
                    for light in [0,200]:
                        named=key+f'-pending-{light}'
                        clli=b'\x4e\x19'+escape(b'\x90\x04'+struct.pack('>HH',light,light//2)+b'\x80')
                        pending=[*groups[:first+1],[clli],*groups[first+1:]]
                        rewritten=[rewrite(g)for g in pending]
                        source_pending=mux(b''.join(b'\0\0\0\1'+n for n in prefix+sum(pending,[])),depth,*size,chroma_format=chroma,chroma_depth=cd,access_units=pending,multilayer=True,inband_parameters=True,empty_configuration=True)
                        target=mux(b''.join(b'\0\0\0\1'+n for n in output_prefix+sum(rewritten,[])),depth,*size,chroma_format=chroma,chroma_depth=cd,access_units=rewritten,inband_parameters=True,empty_configuration=True,independent_layer=True)
                        target_cfg,_,_=inventory(target)
                        files[named+'-source.mp4']=source_pending
                        files[named+'.mp4']=target
                        files[named+'.mkv']=mkv(target_cfg,rewritten,size,True)
                        cases.append(dict(name=named,oracle=oracle,first=first,last=first+2,cll=[100,light],frames=2))
                if not inline:
                    control=[*prefix,*groups[0],b'\x48\x01\x80',b'\x48\x19\x80',b'\x4a\x01\x80']
                    raw=f'{family}-available{available}'
                    files[raw+'.packet']=packet(control);files[raw+'.rewritten']=packet(rewrite(control))
                    if family=='pcm'and available:
                        original_vps=next(n for n in original_prefix if n[0]>>1&63==32)
                        files['no-inbld.packet']=packet([original_vps,*control[1:]])
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Original F.10.2 header oracle and INBLD-marked VPS/SPS from owned synthetic PCM, multislice and separate-plane sources. No private media, external codecs, subprocess or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
