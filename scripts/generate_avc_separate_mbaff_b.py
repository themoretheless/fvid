"""Own MBAFF separate-plane B/direct, mixed co-located layouts and explicit twins."""
import itertools,json,struct
from pathlib import Path
from generate_avc_separate_mbaff_intra import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_b import packet
from generate_avc_separate_mbaff_p import expected
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-separate-mbaff-b'
SWAP={'frame':'field','field':'frame','columns':'checker','checker':'columns'}
VECTORS=[(8,4),(-3,3),(3,-3)]

def generate():
    OUT.mkdir(exist_ok=True);cases=[];layouts=list(MODES);temporal_witnesses=0;conversion_witnesses=0
    for depth,cabac,base,convert,mode in itertools.product([8,10,14],[False,True],range(4),[False,True],['temporal','temporal-skip','spatial','spatial-skip','bi-derived']):
        name=f'separate-mbaff-b-{depth}-e{int(cabac)}-t{base}-c{int(convert)}-{mode}'
        config=configuration(depth,cabac,False);packets=[bytearray()for _ in range(4)];frames=[[]for _ in range(4)]
        for c,motion in enumerate(VECTORS):
            topology=layouts[(base+c)%4];reference_topology=SWAP[topology] if convert else topology
            source=[(x*1703+y*7907+x*y*113+c*349)&((1<<depth)-1) for y in range(64)for x in range(32)]
            first,_=picture(0,depth,8,cabac,topology,False,0,planes_override=[source,[],[]],colour_plane=c)
            later=expected(source,depth,reference_topology,motion,False,'none')
            vectors=[];wrong=[];temporal=not mode.startswith('spatial')
            for current_field,reference_field in zip(MODES[topology],MODES[reference_topology]):
                vertical=motion[1]*2 if reference_field and not current_field else int(motion[1]/2) if current_field and not reference_field else motion[1]
                colocated=(motion[0],vertical);forward=tuple((v+1)//2 for v in colocated);back=tuple(a-b for a,b in zip(forward,colocated))
                vectors.append([forward,back] if temporal else [(0,0),(0,0)])
                raw_forward=tuple((v+1)//2 for v in motion);wrong.append([raw_forward,tuple(a-b for a,b in zip(raw_forward,motion))])
            a=expected(source,depth,topology,(0,0),False,'none',pair_motions=[v[0]for v in vectors])
            z=expected(later,depth,topology,(0,0),False,'none',pair_motions=[v[1]for v in vectors])
            target=[(u+v+1)//2 for u,v in zip(a,z)]
            if temporal:
                zero_a=expected(source,depth,topology,(0,0),False,'none');zero_z=expected(later,depth,topology,(0,0),False,'none')
                zero=[(u+v+1)//2 for u,v in zip(zero_a,zero_z)]
                assert target!=zero;temporal_witnesses+=sum(u!=v for u,v in zip(target,zero))
                if convert:
                    wa=expected(source,depth,topology,(0,0),False,'none',pair_motions=[v[0]for v in wrong])
                    wz=expected(later,depth,topology,(0,0),False,'none',pair_motions=[v[1]for v in wrong])
                    mutation=[(u+v+1)//2 for u,v in zip(wa,wz)]
                    assert target!=mutation;conversion_witnesses+=sum(u!=v for u,v in zip(target,mutation))
            data=[first,prediction(cabac,reference_topology,False,motion,False,1,poc=4,colour_plane=c,monochrome=True),packet(cabac,topology,False,mode,pair_vectors=vectors if mode=='bi-derived' else None,colour_plane=c,monochrome=True),prediction(cabac,topology,False,(0,0),False,2,poc=6,colour_plane=c,monochrome=True)]
            for i,p in enumerate(data):packets[i].extend(p)
            for i,p in enumerate([source,later,target,later]):frames[i]+=p
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        records=[(pts,i==0,bytes(p))for i,(pts,p)in enumerate(zip([0,2,1,3],packets))]
        for suffix,data in [('avcc',config),('mp4',mux(config,records,32,64,25)),('y16',struct.pack('<24576H',*sum(frames,[])))]: (OUT/f'{name}.{suffix}').write_bytes(data)
        cases.append(dict(name=name,depth=depth,cabac=cabac,base_layout=base,convert=convert,mode=mode))
    assert len(cases)==240
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
    (OUT/'coverage.json').write_text(json.dumps(dict(cases=len(cases),temporal_motion_witnesses=temporal_witnesses,vertical_conversion_witnesses=conversion_witnesses),indent=2)+'\n')
if __name__=='__main__':generate()
