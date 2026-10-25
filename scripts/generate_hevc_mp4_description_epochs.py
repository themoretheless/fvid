#!/usr/bin/env python3
"""Owned HEVC stsd/stsc and fragmented description transitions, no codec calls."""
import json, struct
from pathlib import Path
from generate_hevc_inband_layered_dispatch import inventory
from generate_hevc_layered_reconstruction import configuration, parameter_ids
from generate_hevc_colour_plane_fixtures import child
from generate_mp4_fragment_description_regressions import atoms
from hevc_fixture_mp4 import mux
ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
OUT = ROOT / 'hevc-mp4-description-epochs'
def atom(k,b):return struct.pack('>I',len(b)+8)+k+b
def word(*v):return b''.join(struct.pack('>I',n)for n in v)
def entry(name,length=4,extra=False,pasp=False,hdr=None,vps_override=None):
    data=(ROOT/name).read_bytes()
    for k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:data=child(data,k)
    kind,payload=next(atoms(child(data,b'stsd')[8:]));cfg=bytearray(child(payload[78:],b'hvcC'));cfg[21]=(cfg[21]&~3)|(length-1)
    if extra or hdr is not None or vps_override:
        params=configuration(cfg)
        if extra:params[34].append(parameter_ids(params[34][-1],0,[2,0]))
        if vps_override:params[32]=[vps_override]
        if hdr is not None:
            from generate_hevc_colour_plane_fixtures import escape
            params[39]=[b'\x4e\x19'+escape((b'\x05\x11'+bytes(range(1,17))+b'\x7f')*500+b'\x90\x04'+struct.pack('>HH',hdr,hdr//2)+b'\x80')]
        cfg=bytes(cfg[:22])+bytes([len(params)])+b''.join(bytes([(128 if k in [32,33,34]else 0)|k])+struct.pack('>H',len(v))+b''.join(struct.pack('>H',len(n))+n for n in v)for k,v in params.items())
    return atom(kind,payload[:78]+atom(b'hvcC',cfg)+(atom(b'pasp',word(4,3))if pasp else b''))
def generate():
    files={};cases=[]
    base='hevc-pcm-reference-rext8.mp4';multi='hevc-layered-reconstruct-pcm-dependent.mp4';colour='hevc-layered-reconstruct-colour-planes-dependent.mp4'
    cfg,bg,size=inventory(ROOT/base);mcfg,mg,_=inventory(ROOT/multi);_,cg,_=inventory(ROOT/colour);_,inter,_=inventory(ROOT/'hevc-layered-reconstruct-inter-prediction.mp4')
    source_cases=json.loads((ROOT/'hevc-layered-reconstruction.json').read_text())['cases']
    colour_seed=next(c['seed']for c in source_cases if c['file']==colour)
    plans=[('base-multi-base',[base,multi],[bg[0],mg[0],bg[0]],[0,3,0],[base,base,base],[0,0,0],False,[4,4]),
           ('multi-base-multi',[multi,base],[mg[0],bg[0],mg[0]],[3,0,3],[base,base,base],[0,0,0],False,[4,4]),
           ('multi-colour-multi',[multi,colour],[mg[0],cg[0],mg[0]],[3,3,3],[base,colour_seed,base],[0,0,0],False,[4,4]),
           ('equivalent-inter',[multi,multi],[mg[0],inter[1],mg[0]],[3,3,3],[base,base,base],[0,1,0],False,[4,4]),
           ('configuration-addition-inter',[multi,multi],[mg[0],inter[1],mg[0]],[3,3,3],[base,base,base],[0,1,0],True,[4,4]),
           ('length-change-inter',[multi,multi],[mg[0],inter[1],mg[0]],[3,3,3],[base,base,base],[0,1,0],False,[4,2])]
    plans.append(('first-description-two',[base,multi],[mg[0],bg[0],mg[0]],[3,0,3],[base,base,base],[0,0,0],False,[4,2]))
    plans.append(('aud-configuration-inter',[multi,multi],[mg[0],[b'\x01\xf9\x00\x00\x03\xff',b'\x46\x01\x50',*inter[1]],mg[0]],[3,3,3],[base,base,base],[0,1,0],True,[4,2]))
    plans.append(('hvcC-hdr-inter',[multi,multi],[mg[0],inter[1],mg[0]],[3,3,3],[base,base,base],[0,1,0],False,[4,2]))
    plans.append(('illegal-vps-inter',[multi,multi],[mg[0],inter[1],mg[0]],[3,3,3],[base,base,base],[0,1,0],False,[4,4]))
    plans.append(('hvcC-zero-hdr-inter',[multi,multi],[mg[0],inter[1],mg[0]],[3,3,3],[base,base,base],[0,1,0],False,[4,2]))
    plans.append(('metadata-only-transition',[base,multi],[bg[0],[b'\x46\x01\x10'],mg[0],bg[0]],[0,None,3,0],[base]*4,[0]*4,False,[4,2]))
    plans.append(('hvcC-metadata-inter-hdr',[multi,multi],[mg[0],[b'\x46\x01\x10'],inter[1],mg[0]],[3,None,3,3],[base]*4,[0,0,1,0],False,[4,2]))
    plans.append(('hvcC-metadata-idr-hdr',[multi,multi],[mg[0],[b'\x46\x01\x10'],mg[0],mg[0]],[3,None,3,3],[base]*4,[0]*4,False,[4,2]))
    for name,sources,groups,layers,seeds,frames,extra,lengths in plans:
        entries=[entry(sources[0],lengths[0],hdr=100 if name in ['hvcC-hdr-inter','hvcC-zero-hdr-inter','hvcC-metadata-inter-hdr','hvcC-metadata-idr-hdr']else None),
                 entry(sources[1],lengths[1],extra,True,hdr=900 if name in ['hvcC-hdr-inter','hvcC-metadata-inter-hdr','hvcC-metadata-idr-hdr']else(0 if name=='hvcC-zero-hdr-inter'else None),
                       vps_override=(ROOT/'hevc-ols-reselection/illegal.vps').read_bytes()if name=='illegal-vps-inter'else None)]
        ids=[2,1,2]if name=='first-description-two'else([1,2,2,1]if len(groups)==4 else[1,2,1]);count=len(groups);sync=[i+1 for i,g in enumerate(groups)if any(16<=((n[0]>>1)&63)<=23 for n in g)];packets=[b''.join(len(n).to_bytes(lengths[d-1],'big')+n for n in g)for g,d in zip(groups,ids)]
        params=configuration(cfg);stream=b''.join(b'\0\0\0\1'+n for n in sum(params.values(),[])+[n for g in [bg[0]]*count for n in g])
        template=dict(atoms(mux(stream,8,*size,access_units=[bg[0]]*count)))
        ftyp=atom(b'ftyp',template[b'ftyp']);offset=len(ftyp)+8;offsets=[]
        for p in packets:offsets.append(offset);offset+=len(p)
        def rewrite(data,empty=False):
            out=b''
            for k,b in atoms(data):
                if k in [b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b,empty)
                elif k==b'stsd':b=word(0,2)+b''.join(entries)
                elif empty and k in [b'stts',b'stsc',b'stco',b'stss']:b=word(0,0)
                elif k==b'stsz':b=word(0,0,0)if empty else word(0,0,count,*map(len,packets))
                elif k==b'stsc':b=word(0,count,*[v for i,d in enumerate(ids)for v in (i+1,1,d)])
                elif k==b'stco':b=word(0,count,*offsets)
                elif k==b'stss':b=word(0,len(sync),*sync)
                out+=atom(k,b)
            return out
        files[name+'.mp4']=ftyp+atom(b'mdat',b''.join(packets))+atom(b'moov',rewrite(template[b'moov']))
        init=ftyp+atom(b'moov',rewrite(template[b'moov'],True)+atom(b'mvex',atom(b'trex',word(0,1,1,1,0,0x02000000))))
        def fragment(i):
            flags=0x02000000 if i+1 in sync else 0x01010000
            tfhd=atom(b'tfhd',word(0x02003a,1,ids[i],1,len(packets[i]),flags))
            def build(off):return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',tfhd+atom(b'tfdt',word(0,i))+atom(b'trun',word(1,1,off))))
            return build(len(build(0))+8)+atom(b'mdat',packets[i])
        files[name+'-fragmented.mp4']=init+b''.join(fragment(i)for i in range(count))
        if name in ['hvcC-hdr-inter','hvcC-zero-hdr-inter','hvcC-metadata-inter-hdr','hvcC-metadata-idr-hdr']:
            from generate_ffv1_gray_fixtures import element,uint
            from generate_hevc_colour_plane_fixtures import escape
            private=child(entries[0][8+78:],b'hvcC')
            track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_MPEGH/ISO/HEVC')+element(0x63a2,private)+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,size[0])+uint(0xba,size[1]))
            blocks=[];previous=ids[0]
            for i,(g,d)in enumerate(zip(groups,ids)):
                g=list(g)
                if d!=previous:
                    peak=(0 if name=='hvcC-zero-hdr-inter'else 900)if d==2 else 100
                    hdr=b'\x4e\x19'+escape(b'\x90\x04'+struct.pack('>HH',peak,peak//2)+b'\x80')
                    at=1 if g and ((g[0][0]>>1)&63)==35 else 0
                    g[at:at]=[hdr]
                previous=d
                packet=b''.join(len(n).to_bytes(4,'big')+n for n in g)
                blocks.append(element(0x1f43b675,uint(0xe7,i*40)+element(0xa3,b'\x81\0\0'+(b'\x80'if i+1 in sync else b'\0')+packet)))
            files[name+'.mkv']=element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+b''.join(blocks))
        cases.append(dict(name=name,layers=layers,oracles=[str(Path(s).with_suffix('.yuv'))for s in seeds],oracle_frames=frames,descriptions=ids,lengths=lengths,hdr=[100,900,100]if name=='hvcC-hdr-inter'else([100,0,100]if name=='hvcC-zero-hdr-inter'else([100,None,900,100]if name in ['hvcC-metadata-inter-hdr','hvcC-metadata-idr-hdr']else None)),
                          error='active VPS changed'if name=='illegal-vps-inter'else None))
        if name=='hvcC-zero-hdr-inter':
            # The opening hvcC itself receives positive and then zero CLL.
            # This witnesses initial configuration receipt, without an inline
            # SEI that could conceal an incorrectly merged opening value.
            k,e=next(atoms(entries[0]));initial_cfg=child(e[78:],b'hvcC');params=configuration(initial_cfg)
            from generate_hevc_colour_plane_fixtures import escape
            params[39].append(b'\x4e\x19'+escape(b'\x90\x04\0\0\0\0\x80'))
            new_cfg=initial_cfg[:22]+bytes([len(params)])+b''.join(bytes([(128 if k in [32,33,34]else 0)|k])+struct.pack('>H',len(v))+b''.join(struct.pack('>H',len(n))+n for n in v)for k,v in params.items())
            entries[0]=atom(k,e[:78]+atom(b'hvcC',new_cfg))
            files['initial-zero.mp4']=ftyp+atom(b'mdat',b''.join(packets))+atom(b'moov',rewrite(template[b'moov']))
    files['manifest.json']=(json.dumps(dict(cases=cases,initial_zero='initial-zero.mp4',provenance='Original repository-owned HEVC entropy, parameter banks and pixel oracles. Own regular/fragmented MP4 muxing; no private media, external codec process, FFmpeg or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
