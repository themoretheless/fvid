#!/usr/bin/env python3
"""Owned F.10.3 additional-tree streams; saved synthetic entropy, no subprocess/network."""
import json,struct
from pathlib import Path
from generate_hevc_layered_sps import vps,ue,ptl,representation
from generate_hevc_inband_layered_dispatch import inventory
from generate_hevc_layered_reconstruction import configuration,bits,parameter_ids,layer
from generate_hevc_colour_plane_fixtures import unescape,escape,packed
from hevc_fixture_mp4 import mux
from generate_ffv1_gray_fixtures import element,uint
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'hevc-additional-layer-sets'
def packet(nals):return b''.join(len(n).to_bytes(4,'big')+n for n in nals)
def three_root_vps(fmt,dependent=False):
    # Additional set=(3,7). Independent variant outputs only root 3;
    # dependent variant makes 7 depend on 3 and outputs 7 (both necessary).
    b='0000'+'11'+'000010'+'0101'+'1'*16+ptl(True)
    b+='0'+ue(15)+ue(2)+ue(0)
    b+=f'{7:06b}'+ue(1)+'10010001'+'0'+'1'
    b+='1'*((-len(b))%8)
    b+=ptl(False,level=90)+'0'+'0'*16+'1'+f'{3:06b}{7:06b}'+'0000'+('001'if dependent else'000')
    b+=ue(1)+('10'if dependent else'11')+'000' # additional set=(3,7)
    b+=ue(1)+ue(0)+'00'+'111'+('01'+'11'+'0'if dependent else'10'+'1')
    b+=ue(0)+representation(fmt)+'01'+('1'if dependent else'11')
    for counts in [[4,6,6],[6,6]if dependent else[6]]:
        b+='1'
        for tid in range(3):
            if tid:b+='1'
            b+=''.join(ue(v+tid-1)for v in counts)+ue(1+tid)+ue(0 if tid==0 else tid+1)
    b+=ue(0)+'1'+'10'+ue(0)+'0'+'0'+'1'
    return b'\x40\x01'+escape(packed(b))
def external_vps(fmt,profile,dependent=False):
    ids=[0,3,7]if dependent else[0,3]
    b='0000'+'01'+f'{len(ids)-1:06b}'+'0101'+'1'*16+ptl(True)
    b+='0'+ue(0)*3+f'{ids[-1]:06b}'+ue(1)+''.join('1'if i in ids else'0'for i in range(ids[-1]+1))+'0'+'1'
    b+='1'*((-len(b))%8)
    b+='0'+'0'*16+'1'+''.join(f'{i:06b}'for i in ids[1:])+'0000'+('001'if dependent else'0')
    b+=ue(1)+('10'if dependent else'1')+'000'
    b+=ue(1)+'1'+profile+'0'*16+ue(0)+'00'+'1'*len(ids)+('01'+'11'+'0'if dependent else'11')
    b+=ue(0)+representation(fmt)+'01'+'1'
    for _ in range(2):
        b+='1'
        for tid in range(3):
            if tid:b+='1'
            b+=''.join(ue(6+tid-1)for _ in ids[1:])+ue(1+tid)+ue(0 if tid==0 else tid+1)
    b+=ue(0)+'1'+'10'+ue(0)+'0'+'0'+'1'
    return b'\x40\x01'+escape(packed(b))
