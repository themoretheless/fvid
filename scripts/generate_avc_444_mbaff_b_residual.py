"""Owned shared 444 MBAFF retained B residuals, scaling and per-cell deblocking."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_444_mbaff_inter import configuration,pcm,packet,expected
from generate_avc_444_mbaff_p_partitions import reverse
from generate_avc_444_mbaff_b_partitions import combine
from generate_avc_422_mbaff_b_partitions import state,coded_packet,reference,motion,MODES_B
from generate_avc_422_mbaff_b_colocated import SWAP
from generate_avc_444_deblock import chroma_qp
from generate_avc_separate_mbaff_8x8_deblock import deblock_mbaff_plane
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_pcm_formats import mux
from avc_fixture_weights import B_PROFILES
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-b-residual'
PARTS4=['mb4','mb9','mb12','mb20','sub0','sub3','sub12','mix3']
PARTS8=['mb4','mb9','mb12','mb20','sub0','sub3','mix0']
SCALING=((8,24,32),(12,20,28))
@lru_cache(None)
def source_planes(yd,cd,index):
    return [[(90+20*p+7*index+2*((x//4+y//4)%2)+2*(y%2))<<(d-8)for y in range(64)for x in range(32)]for p,d in enumerate([yd,cd,cd])]
def mask(activity,p,address,block,eight):
    if activity!='checker':return activity=='all'or(activity=='cb'and p==1)or(activity=='cr'and p==2)
    x,y=(block%2,block//2)if eight else((block&1)+((block>>2)&1)*2,((block>>1)&1)+(block>>3)*2)
    return(p+address+x+y)%3==0
@lru_cache(None)
def prediction(yd,cd,topology,mode,weighted):
    negative=MODES_B.index(mode)%2!=0;cells,_=state(topology,mode,negative);pred=[]
    for li in range(2):
        vectors={k:motion(v[li])for k,v in cells.items()};refs={k:reference(v[li])or 0 for k,v in cells.items()}
        plane,_=expected(source_planes(yd,cd,li),[yd,cd,cd],topology,(0,0),'mc',False,False,False,cell_vectors=vectors,cell_references=refs);pred.append(plane)
    return cells,combine(cells,pred,[yd,cd,cd],topology,'explicit'if weighted else'plain')[0]
def residual_picture(yd,cd,topology,mode,weighted,activity,offsets,scaling,eight):
    cells,pred=prediction(yd,cd,topology,mode,weighted);qps=[26,chroma_qp(26,offsets[0],cd),chroma_qp(26,offsets[1],cd)]
    planes=[pred[p*2048:(p+1)*2048]for p in range(3)]
    raw,_=expected(planes,[yd,cd,cd],topology,(0,0),'ac8'if eight else'ac4',False,False,False,qps=qps,block_active=lambda p,a,b,e:mask(activity,p,a,b,e),scaling4=SCALING[0]if scaling else(16,)*3,scaling8=SCALING[1]if scaling else(16,)*3)
    return cells,pred,raw,qps

def strength_callback(cells,topology,activity,component,eight,force=False,ignore_motion=False):
    def callback(pa,pl,qa,ql):return cached(pa,pl[0]//4,pl[1]//4,qa,ql[0]//4,ql[1]//4)
    @lru_cache(None)
    def cached(pa,px,py,qa,qx,qy):
        def active(address,x,y):
            block=(y//2)*2+x//2 if eight else(x&1)+((y&1)<<1)+((x>>1)<<2)+((y>>1)<<3)
            return mask(activity,component,address,block,eight)
        if force or active(pa,px,py)or active(qa,qx,qy):return 2
        pf,qf=MODES[topology][pa//2],MODES[topology][qa//2]
        if pf!=qf:return 1
        if ignore_motion:return 0
        def refs(address,x,y):
            return [None if v is False else(li*3+(1+((address%2)^reference(v))if pf else 0),motion(v))for li,v in enumerate(cells[(address,x,y)])]
        a,z=refs(pa,px,py),refs(qa,qx,qy)
        def equal(l,r):return l==r if l is None or r is None else l[0]==r[0]and abs(l[1][0]-r[1][0])<4 and abs(l[1][1]-r[1][1])<(2 if pf else 4)
        return int(not(equal(a[0],z[0])and equal(a[1],z[1]))and not(equal(a[0],z[1])and equal(a[1],z[0])))
    return callback
@lru_cache(maxsize=256)
def prepared(yd,cd,topology,mode,weighted,activity,offsets,scaling,eight):
    cells,pred,raw,qps=residual_picture(yd,cd,topology,mode,weighted,activity,offsets,scaling,eight)
    @lru_cache(None)
    def filtered(filter_idc,force=False,ignore_motion=False):
        return sum([deblock_mbaff_plane(raw[p*2048:(p+1)*2048],d,topology,filter_idc,False,transform8=eight,qp=qps[p],edge_strength=strength_callback(cells,topology,activity,p,eight,force,ignore_motion))for p,d in enumerate([yd,cd,cd])],[])
    residual_changes=sum(a!=b for a,b in zip(pred,raw));scaling_changes=0
    if scaling:
        wrong=residual_picture(yd,cd,topology,mode,weighted,activity,offsets,False,eight)[2];scaling_changes=sum(a!=b for a,b in zip(raw,wrong))
    result=[]
    for idc in [0,1,2]:
        target=filtered(idc);witness={'residual':residual_changes,'filter':sum(a!=b for a,b in zip(raw,target)),'component_flags':0,'motion_strength':0,'slice_edges':0,'scaling':scaling_changes}
        if idc!=1:
            witness['component_flags']=sum(a!=b for a,b in zip(target,filtered(idc,force=True)))
            witness['motion_strength']=sum(a!=b for a,b in zip(target,filtered(idc,ignore_motion=True)))
            witness['slice_edges']=sum(a!=b for a,b in zip(filtered(0),filtered(2)))
        else:assert target==raw
        result.append((target,witness))
    return result

def samples(yd,cd,topology,mode,weighted,activity,offsets,scaling,eight,idc):
    return prepared(yd,cd,topology,mode,weighted,activity,offsets,scaling,eight)[idc]
@lru_cache(None)
def reference_packets(yd,cd,cabac,topology):
    return [pcm(source_planes(yd,cd,0),[yd,cd,cd],topology,cabac),pcm(source_planes(yd,cd,1),[yd,cd,cd],SWAP[topology],cabac,index=1,poc=4)]
def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={k:0 for k in ['residual','filter','component_flags','motion_strength','slice_edges','scaling']}
    for eight in [False,True]:
        modes=PARTS8 if eight else PARTS4
        for di,(yd,cd)in enumerate([(8,8),(10,12),(14,8)]):
            for entropy,ti,mi,activity,idc in itertools.product([-1,0,1,2],range(4),range(len(modes)),['all','cb','cr','checker'],[0,1,2]):
                topology=list(MODES)[ti];mode=modes[mi];cabac=entropy>=0;init=max(entropy,0)
                offsets=(-6,6)if(mi+entropy)%2 else(0,0);scaling=(ti+entropy)%2!=0;weighted=(di+entropy)%2!=0
                target,w=samples(yd,cd,topology,mode,weighted,activity,offsets,scaling,eight,idc)
                for k,v in w.items():witness[k]+=v
                cfg=configuration(yd,cd,cabac,False,offsets=offsets,bipred=int(weighted),scaling=SCALING if scaling else None)
                first,future=reference_packets(yd,cd,cabac,topology);negative=MODES_B.index(mode)%2!=0;aso=negative^cabac
                b=coded_packet(cabac,topology,mode,negative,False,weights=B_PROFILES['mixed']if weighted else None,full_resolution=True,init=init,residual=lambda p,a,b,e:mask(activity,p,a,b,e),filter_mode=idc,keep_reference=True,eight=eight)
                packets=[first,future,b,packet(topology,cabac,init,(0,0),'mc',False,False,index=3,poc=6)]
                if aso:packets=[reverse(p)for p in packets]
                name=f'bres-y{yd}-c{cd}-e{entropy}-{topology}-{mode}-t{8 if eight else 4}-{activity}-d{idc}'
                for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
                for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(pts,i==0,p)for i,(pts,p)in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<24576H',*(sum(source_planes(yd,cd,0),[])+sum(source_planes(yd,cd,1),[])+target+target)))]:
                    (OUT/f'{name}.{ext}').write_bytes(data)
                cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,topology=topology,mode=mode,eight=eight,activity=activity,idc=idc,offsets=offsets,scaling=scaling,weighted=weighted,aso=aso))
    assert len(cases)==8640 and all(v>0 for v in witness.values()),witness
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
