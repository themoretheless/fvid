"""Own joined MBAFF spatial-direct seeds, neighbour conversion and raw colZero."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_p_fractional import expected
from generate_avc_422_mbaff_b_colocated import SWAP,future_samples
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-b-spatial'
@lru_cache(None)
def geometry(topology):
    owners={};locations=[]
    for pair,field in enumerate(MODES[topology]):
        for parity in range(2):
            x=pair%2*16;y=pair//2*32+(parity if field else parity*16);step=2 if field else 1
            address=len(locations);locations.append((x,y,step,field))
            for row in range(16):
                for col in range(16):owners[(x+col,y+row*step)]=address
    assert len(owners)==2048
    return owners,locations

def neighbours(topology,address):
    owners,locations=geometry(topology);x,y,step,_=locations[address]
    return [owner if owner is not None and owner<address else None for owner in [owners.get((x-1,y)),owners.get((x,y-step)),owners.get((x+16,y-step)),owners.get((x-1,y-step))]]
@lru_cache(None)
def vectors(topology,reference_topology,seed,future_motion,wrong_zero=False,ignore_zero=False,force_zero=False):
    _,locations=geometry(topology);result=[]
    for address,(_,_,_,field) in enumerate(locations):
        if address<2:result.append([seed,tuple(-v for v in seed)]);continue
        owners=neighbours(topology,address);derived=[]
        for list_index in range(2):
            candidates=[]
            for owner in owners:
                if owner is None:candidates.append(None);continue
                vx,vy=result[owner][list_index];source_field=locations[owner][3]
                if field and not source_field:vy=int(vy/2)
                elif source_field and not field:vy*=2
                candidates.append((vx,vy))
            a,b,c,d=candidates;c=d if c is None else c
            if b is None and c is None and a is not None:b=c=a
            available=[v for v in [a,b,c] if v is not None]
            if len(available)==1:vector=available[0]
            else:vector=tuple(sorted((a or (0,0),b or (0,0),c or (0,0))[i][axis] for i in range(3))[1] for axis in range(2))
            raw=future_motion
            if wrong_zero:
                source_field=MODES[reference_topology][address//2]
                raw=(raw[0],int(raw[1]/2) if field and not source_field else raw[1]*2 if source_field and not field else raw[1])
            if force_zero or (not ignore_zero and all(-1<=v<=1 for v in raw)):vector=(0,0)
            derived.append(vector)
        result.append(derived)
    return result

def b_packet(cabac,topology,seed,bottom_skip):
    b=Writer();b.ue(0);b.ue(1);b.ue(0);b.u(2,4);b.u(0);b.u(2,4);b.u(1);b.u(0);b.u(0);b.u(0)
    if cabac:b.ue(0)
    b.se(0);b.ue(1)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26)
    owners,locations=geometry(topology);states=[];pending=0
    for address,(x,y,step,field) in enumerate(locations):
        pair=address//2;parity=address%2;explicit=address<2;skipped=bottom_skip and parity==1 and not explicit
        a,top,_,_=neighbours(topology,address)
        if cabac:
            # mb_skip_flag precedes mb_field_decoding_flag. At the top of a
            # pair, use the mode inferred from already decoded pair neighbours.
            provisional=field if parity else MODES[topology][pair-1] if pair%2 else MODES[topology][pair-2] if pair>=2 else False
            py=pair//2*32+(parity if provisional else parity*16);pstep=2 if provisional else 1
            skip_a=owners.get((x-1,py));skip_b=owners.get((x,py-pstep))
            inc=sum(owner is not None and owner<address and states[owner]>=2 for owner in [skip_a,skip_b])
            c.decision(24+inc,int(skipped))
        if skipped:
            if not cabac:pending+=1
            states.append(1)
        else:
            if not cabac:b.ue(pending);pending=0
            if parity==0:
                if cabac:
                    modes=MODES[topology];ctx=70+int(pair%2!=0 and modes[pair-1])+int(pair>=2 and modes[pair-2]);c.decision(ctx,int(field))
                else:b.u(int(field))
            if not cabac:
                b.ue(3 if explicit else 0)
                if explicit:
                    if field:b.u(1);b.u(1)
                    for list_index in range(2):
                        for v in seed:b.se(0 if parity==1 and not field else v*(1 if list_index==0 else -1))
                b.ue(0)
            else:
                ctx=27+int(a is not None and states[a]==3)+int(top is not None and states[top]==3)
                c.decision(ctx,int(explicit))
                if explicit:
                    c.decision(30,1);c.decision(31,0)
                    for _ in range(3):c.decision(32,0)
                    if field:c.decision(54,0);c.decision(54,0)
                    for list_index in range(2):
                        for axis,v in enumerate(seed):c.mvd(axis,0 if parity==1 and not field else v*(1 if list_index==0 else -1),abs(v) if parity==1 and not field else 0)
                for block in range(4):
                    bx=block%2;by=block//2
                    def available(px,py):
                        owner=owners.get((px,py));return owner is not None and owner<address
                    left=bx>0 or available(x-1,y+by*8*step);above=by>0 or available(x+bx*8,y-step)
                    c.decision(73+int(left)+2*int(above),0)
                c.decision(77,0)
            states.append(3 if explicit else 2)
        if cabac and parity==1 and address!=7:c.range-=2;c.renormalize()
    if not cabac and pending:b.ue(pending)
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x01,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

@lru_cache(None)
def samples(yd,cd,topology,seed,future_motion,wrong_zero=False,ignore_zero=False,force_zero=False):
    reference_topology=SWAP[topology];source,later=future_samples(yd,cd,reference_topology,future_motion)
    motions=vectors(topology,reference_topology,seed,future_motion,wrong_zero,ignore_zero,force_zero)
    a=expected(source,yd,cd,topology,(0,0),False,mb_motions=tuple(v[0] for v in motions))
    z=expected(later,yd,cd,topology,(0,0),False,mb_motions=tuple(v[1] for v in motions))
    target=[(left+right+1)//2 for left,right in zip(a,z)]
    return source,later,target,motions

def generate():
    OUT.mkdir(exist_ok=True);cases=[];raw_witnesses=0
    for (yd,cd),cabac,topology,seed,future_motion,bottom_skip in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[(8,4),(-8,-4)],[(0,0),(1,2),(8,4)],[False,True]):
        reference_topology=SWAP[topology];crop=seed[0]<0;aso=cabac^bottom_skip
        name=f'422-mbaff-spatial-y{yd}-c{cd}-e{int(cabac)}-{topology}-seed{seed[0]}_{seed[1]}-col{future_motion[0]}_{future_motion[1]}-skip{int(bottom_skip)}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True);source,later,target,motions=samples(yd,cd,topology,seed,future_motion)
        if future_motion==(0,0):
            assert all(v==[(0,0),(0,0)] for v in motions[2:]),'colZero must override nonzero neighbours'
            assert target!=samples(yd,cd,topology,seed,future_motion,ignore_zero=True)[2],'ignoring colZero must change clipped pixels'
        else:
            assert all(any(v[0]) and any(v[1]) for v in motions[2:]),'spatial direct must inherit nonzero neighbours'
            assert target!=samples(yd,cd,topology,seed,future_motion,force_zero=True)[2],'zeroing spatial vectors must change clipped pixels' 
        if future_motion==(1,2) and any(a and not b for a,b in zip(MODES[topology],MODES[reference_topology])):
            assert target!=samples(yd,cd,topology,seed,future_motion,True)[2];raw_witnesses+=1
        first,original=picture(2,yd,cd,cabac,topology,aso,0);assert original==source
        packets=[first,prediction(cabac,reference_topology,aso,future_motion,False,1,poc=4),b_packet(cabac,topology,seed,bottom_skip),prediction(cabac,topology,aso,(0,0),False,2,poc=6)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<16384H',*(source+target+later+later)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,reference_topology=reference_topology,seed=seed,future_motion=future_motion,bottom_skip=bottom_skip,aso=aso,decode_order=[0,2,1,3],mb_vectors=motions,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==288 and raw_witnesses==72
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('288 own seeded spatial-direct MBAFF movies; raw-colZero witnesses:',raw_witnesses)
if __name__=='__main__':generate()