def generate():
    files={};cases=[]
    for label,seed in [('pcm','hevc-pcm-reference-rext8.yuv'),('multislice','hevc-pcm-dependent-rext8.yuv'),('colour-planes','hevc-separate-colour-planes-distinct-synthetic.yuv'),('subset','hevc-pcm-reference-rext8.yuv'),('dependent','hevc-pcm-reference-rext8.yuv'),('external','hevc-pcm-reference-rext8.yuv'),('external-dependent','hevc-pcm-reference-rext8.yuv')]:
        cfg,groups,size=inventory(ROOT/f'hevc-layered-reconstruct-{"pcm"if label in ["subset","dependent","external","external-dependent"]else label}-independent.mp4');params=configuration(cfg)
        fmt=dict(size=size,depth=[8+(cfg[17]&7),8+(cfg[18]&7)],chroma=cfg[16]&3,separate=label=='colour-planes',crop=[0]*4)
        nal,_,_=vps(fmt,independent=True,poc_absent=True,base_format=fmt,additional_sets=True)
        if label in ['subset','dependent']:nal=three_root_vps(fmt,label=='dependent')
        b=bits(unescape(nal[2:]));old=bits(unescape(params[32][0][2:]));nal=nal[:2]+escape(packed(b[:32]+old[32:128]+b[128:]))
        if label.startswith('external'):nal=external_vps(fmt,old[32:128],label=='external-dependent')
        if label in ['subset','dependent','external-dependent']:
            sps=next(n for n in params[33]if (int.from_bytes(n[:2],'big')>>3)&63==3)
            pps=next(n for n in params[34]if (int.from_bytes(n[:2],'big')>>3)&63==3)
            start=104
            if label in ['dependent','external-dependent']:
                ml_config,_,_=inventory(ROOT/'hevc-layered-reconstruct-multilayer-sps.mp4')
                sps=next(n for n in configuration(ml_config)[33]if (int.from_bytes(n[:2],'big')>>3)&63==3)
                assert bits(unescape(sps[2:]))[4:7]=='111';start=7
            params[33].append(parameter_ids(layer(sps,7),start,[2]))
            params[34].append(parameter_ids(layer(pps,7),0,[2,2]))
            extra=groups[0]
            if label in ['dependent','external-dependent']:
                _,extra_groups,_=inventory(ROOT/'hevc-layered-reconstruct-pcm-dependent.mp4')
                extra=extra_groups[0]
            for n in list(extra):
                if (int.from_bytes(n[:2],'big')>>3)&63==3:
                    raw=bits(unescape(n[2:]));assert raw[2:5]=='010'
                    groups[0].append(layer(n,7)[:2]+escape(packed(raw[:2]+'011'+raw[5:])))
        for available in [True,False]:
            raw=bytearray(nal);raw[2]=(raw[2]|4)if available else(raw[2]&~4);vv=bytes(raw)
            prefix=[vv,*[n for k in [33,34]for n in params[k]if not label.startswith('external')or ((int.from_bytes(n[:2],'big')>>3)&63)!=0]]
            coded=[n for n in groups[0]if available and not label.startswith('external')or ((int.from_bytes(n[:2],'big')>>3)&63)!=0]
            expected={3}|({7}if label in ['subset','dependent','external-dependent']else set())|({0}if available and not label.startswith('external')else set())
            assert {((int.from_bytes(n[:2],'big')>>3)&63)for n in coded if(n[0]>>1)&63<32}==expected
            for inline in [False,True]:
                name=f'{label}-available{int(available)}-inline{int(inline)}'
                packets=[prefix,coded,coded]if inline else[coded,coded]
                stream=b''.join(b'\0\0\0\1'+n for n in prefix+sum(packets,[]))
                video=mux(stream,fmt['depth'][0],*size,chroma_format=fmt['chroma'],chroma_depth=fmt['depth'][1],access_units=packets,multilayer=True,inband_parameters=True,empty_configuration=inline)
                files[name+'.mp4']=video
                if label=='pcm' and available and not inline:
                    bad=[[n[:-128] if ((int.from_bytes(n[:2],'big')>>3)&63)==3 else n for n in coded]]
                    authored=[coded,*bad]
                    files['corrupt-second-root.mp4']=mux(b''.join(b'\0\0\0\1'+n for n in prefix+sum(authored,[])),8,*size,chroma_format=fmt['chroma'],access_units=authored,multilayer=True,inband_parameters=True)
                # Preserve populated/empty hvcC bootstraps in both containers.
                from generate_hevc_colour_plane_fixtures import child
                st=video
                for k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:st=child(st,k)
                entry=child(st,b'stsd')[16:];private=child(entry[78:],b'hvcC')
                track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,size[0])+uint(0xba,size[1]))
                clusters=b''.join(element(0x1f43b675,uint(0xe7,i*40000000)+element(0xa3,b'\x81\0\0'+bytes([8 if inline and i==0 else 0x80])+packet(g)))for i,g in enumerate(packets))
                files[name+'.mkv']=element(0x1a45dfa3,uint(0x4286,1)+element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,track))+clusters)
                cases.append(dict(name=name,oracle='../'+seed,first=int(inline),frames=2,available=available,subset=label=='subset',dependent=label in ['dependent','external-dependent'],external=label.startswith('external')))
            # Independently authored F.10.3 byte oracle, including base SEI,
            # parameter/EOS retention and global EOB retention.
            sei0=b'\x4e\x01\x90\x04\0\x64\0\x32\x80'
            sei3=bytes([0x4e,25])+sei0[2:]
            source=[*prefix,sei0,sei3,*coded,b'\x48\x01\x80',b'\x4a\x01\x80']
            wanted=[]
            for n in source:
                kind=(n[0]>>1)&63;nal_layer=(int.from_bytes(n[:2],'big')>>3)&63
                keep=kind==37 or nal_layer in ([3,7]if label in ['subset','dependent','external-dependent']else[3]) or nal_layer==0 and kind in [32,33,34,36]
                if keep:
                    v=bytearray(n)
                    if kind==32:v[2]&=~4
                    wanted.append(bytes(v))
            key=f'{label}-available{int(available)}'
            files[key+'.packet']=packet(source);files[key+'.exported']=packet(wanted)
            if label=='pcm' and available:
                damaged=[n[:-128]if ((n[0]>>1)&63)<32 and ((int.from_bytes(n[:2],'big')>>3)&63)==3 else n for n in source]
                files['corrupt-root.packet']=packet(damaged)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Original additional-tree VPS syntax and F.10.3 byte oracle; saved owned PCM/multislice/separate-plane entropy; no private data, external codec, subprocess or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
