"""Own joined-slice CAVLC 422 MBAFF AC-count contexts and matrix-basis oracle."""
import itertools,json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_pcm_formats import Writer,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-p-dense'
BASIS=[[2,2,2,2],[2,1,-1,-2],[2,-2,-2,2],[1,-2,2,-1]]
FRAME=[(1,0),(0,1),(0,2),(1,1)]
FIELD=[(0,1),(1,0),(0,2),(0,3)]
def block_counts(address,component):
    pattern=[4,0,1,4,0,1,4,1]
    return [pattern[(i+address+component)%8] for i in range(8)]
def values(count,sign,depth,field):
    positions=(FIELD if field else FRAME)[:count];scale=16*(1<<(depth-8));out=[]
    for y in range(4):
        for x in range(4):
            numerator=sum(sign*scale*(20 if u%2 and v%2 else 13 if u%2==0 and v%2==0 else 16)*BASIS[u][x]*BASIS[v][y] for u,v in positions)
            assert numerator%4==0
            out.append((numerator//4+32)//64)
    return out

def packet(source,cd,topology,negative):
    # The complete physical map defines neighbour ownership independently of
    # decoder layouts, then entropy availability only admits previously coded cells.
    owners={};locations=[]
    for pair,field in enumerate(MODES[topology]):
        for parity in range(2):
            ox=pair%2*8;oy=pair//2*32+(parity if field else parity*16);step=2 if field else 1
            address=len(locations);locations.append((ox,oy,step,field))
            for row in range(16):
                for col in range(8):
                    xy=(ox+col,oy+row*step);assert xy not in owners;owners[xy]=(address,row//4*2+col//4)
    assert len(owners)==1024
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(1,4);b.u(0);b.u(2,4);b.u(0);b.u(0);b.u(0);b.se(0);b.ue(1)
    coded={};target=source[:];tables=set();witnesses=0
    for address,(ox,oy,step,field) in enumerate(locations):
        b.ue(0)
        if address%2==0:b.u(int(field))
        b.ue(0)
        if field:b.u(1)
        b.se(0);b.se(0);b.ue(6);b.se(0);b.u(1);b.u(1) # CBP32, zero 422 DC
        for component in range(2):
            for block,count in enumerate(block_counts(address,component)):
                bx=block%2;by=block//2;x=ox+bx*4;y=oy+by*4*step
                def neighbour(xy):
                    owner=owners.get(xy)
                    return coded.get((component,*owner)) if owner is not None else None
                left=neighbour((x-1,y));top=neighbour((x,y-step))
                nc=(left+top+1)//2 if left is not None and top is not None else left if left is not None else top if top is not None else 0
                table=0 if nc<2 else 1 if nc<4 else 2;tables.add(table)
                sign=-1 if (address+component+block+int(negative))%2 else 1
                if count==0:
                    code,bits=[(1,1),(3,2),(15,4)][table];b.u(code,bits)
                elif count==1:
                    code,bits=[(1,2),(2,2),(14,4)][table];b.u(code,bits);b.u(int(sign<0));b.u(1)
                else:
                    assert count==4
                    code,bits=[(3,6),(4,4),(11,4)][table];b.u(code,bits)
                    for _ in range(3):b.u(int(sign<0))
                    b.u(1,2 if sign<0 else 1);b.u(3,5) # fourth unit level, total_zeros0
                coded[(component,address,block)]=count
                correct=values(count,sign,cd,field);wrong=values(count,sign,cd,not field)
                for row in range(4):
                    for col in range(4):
                        at=2048+component*1024+(y+row*step)*16+x+col;pred=source[at];maximum=(1<<cd)-1
                        value=max(0,min(maximum,pred+correct[row*4+col]));other=max(0,min(maximum,pred+wrong[row*4+col]));target[at]=value;witnesses+=int(value!=other)
    assert tables=={0,1,2},'fixture must enter all three nC VLC classes below eight'
    assert witnesses>0
    for component in range(2):assert source[2048+component*1024:3072+component*1024]!=target[2048+component*1024:3072+component*1024]
    nal=b.nal(0x41);return len(nal).to_bytes(4,'big')+nal,target

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),topology,negative,crop,aso in itertools.product([(8,8),(10,12),(14,8)],MODES,[False,True],[False,True],[False,True]):
        name=f'422-mbaff-dense-y{yd}-c{cd}-{topology}-n{int(negative)}-crop{int(crop)}-a{int(aso)}'
        config=configuration(2,yd,cd,crop,False,height_mbs=2,mbaff=True)
        first,source=picture(2,yd,cd,False,topology,aso,0);second,target=packet(source,cd,topology,negative)
        packets=[first,second,prediction(False,topology,aso,(0,0),False,2)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<12288H',*(source+target+target)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=False,topology=topology,negative=negative,aso=aso,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==96
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('96 own joined-slice 422 MBAFF count-context movies')
if __name__=='__main__':generate()
