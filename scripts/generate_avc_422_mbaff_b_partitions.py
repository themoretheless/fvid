"""Owned complete B partition tables, independent two-list motion and 422 assembly."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_p_fractional import expected
from generate_avc_422_mbaff_b_colocated import SWAP
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from avc_fixture_weights import B_PROFILES,table as weight_table,uni,bi
from generate_avc_422_mbaff_b_field_weights import scale,PAST,CURRENT,FUTURE
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-b-partitions'
MB_BITS=['0','100','101','110000','110001','110010','110011','110100','110101','110110','110111','111110','1110000','1110001','1110010','1110011','1110100','1110101','1110110','1110111','1111000','1111001','111111']
SUB_BITS=['0','100','101','11000','11001','11010','11011','111000','111001','111010','111011','11110','11111']
MODES_B=['mb'+str(i) for i in range(4,22)]+['sub'+str(i) for i in range(13)]+['mix'+str(i) for i in range(4)]
def uses(mode,list_index):return mode=='bi' or mode==['l0','l1'][list_index]
def parts(mode):
    if mode.startswith('mb'):
        code=int(mode[2:]);preds=[('l0','l0'),('l1','l1'),('l0','l1'),('l1','l0'),('l0','bi'),('l1','bi'),('bi','l0'),('bi','l1'),('bi','bi')][(code-4)//2]
        w,h=(16,8) if code%2==0 else (8,16)
        return code,None,[(0,0,w,h,0,preds[0]),(0 if w==16 else 8,8 if w==16 else 0,w,h,1,preds[1])]
    codes=[int(mode[3:])]*4 if mode.startswith('sub') else [[0,1,2,3],[4,7,10,12],[5,6,8,11],[9,2,1,0]][int(mode[3:])]
    result=[]
    for group,code in enumerate(codes):
        if code==0:pred,w,h='direct',4,4
        elif code<=3:pred,w,h=['l0','l1','bi'][code-1],8,8
        elif code<=9:pred=['l0','l1','bi'][(code-4)//2];w,h=(8,4) if code%2==0 else (4,8)
        else:pred,w,h=['l0','l1','bi'][code-10],4,4
        for y in range(0,8,h):
            for x in range(0,8,w):result.append((group%2*8+x,group//2*8+y,w,h,group,pred))
    return 22,codes,result

def reference(value):return value[0] if value is not None and value is not False else None
def motion(value):return value[1] if value is not None and value is not False else (0,0)
def candidates(values):
    a,b,c,d=values;return [a,b,d if c is None else c]
def predict(values,ref,kind='median'):
    a,b,c=candidates(values);preferred=b if kind=='top' else a if kind in ['bottom','left'] else c if kind=='right' else None
    if reference(preferred)==ref:return motion(preferred)
    if b is None and c is None and a is not None:b=c=a
    matching=[v for v in [a,b,c] if reference(v)==ref]
    if len(matching)==1:return motion(matching[0])
    return tuple(sorted(motion(v)[axis] for v in [a,b,c])[1] for axis in range(2))
def direct(values):
    refs=[min((reference(v) for v in candidates(values[i]) if reference(v) is not None),default=None) for i in range(2)]
    if refs==[None,None]:return [(0,(0,0)),(0,(0,0))]
    return [False if ref is None else (ref,predict(values[i],ref)) for i,ref in enumerate(refs)]

def state(topology,mode,negative,direct_by_address=None):
    _,_,partition=parts(mode);cells_out={};deltas={}
    for pair,field in enumerate(MODES[topology]):
        cells={}
        for parity in range(2):
            address=pair*2+parity;ox=pair%2*16;oy=pair//2*32+(parity if field else parity*16);step=2 if field else 1
            def neighbours(x,y,w,list_index):
                at=[(ox+x-1,oy+y*step),(ox+x,oy+(y-1)*step),(ox+x+w,oy+(y-1)*step),(ox+x-1,oy+(y-1)*step)]
                return [cells[p][list_index] if p in cells else None for p in at]
            # Spatial direct uses the original external macroblock neighbours,
            # never explicit partitions reconstructed earlier in this MB.
            frozen=[neighbours(0,0,16,i) for i in range(2)]
            for index,(x,y,w,h,group,pred) in enumerate(partition):
                values=[False,False]
                if pred=='direct':values=direct(frozen)if direct_by_address is None else direct_by_address[address]
                else:
                    for list_index in range(2):
                        if not uses(pred,list_index):continue
                        ref=int(field and (group+list_index+int(negative))%2)
                        kind=('top' if y==0 else 'bottom') if (w,h)==(16,8) else ('left' if x==0 else 'right') if (w,h)==(8,16) else 'median'
                        predictor=predict(neighbours(x,y,w,list_index),ref,kind)
                        sign=-1 if (pair+parity+index+list_index+int(negative))%2 else 1;delta=(sign*(1+index%2),-sign*(1+(index//2)%2));deltas[(address,index,list_index)]=delta
                        values[list_index]=(ref,tuple(a+b for a,b in zip(predictor,delta)))
                for row in range(y,y+h):
                    for col in range(x,x+w):cells[(ox+col,oy+row*step)]=values;cells_out[(address,col//4,row//4)]=values
    assert len(cells_out)==128
    return cells_out,deltas

def coded_packet(cabac,topology,mode,negative,aso,weights=None,poc=2,bottom_delta=None,full_resolution=False,init=0,spatial=True,residual=None,filter_mode=1,keep_reference=False,eight=False):
    assert residual is None or full_resolution
    code,subcodes,partition=parts(mode)
    allow8=subcodes is None or all(c<=3 for c in subcodes)
    assert not eight or allow8
    if residual is not None:
        from avc_cabac_444_writer import Cabac444Writer
        from generate_avc_444_mbaff_inter import single
        from generate_avc_444_deblock import coefficient
    _,deltas=state(topology,mode,negative);nals=[]
    for pair,field in enumerate(MODES[topology]):
        b=Writer();b.ue(pair);b.ue(1);b.ue(0);b.u(2,4);b.u(0);b.u(poc,4)
        if bottom_delta is not None:b.se(bottom_delta)
        b.u(int(spatial));b.u(0);b.u(0);b.u(0)
        if weights is not None:weight_table(b,weights)
        if keep_reference:b.u(0)
        if cabac:b.ue(init)
        b.se(0);b.ue(filter_mode)
        if filter_mode!=1:b.se(0);b.se(0)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=(Cabac444Writer if residual is not None else CabacWriter)(init,26)
        refs=[{},{}];mags=[{},{}];cbfs=[{}, {}, {}]
        for parity in range(2):
            address=pair*2+parity;ox=pair%2*16;oy=pair//2*32+(parity if field else parity*16);step=2 if field else 1;bottom=parity==1 and not field
            if not cabac:
                b.ue(0)
                if parity==0:b.u(int(field))
                b.ue(code)
                if subcodes is not None:
                    for v in subcodes:b.ue(v)
            else:
                c.decision(24+int(bottom),0)
                if parity==0:c.decision(70,int(field))
                prior=0
                for i,bit in enumerate(MB_BITS[code]):
                    ctx=27+int(bottom) if i==0 else 30 if i==1 else (31 if prior&1 else 32) if i==2 else 32
                    c.decision(ctx,int(bit));prior=prior*2+int(bit)
                if subcodes is not None:
                    for subcode in subcodes:
                        prior=0
                        for i,bit in enumerate(SUB_BITS[subcode]):
                            ctx=36 if i==0 else 37 if i==1 else (38 if prior&1 else 39) if i==2 else 39
                            c.decision(ctx,int(bit));prior=prior*2+int(bit)
            for list_index in range(2):
                for group in sorted({p[4] for p in partition}):
                    selected=[p for p in partition if p[4]==group and uses(p[5],list_index)]
                    if not selected:continue
                    x,y,_,_,_,_=selected[0];ref=int(field and (group+list_index+int(negative))%2)
                    if field:
                        if not cabac:b.u(int(ref==0))
                        else:
                            a=refs[list_index].get((ox+x-1,oy+y*step),0);top=refs[list_index].get((ox+x,oy+(y-1)*step),0)
                            c.decision(54+int(a>0)+2*int(top>0),ref)
                            if ref:c.decision(58,0)
                    for x,y,w,h,_,_ in selected:
                        for row in range(y,y+h):
                            for col in range(x,x+w):refs[list_index][(ox+col,oy+row*step)]=ref
            for list_index in range(2):
                for index,(x,y,w,h,_,pred) in enumerate(partition):
                    if not uses(pred,list_index):continue
                    delta=deltas[(address,index,list_index)]
                    if not cabac:
                        for v in delta:b.se(v)
                    else:
                        a=mags[list_index].get((ox+x-1,oy+y*step),(0,0));top=mags[list_index].get((ox+x,oy+(y-1)*step),(0,0))
                        for axis,v in enumerate(delta):c.mvd(axis,v,a[axis]+top[axis])
                    for row in range(y,y+h):
                        for col in range(x,x+w):mags[list_index][(ox+col,oy+row*step)]=tuple(abs(v) for v in delta)
            if not cabac:
                b.ue(9 if residual is not None else 0)
                if residual is not None:
                    if allow8:b.u(int(eight))
                    b.se(0)
            else:
                for ctx in ([73]*4 if residual is not None else[75,76,75,76]if bottom else[73,74,75,76]):c.decision(ctx,int(residual is not None))
                if not full_resolution:c.decision(77,0)
                if residual is not None:
                    if allow8:c.decision(399+int(bottom and eight),int(eight))
                    c.decision(60,0)
            if residual is not None:
                for p in range(3):
                    for block in range(4 if eight else 16):
                        if eight:x,y=block%2*8,block//2*8;side=8
                        else:x,y=((block&1)+((block>>2)&1)*2)*4,(((block>>1)&1)+(block>>3)*2)*4;side=4
                        xx,yy=ox+x,oy+y*step;active=residual(p,address,block,eight);sign=-1 if(p+pair+parity+block)%2 else 1
                        if cabac:
                            inc=int(cbfs[p].get((xx-1,yy),False))+2*int(cbfs[p].get((xx,yy-step),False))
                            single(c,p,eight,field,inc,sign,zero=not active)
                        elif eight:
                            for sub in range(4):
                                if active and sub==3:coefficient(b,4*sign,0)
                                else:b.u(1)
                        elif active:coefficient(b,sign,1)
                        else:b.u(1)
                        for row in range(side):
                            for col in range(side):cbfs[p][(xx+col,yy+row*step)]=active
        if cabac:b.bits.extend(c.finish())
        nals.append(b.nal(0x41 if keep_reference else 0x01,trailing=not cabac))
    if aso:nals.reverse()
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)

@lru_cache(None)
def prediction_samples(yd,cd,topology,mode,negative):
    _,source=picture(2,yd,cd,False,topology,False,0);_,later=picture(2,yd,cd,False,SWAP[topology],False,1,poc=4);cells,_=state(topology,mode,negative)
    predicted=[]
    for list_index,reference_plane in enumerate([source,later]):
        vectors={key:motion(value[list_index]) for key,value in cells.items()};refs={key:reference(value[list_index]) or 0 for key,value in cells.items()}
        predicted.append(expected(reference_plane,yd,cd,topology,(0,0),False,cell_motions=vectors,cell_references=refs))
    return source,later,cells,predicted

@lru_cache(None)
def samples(yd,cd,topology,mode,negative,weight_mode=None):
    source,later,cells,predicted=prediction_samples(yd,cd,topology,mode,negative)
    if weight_mode=='explicit':
        weighted=[uni(predicted[0],[yd,cd,cd],B_PROFILES['mixed'],0)[0],uni(predicted[1],[yd,cd,cd],B_PROFILES['mixed'],1)[0]]
        combined=bi(predicted[0],predicted[1],[yd,cd,cd],B_PROFILES['mixed'])[0]
    target=[]
    for component in range(3):
        width,bw,offset=(32,16,0) if component==0 else (16,8,2048+(component-1)*1024)
        plane=[None]*(width*64)
        for pair,field in enumerate(MODES[topology]):
            for parity in range(2):
                address=pair*2+parity;ox=pair%2*bw;oy=pair//2*32+(parity if field else parity*16)
                for row in range(16):
                    for col in range(bw):
                        key=(address,col*(1 if component==0 else 2)//4,row//4);active=[v is not False for v in cells[key]];assert any(active)
                        at=(oy+row*(2 if field else 1))*width+ox+col;a,z=predicted[0][offset+at],predicted[1][offset+at]
                        if weight_mode=='explicit':value=combined[offset+at] if all(active) else weighted[int(active[1])][offset+at]
                        elif weight_mode is not None and all(active):
                            if weight_mode=='implicit':w1=21
                            elif field:
                                r0=0 if weight_mode=='implicit-matching' else reference(cells[key][0]);r1=0 if weight_mode=='implicit-matching' else reference(cells[key][1])
                                w1=scale(CURRENT[parity],PAST[parity^r0],FUTURE[parity^r1])>>2 if weight_mode!='implicit-picture' else 42
                            else:w1=42
                            assert -64<=w1<=128
                            value=max(0,min((1<<(yd if component==0 else cd))-1,((64-w1)*a+w1*z+32)//64))
                        else:value=(a+z+1)//2 if all(active) else a if active[0] else z
                        plane[at]=value
        assert None not in plane;target+=plane
    return source,later,target

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,topology,mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,MODES_B):
        mode_id=MODES_B.index(mode);negative=mode_id%2!=0;aso=negative^cabac;crop=mode_id%3!=0
        name=f'422-mbaff-bparts-y{yd}-c{cd}-e{int(cabac)}-{topology}-{mode}'
        source,later,target=samples(yd,cd,topology,mode,negative);config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True)
        first,a=picture(2,yd,cd,cabac,topology,aso,0);future,z=picture(2,yd,cd,cabac,SWAP[topology],aso,1,poc=4);assert a==source and z==later
        packets=[first,future,coded_packet(cabac,topology,mode,negative,aso),prediction(cabac,topology,aso,(0,0),False,2,poc=6)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<16384H',*(source+target+later+later)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,mode=mode,negative=negative,aso=aso,decode_order=[0,2,1,3],crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==840
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('840 own MBAFF B partition movies')
if __name__=='__main__':generate()
