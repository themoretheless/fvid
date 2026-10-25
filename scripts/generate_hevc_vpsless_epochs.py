"""Original VPS-less -> multilayer HEVC epochs, using owned synthetic entropy."""
import json
from pathlib import Path
from generate_hevc_topology_transitions import generate as topology
from generate_hevc_independent_rewrite import inventory,mkv
from generate_hevc_layered_reconstruction import configuration
from hevc_fixture_mp4 import mux
from generate_vp9_chroma import coloured,mp4 as vp9_movie
from generate_mixed_layered_descriptions import sample_entry
from generate_hevc_mp4_description_epochs import atom,word
from generate_mp4_fragment_description_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'hevc-vpsless-epochs'
def mixed_movie(hevc,groups):
    vp,oracle=coloured(8,'420',16)
    entries=[sample_entry(vp9_movie([vp],8,'420',(16,16),[1])),sample_entry(hevc)]
    packets=[vp,*[b''.join(len(n).to_bytes(4,'big')+n for n in g)for g in groups],vp]
    count=len(packets);ids=[1]+[2]*len(groups)+[1]
    sync=sorted(set([1,count]+[i+2 for i,g in enumerate(groups)if any(16<=n[0]>>1&63<=23 for n in g)]))
    template=dict(atoms(vp9_movie([vp]*count,8,'420',(16,16),sync)))
    ftyp=atom(b'ftyp',template[b'ftyp']);at=len(ftyp)+8;offsets=[]
    for p in packets:offsets.append(at);at+=len(p)
    def rewrite(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b)
            elif k==b'stsd':b=word(0,2)+b''.join(entries)
            elif k==b'stsz':b=word(0,0,count,*map(len,packets))
            elif k==b'stsc':b=word(0,count,*[v for i,d in enumerate(ids)for v in [i+1,1,d]])
            elif k==b'stco':b=word(0,count,*offsets)
            elif k in [b'mdhd',b'mvhd']:b=b[:12]+word(25)+b[16:]
            out+=atom(k,b)
        return out
    return ftyp+atom(b'mdat',b''.join(packets))+atom(b'moov',rewrite(template[b'moov'])),bytes(oracle[::2])
def generate():
    seeds=topology();files={};cases=[];mixed=[]
    manifest=json.loads(seeds['manifest.json'])['cases']
    def layer(n):return(int.from_bytes(n[:2],'big')>>3)&63
    def kind(n):return n[0]>>1&63
    for source in ['base-to-multi','multislice-roundtrip','colour-planes-roundtrip']:
        c=next(c for c in manifest if c['name']==source)
        cfg,groups,size=inventory(seeds[source+'.mp4']);params=configuration(cfg)
        initial=[n for k in [32,33,34]for n in params[k]if k==32 or layer(n)==0]
        first_vps=next(i for i,g in enumerate(groups)if any(kind(n)==32 for n in g))
        groups[first_vps]=[*[n for k in [33,34]for n in params[k]if layer(n)!=0],*groups[first_vps]]
        levels=[]
        for i,g in enumerate(groups):
            present={layer(n)for n in g if kind(n)<32}
            guidance=[]
            for l in sorted(present):
                value=100 if i<first_vps else 300 if l else 200
                h=(39<<9)|(l<<3)|1
                guidance.append(h.to_bytes(2,'big')+bytes([144,4])+value.to_bytes(2,'big')+(value//2).to_bytes(2,'big')+b'\x80')
            groups[i]=guidance+g
        levels=[100 if i<first_vps else 300 if l else 200 for i,l in zip(c['samples'],c['layers'])]
        # The helper validates its authored parameter prefix; only SPS/PPS are
        # recorded in the actual output hvcC. No VPS is in the initial samples.
        for inline in [False,True]:
            name=source+('-inline'if inline else '')
            authored=([[n for n in initial if kind(n)!=32]]if inline else [])+groups
            movie=mux(b''.join(b'\0\0\0\1'+n for n in initial+sum(authored,[])),8+(cfg[17]&7),*size,chroma_format=cfg[16]&3,chroma_depth=8+(cfg[18]&7),access_units=authored,multilayer=True,inband_parameters=True,configuration_types=[33,34],empty_configuration=inline)
            private,_,_=inventory(movie)
            files[name+'.mp4']=movie;files[name+'.mkv']=mkv(private,authored,size,inline)
            cases.append(dict(name=name,samples=[i+int(inline)for i in c['samples']],layers=c['layers'],oracle='../'+c['oracle'],first_vps=first_vps+int(inline),hdr=levels,inline=inline))
        if source=='base-to-multi':
            normal=files[source+'.mp4']
            for label,authored,indices,lights in [('mixed-handoff',groups,c['samples'],levels),('mixed-prefix-only',groups[:first_vps],[0,1],[100,100])]:
                movie,vp= mixed_movie(normal,authored)
                files[label+'.mp4']=movie;files['vp9.yuv']=vp
                mixed.append(dict(name=label,samples=[0,*[i+1 for i in indices],len(authored)+1],hdr=[0,*lights,0],oracles=['vp9.yuv',*['../'+c['oracle']]*len(indices),'vp9.yuv']))
            _,illegal,illegal_size=inventory(seeds['illegal-base-to-multi.mp4'])
            illegal[first_vps]=[*[n for k in [33,34]for n in params[k]if layer(n)!=0],*illegal[first_vps]]
            invalid=mux(b''.join(b'\0\0\0\1'+n for n in initial+sum(illegal,[])),8+(cfg[17]&7),*illegal_size,chroma_format=cfg[16]&3,chroma_depth=8+(cfg[18]&7),access_units=illegal,multilayer=True,inband_parameters=True,configuration_types=[33,34])
            files['illegal-handoff.mp4']=invalid
            invalid_cfg,_,_=inventory(invalid)
            files['illegal-handoff.mkv']=mkv(invalid_cfg,illegal,illegal_size,False)
            damaged=[list(g)for g in groups]
            at=next(i for i,g in enumerate(damaged)if any(kind(n)<32 and layer(n)==3 for n in g))
            for j,n in enumerate(damaged[at]):
                if kind(n)<32 and layer(n)==3:
                    assert len(n)>130
                    damaged[at][j]=n[:-128]
                    break
            bad=mux(b''.join(b'\0\0\0\1'+n for n in initial+sum(damaged,[])),8+(cfg[17]&7),*size,chroma_format=cfg[16]&3,chroma_depth=8+(cfg[18]&7),access_units=damaged,multilayer=True,inband_parameters=True,configuration_types=[33,34])
            files['corrupt-selected.mp4']=bad
            badcfg,_,_=inventory(bad)
            files['corrupt-selected.mkv']=mkv(badcfg,damaged,size,False)

    files['manifest.json']=(json.dumps(dict(cases=cases,mixed=mixed,provenance='Owned synthetic PCM, dependent multislice and separate-colour-plane entropy. Initial hvcC intentionally omits VPS; genuine later VPS is transmitted in-band. No private media, subprocess or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
