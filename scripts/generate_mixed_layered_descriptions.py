#!/usr/bin/env python3
"""Owned VP9/layered-HEVC description switches with independent pixel controls."""
import json,struct
from pathlib import Path
from generate_vp9_chroma import coloured,mp4
from generate_hevc_inband_layered_dispatch import inventory
from generate_hevc_mp4_description_epochs import atom,word
from generate_mp4_fragment_description_regressions import atoms
from generate_hevc_colour_plane_fixtures import child
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'mixed-layered-descriptions'
def sample_entry(data):
    for k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:data=child(data,k)
    k,p=next(atoms(child(data,b'stsd')[8:]));return atom(k,p)
def generate():
    files={};cases=[]
    vp,oracle=coloured(8,'420',16);vcontrol=mp4([vp],8,'420',(16,16),[1]);ve=sample_entry(vcontrol)
    assert not any(oracle[1::2]);files['vp9.yuv']=bytes(oracle[::2])
    source='hevc-layered-reconstruct-pcm-dependent.mp4';_,g,_=inventory(ROOT/source);he=sample_entry((ROOT/source).read_bytes());hp=b''.join(len(n).to_bytes(4,'big')+n for n in g[0])
    _,rg,_=inventory(ROOT/'hevc-layered-output-reorder-ctts.mp4');rp=[b''.join(len(n).to_bytes(4,'big')+n for n in x)for x in rg[:3]]
    plans=[('vp9-hevc-vp9',[ve,he],[vp,hp,vp],[1,2,1],[0,1,2],[None,3,None]),
           ('hevc-vp9-hevc',[he,ve],[hp,vp,hp],[1,2,1],[0,1,2],[3,None,3]),
           ('reordered-vp9-hevc',[ve,he],[vp,*rp,vp],[1,2,2,2,1],[1,0,3,2,4],[None,3,3,3,None])]
    ac='avc-pcm-formats/avc-pcm-format3-y8-c8-0'
    ae=sample_entry((ROOT/(ac+'.mp4')).read_bytes());ap=(ROOT/(ac+'-0.packet')).read_bytes()
    files['avc.yuv']=bytes((x*31+y*7+p*43)&255 if x>=4 or y!=0 else [0,255,1,254][x] for p in range(3)for y in range(16)for x in range(32))
    an='av1-separate-tiles-w128-h128-groups2';av1e=sample_entry((ROOT/(an+'.mp4')).read_bytes());av1p=(ROOT/(an+'.obu')).read_bytes()
    for label,e,p in [('avc',ae,ap),('av1',av1e,av1p)]:
        plans += [(label+'-hevc-'+label,[e,he],[p,hp,p],[1,2,1],[0,1,2],[None,3,None]),
                  ('hevc-'+label+'-hevc',[he,e],[hp,p,hp],[1,2,1],[0,1,2],[3,None,3])]
    plans.append(('all-owned-codecs',[ae,he,ve,av1e],[ap,hp,vp,hp,av1p],[1,2,3,2,4],list(range(5)),[None,3,None,3,None]))
    bad=bytearray(hp);first=int.from_bytes(bad[:4],'big');second=4+first;length=int.from_bytes(bad[second:second+4],'big');assert second+4+length==len(bad);bad[second:second+4]=(length-128).to_bytes(4,'big');del bad[-128:]
    plans.append(('corrupt-hevc-between-vp9',[ve,he],[vp,bytes(bad),vp],[1,2,1],[0,1,2],[None,3,None]))
    inline='hevc-inband-layered-dispatch/pcm-dependent-empty-prefix.mp4';_,ig,_=inventory(ROOT/inline);ip=[b''.join(len(n).to_bytes(4,'big')+n for n in g)for g in ig];assert len(ip)==2
    plans.append(('metadata-hevc-window',[ve,sample_entry((ROOT/inline).read_bytes())],[vp,*ip,vp],[1,2,2,1],list(range(4)),[None,None,3,None]))
    base='hevc-pcm-reference-rext8.mp4';_,base_groups,_=inventory(ROOT/base);bp=b''.join(len(n).to_bytes(4,'big')+n for n in base_groups[0])
    plans.append(('vp9-hevc-topology-vp9',[ve,sample_entry((ROOT/base).read_bytes()),he],[vp,bp,hp,bp,vp],[1,2,3,2,1],list(range(5)),[None,0,3,0,None]))
    hdr_data=(ROOT/'hevc-mp4-description-epochs/hvcC-zero-hdr-inter.mp4').read_bytes();st=hdr_data
    for k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:st=child(st,k)
    hs=[atom(k,v)for k,v in atoms(child(st,b'stsd')[8:])];sizes=struct.unpack('>3I',child(st,b'stsz')[12:]);offsets=struct.unpack('>3I',child(st,b'stco')[8:]);hpkt=[hdr_data[o:o+n]for o,n in zip(offsets,sizes)]
    plans.append(('vp9-hevc-hdr-vp9',[ve,*hs],[vp,*hpkt,vp],[1,2,3,2,1],list(range(5)),[None,3,3,3,None]))
    plans.append(('opening-hdr',[hs[0],ve],[hpkt[0],vp,hpkt[2]],[1,2,1],[0,1,2],[3,None,3]))
    for name,entries,packets,ids,pts,layers in plans:
        count=len(packets);sync=[1,2,5]if name=='reordered-vp9-hevc'else([1,3,4]if name=='metadata-hevc-window'else([1,2,4,5]if name=='vp9-hevc-hdr-vp9'else list(range(1,count+1))));template=dict(atoms(mp4([vp]*count,8,'420',(16,16),sync)))
        ftyp=atom(b'ftyp',template[b'ftyp']);at=len(ftyp)+8;offsets=[]
        for p in packets:offsets.append(at);at+=len(p)
        def rewrite(data,empty=False):
            out=b''
            for k,b in atoms(data):
                if k in [b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b,empty)
                elif k==b'stsd':b=word(0,len(entries))+b''.join(entries)
                elif k==b'ctts':continue
                elif empty and k in [b'stts',b'stsc',b'stco',b'stss']:b=word(0,0)
                elif k==b'stsz':b=word(0,0,0)if empty else word(0,0,count,*map(len,packets))
                elif k==b'stsc':b=word(0,count,*[v for i,d in enumerate(ids)for v in [i+1,1,d]])
                elif k==b'stco':b=word(0,count,*offsets)
                out+=atom(k,b)
            if not empty and b'stbl' not in data[:8] and any(k==b'stsd'for k,_ in atoms(data)):
                out+=atom(b'ctts',word(0x01000000,count)+b''.join(word(1)+struct.pack('>i',p-i)for i,p in enumerate(pts)))
            return out
        files[name+'.mp4']=ftyp+atom(b'mdat',b''.join(packets))+atom(b'moov',rewrite(template[b'moov']))
        init=ftyp+atom(b'moov',rewrite(template[b'moov'],True)+atom(b'mvex',atom(b'trex',word(0,1,1,1,0,0x02000000))))
        def fragment(i):
            tfhd=atom(b'tfhd',word(0x02003a,1,ids[i],1,len(packets[i]),0x02000000 if i+1 in sync else 0x01010000))
            def moof(off):return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',tfhd+atom(b'tfdt',word(0,i))+atom(b'trun',word(0x01000801,1,off)+struct.pack('>i',pts[i]-i))))
            return moof(len(moof(0))+8)+atom(b'mdat',packets[i])
        files[name+'-fragmented.mp4']=init+b''.join(fragment(i)for i in range(count))
        if name=='opening-hdr':continue
        oracles=[]
        for d,l in zip(ids,layers):
            codec=entries[d-1][4:8]
            oracles.append('vp9.yuv'if codec==b'vp09'else 'avc.yuv'if codec==b'avc1'else '../'+an+'.yuv'if codec==b'av01'else '../hevc-pcm-reference-rext8.yuv')
        cases.append(dict(name=name,descriptions=ids,pts=pts,layers=layers,oracles=oracles,oracle_frames=[1 if name=='vp9-hevc-hdr-vp9'and i==2 else 0 for i in range(count)],visible=[i for i in range(count)if not(name=='metadata-hevc-window'and i==1)],hdr=[0,100,0,100,0]if name=='vp9-hevc-hdr-vp9'else None,error='truncated or oversized bit field'if name.startswith('corrupt-')else None))
    files['manifest.json']=(json.dumps(dict(cases=cases,opening_hdr='opening-hdr',provenance='Original synthetic VP9 integer entropy and pixels, saved owned HEVC PCM entropy/oracle; own MP4 muxing, no private data, codecs, FFmpeg or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
